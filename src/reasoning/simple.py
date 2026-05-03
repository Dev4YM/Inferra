from __future__ import annotations

from collections import Counter
from typing import Any

from analysis.anomaly import AnomalyScorer
from core.enums import CauseType, Severity
from events.models import NormalizedEvent
from reasoning.calibration import ConfidenceCalibrator, ConfidenceInput
from reasoning.validation import HypothesisValidator
from runtime.service_graph import ServiceGraph


class SimpleHypothesisEngine:
    """Small deterministic v0 hypothesis engine.

    This is the first buildable stepping stone. The full signal composition
    system can replace this module without changing the API shape.
    """

    def __init__(self, service_graph: ServiceGraph | None = None) -> None:
        self.anomaly_scorer = AnomalyScorer()
        self.service_graph = service_graph or ServiceGraph()
        self.validator = HypothesisValidator()
        self.calibrator = ConfidenceCalibrator()

    def generate(self, incident_id: str, events: list[NormalizedEvent]) -> list[dict[str, Any]]:
        if not events:
            return []
        tags = Counter(tag for event in events for tag in event.tags)
        services = sorted({event.service_id for event in events})
        max_severity = max(event.severity for event in events)
        max_anomaly = max(self.anomaly_scorer.event_score(event) for event in events)
        event_ids = [event.event_id for event in events]
        root_cause_event_id = self._root_cause_event_id(events)
        dependency_proximity = self._dependency_proximity(services)
        hypotheses: list[dict[str, Any]] = []

        if tags["connection_refused"] or tags["timeout"]:
            root_service = self._root_service(root_cause_event_id, events)
            description = f"Connection failures or timeouts affecting {', '.join(services)}"
            if root_service:
                description += f"; likely upstream/root service: {root_service}"
            hypotheses.append(
                self._hypothesis(
                    incident_id,
                    rank=1,
                    cause_type=CauseType.DEPENDENCY_FAILURE,
                    description=description,
                    score=(0.72 if max_severity >= Severity.ERROR else 0.55) + dependency_proximity * 0.12,
                    event_ids=event_ids,
                    services=services,
                    checks=["Check upstream service health", "Review recent connection timeout/refused logs"],
                    dependency_proximity=dependency_proximity,
                    root_cause_event_id=root_cause_event_id,
                )
            )

        if tags["oom"] or tags["disk_full"] or tags["resource_pressure"] or max_anomaly >= 0.75:
            hypotheses.append(
                self._hypothesis(
                    incident_id,
                    rank=len(hypotheses) + 1,
                    cause_type=CauseType.RESOURCE_EXHAUSTION,
                    description=f"Resource exhaustion signals detected on {', '.join(services)}",
                    score=max(0.78, max_anomaly),
                    event_ids=event_ids,
                    services=services,
                    checks=[
                        "Check host CPU, memory, and disk utilization",
                        "Inspect high-usage processes and service resource limits",
                    ],
                    dependency_proximity=dependency_proximity,
                    root_cause_event_id=root_cause_event_id,
                )
            )

        if tags["restart"] or tags["crash"]:
            hypotheses.append(
                self._hypothesis(
                    incident_id,
                    rank=len(hypotheses) + 1,
                    cause_type=CauseType.APPLICATION_BUG,
                    description=f"Crash or restart pattern detected on {', '.join(services)}",
                    score=0.68,
                    event_ids=event_ids,
                    services=services,
                    checks=["Inspect service logs before restart", "Check process exit codes"],
                    dependency_proximity=dependency_proximity,
                    root_cause_event_id=root_cause_event_id,
                )
            )

        if tags["config_change"] or tags["deployment"]:
            hypotheses.append(
                self._hypothesis(
                    incident_id,
                    rank=len(hypotheses) + 1,
                    cause_type=CauseType.CONFIGURATION_ERROR,
                    description=f"Errors occurred near a deployment or configuration change on {', '.join(services)}",
                    score=0.58,
                    event_ids=event_ids,
                    services=services,
                    checks=["Review recent deployments", "Diff recent configuration changes"],
                    dependency_proximity=dependency_proximity,
                    root_cause_event_id=root_cause_event_id,
                )
            )

        if not hypotheses:
            hypotheses.append(
                self._hypothesis(
                    incident_id,
                    rank=1,
                    cause_type=CauseType.UNKNOWN,
                    description=f"Elevated WARN+ events detected on {', '.join(services)}",
                    score=max(0.4, min(0.62, max_anomaly)),
                    event_ids=event_ids,
                    services=services,
                    checks=["Inspect grouped event timeline", "Add service topology for stronger correlation"],
                    dependency_proximity=dependency_proximity,
                    root_cause_event_id=root_cause_event_id,
                )
            )

        hypotheses = [self.validator.validate(item, events) for item in hypotheses]
        hypotheses.sort(key=lambda item: item["total_score"], reverse=True)
        for index, item in enumerate(hypotheses, start=1):
            item["rank"] = index
            item["confidence_label"] = self._confidence_label(item)
        return hypotheses

    def _hypothesis(
        self,
        incident_id: str,
        rank: int,
        cause_type: CauseType,
        description: str,
        score: float,
        event_ids: list[str],
        services: list[str],
        checks: list[str],
        dependency_proximity: float = 0.5,
        root_cause_event_id: str | None = None,
    ) -> dict[str, Any]:
        return {
            "hypothesis_id": f"{incident_id}-h{rank}",
            "incident_id": incident_id,
            "rank": rank,
            "cause_type": cause_type.value,
            "description": description,
            "root_cause_event_id": root_cause_event_id,
            "total_score": round(min(1.0, score), 4),
            "score_breakdown": {
                "temporal_alignment": 0.5,
                "correlation_strength": 0.4,
                "frequency_weight": min(1.0, len(event_ids) / 10.0),
                "dependency_proximity": round(dependency_proximity, 4),
                "evidence_coverage": 1.0,
                "anomaly_severity": round(score, 4),
            },
            "supporting_events": event_ids,
            "contradicting_events": [],
            "affected_services": services,
            "suggested_checks": checks,
            "confidence_label": "low",
            "is_valid": True,
            "invalidation_reasons": [],
        }

    def _confidence_label(self, hypothesis: dict[str, Any]) -> str:
        breakdown = hypothesis.get("score_breakdown", {})
        return self.calibrator.label(
            ConfidenceInput(
                score=float(hypothesis.get("total_score") or 0.0),
                supporting_count=len(hypothesis.get("supporting_events") or []),
                contradiction_count=int(breakdown.get("contradiction_count") or 0),
                dependency_proximity=float(breakdown.get("dependency_proximity") or 0.5),
            )
        )

    def _dependency_proximity(self, services: list[str]) -> float:
        if len(services) < 2:
            return 0.5
        distances: list[int] = []
        for index, source in enumerate(services):
            for target in services[index + 1 :]:
                distance = self.service_graph.shortest_path_length(source, target, max_depth=4)
                if distance is not None:
                    distances.append(distance)
        if not distances:
            return 0.25
        nearest = min(distances)
        return max(0.35, 1.0 - (nearest - 1) * 0.2)

    def _root_cause_event_id(self, events: list[NormalizedEvent]) -> str | None:
        if not events:
            return None
        services = {event.service_id for event in events}
        for event in sorted(events, key=lambda item: item.timestamp):
            dependents = self.service_graph.get_dependents(event.service_id)
            if dependents & services:
                return event.event_id
        return sorted(events, key=lambda item: (item.timestamp, -self.anomaly_scorer.event_score(item)))[0].event_id

    def _root_service(self, event_id: str | None, events: list[NormalizedEvent]) -> str | None:
        if event_id is None:
            return None
        for event in events:
            if event.event_id == event_id:
                return event.service_id
        return None
