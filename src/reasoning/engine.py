from __future__ import annotations

from dataclasses import fields
from typing import TYPE_CHECKING, Any

from analysis.models import EventCluster
from config.models import InferraConfig
from core.enums import CauseType
from core.models import CalibrationModel, Incident, InferenceGraph, ScoreBreakdown, ScoredHypothesis
from events.models import NormalizedEvent
from reasoning.composer import (
    RawHypothesis,
    compose_from_paths,
    compose_from_signals,
    dedup_raw_hypotheses,
    merge_rules,
    standalone_signal_hypothesis,
)
from reasoning.inference_graph import build_inference_graph
from reasoning.scoring import compute_score_breakdown, merge_config_weights, rank_hypotheses, weighted_total
from reasoning.signals.registry import collect_signals
from reasoning.signals.types import SignalContext
from reasoning.validation import HypothesisValidator
from runtime.service_graph import ServiceGraph
from storage.calibration_store import assign_confidence_label

if TYPE_CHECKING:
    from storage import CalibrationStore, WeightStore


class HypothesisEngine:
    """Deterministic graph and signal composition engine."""

    def __init__(
        self,
        service_graph: ServiceGraph | None = None,
        config: InferraConfig | None = None,
        *,
        weight_store: WeightStore | None = None,
        calibration_store: CalibrationStore | None = None,
    ) -> None:
        from analysis.anomaly import AnomalyScorer

        self.service_graph = service_graph or ServiceGraph()
        self._config = config or InferraConfig()
        self._weight_store = weight_store
        self._calibration_store = calibration_store
        self.validator = HypothesisValidator(self._config)
        self.anomaly_scorer = AnomalyScorer(self._config.anomaly_detection)
        self.last_inference_graph: InferenceGraph | None = None

    def generate(
        self,
        incident_id: str,
        events: list[NormalizedEvent],
        *,
        cluster: EventCluster | None = None,
        incident: Incident | None = None,
        incident_event_ids: list[str] | None = None,
    ) -> list[dict[str, Any]]:
        self.last_inference_graph = None
        if not events:
            return []
        ordered = sorted(events, key=lambda e: (e.timestamp, e.event_id))
        graph = build_inference_graph(ordered, self.service_graph, self._config.inference_graph)
        self.last_inference_graph = graph
        ctx = SignalContext.build(ordered, self.service_graph, inferra_config=self._config)
        signals = collect_signals(ctx)
        by_id = {e.event_id: e for e in ordered}
        rules = merge_rules(self._config.hypothesis_engine)
        raw_list: list[RawHypothesis] = []
        raw_list.extend(compose_from_paths(graph, by_id, incident_id))
        raw_list.extend(compose_from_signals(signals, rules, by_id, incident_id))
        raw_list = dedup_raw_hypotheses(raw_list, float(self._config.hypothesis_engine.dedup_overlap_threshold))
        min_conf = float(self._config.hypothesis_engine.min_generation_confidence)
        min_ev = int(self._config.hypothesis_engine.min_supporting_events)
        raw_list = [h for h in raw_list if h.generation_confidence >= min_conf and len(h.supporting_events) >= min_ev]
        if not raw_list:
            raw_list.extend(_fallback_unknown(incident_id, ordered, self.anomaly_scorer))
        for s in signals:
            if any(s.name in h.generation_rule for h in raw_list):
                continue
            if s.confidence >= 0.6 and s.name not in {"error_spike"}:
                raw_list.append(standalone_signal_hypothesis(incident_id, s, by_id))
        raw_list = dedup_raw_hypotheses(raw_list, float(self._config.hypothesis_engine.dedup_overlap_threshold))
        raw_list.sort(key=lambda h: (-h.generation_confidence, h.hypothesis_id))
        max_h = int(self._config.hypothesis_engine.max_hypotheses_per_incident)
        raw_list = raw_list[:max_h]
        services = sorted({e.service_id for e in ordered})
        root_default = self._root_cause_event_id(ordered)
        weight_dict = self._active_weights()
        if cluster is not None and cluster.anomaly_scores:
            anomaly_by_service = dict(cluster.anomaly_scores)
        else:
            anomaly_by_service = self.anomaly_scorer.service_scores(ordered)
        anomaly_event_scores = {e.event_id: self.anomaly_scorer.event_score(e) for e in ordered}
        hypotheses: list[dict[str, Any]] = []
        for rank, raw in enumerate(raw_list, start=1):
            hyp = self._raw_to_dict(
                incident_id,
                rank,
                raw,
                ordered,
                services,
                root_default,
            )
            breakdown = compute_score_breakdown(
                hyp,
                by_id,
                cluster=cluster,
                incident=incident,
                incident_event_ids=incident_event_ids,
                service_graph=self.service_graph,
                anomaly_by_service=anomaly_by_service,
                anomaly_event_scores=anomaly_event_scores,
            )
            hyp["score_breakdown"] = breakdown
            hyp["total_score"] = weighted_total(breakdown, weight_dict)
            hypotheses.append(self.validator.validate(hyp, ordered))
        tiebreak = list(self._config.scoring.tuning.tiebreak_order)
        hypotheses = rank_hypotheses(hypotheses, tiebreak, by_id)
        cal = self._calibration_store.load() if self._calibration_store else CalibrationModel()
        for index, item in enumerate(hypotheses, start=1):
            item["rank"] = index
            item["confidence_label"] = assign_confidence_label(
                float(item.get("total_score") or 0.0),
                cal,
                defaults=self._config.calibration.defaults,
                min_samples=int(self._config.calibration.min_samples_per_bucket),
                staleness_days=int(self._config.calibration.staleness_threshold_days),
            )
        return hypotheses

    def _active_weights(self) -> dict[str, float]:
        if self._weight_store is None:
            return merge_config_weights({}, self._config)
        return merge_config_weights(self._weight_store.load().weights, self._config)

    def _raw_to_dict(
        self,
        incident_id: str,
        rank: int,
        raw: RawHypothesis,
        events: list[NormalizedEvent],
        services: list[str],
        root_default: str | None,
    ) -> dict[str, Any]:
        by_id = {e.event_id: e for e in events}
        supporting = [eid for eid in raw.supporting_events if eid in by_id]
        all_service_ids = sorted({e.service_id for e in events})
        if raw.cause_type == CauseType.DEPENDENCY_FAILURE:
            supporting = [e.event_id for e in sorted(events, key=lambda e: (e.timestamp, e.event_id))]
        root_id = raw.root_cause_event_id if raw.root_cause_event_id in by_id else root_default
        if raw.cause_type == CauseType.DEPENDENCY_FAILURE:
            sup_events = [by_id[e] for e in supporting if e in by_id]
            root_id = self._root_cause_event_id(sup_events) if sup_events else self._root_cause_event_id(events)
            root_id = root_id or root_default
        if raw.cause_type == CauseType.DEPENDENCY_FAILURE:
            affected = sorted(set(raw.affected_services) | set(all_service_ids))
        else:
            affected = sorted(raw.affected_services) if raw.affected_services else services
        description = raw.description
        if raw.cause_type == CauseType.DEPENDENCY_FAILURE and root_id and root_id in by_id:
            rsvc = by_id[root_id].service_id
            if "likely upstream/root service" not in description:
                joined = ", ".join(sorted(set(all_service_ids)))
                description = f"Connection failures or timeouts affecting {joined}; likely upstream/root service: {rsvc}"
        return {
            "hypothesis_id": raw.hypothesis_id,
            "incident_id": incident_id,
            "rank": rank,
            "cause_type": raw.cause_type.value,
            "description": description,
            "root_cause_event_id": root_id,
            "supporting_events": supporting if supporting else [e.event_id for e in events],
            "contradicting_events": [],
            "affected_services": affected,
            "suggested_checks": list(raw.suggested_checks),
            "confidence_label": "low",
            "is_valid": True,
            "invalidation_reasons": [],
        }

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
            dependents = self.service_graph.get_dependents(_svc(event.service_id))
            if dependents & {_svc(s) for s in services}:
                return event.event_id
        return sorted(events, key=lambda item: (item.timestamp, -self.anomaly_scorer.event_score(item)))[0].event_id


def _svc(service_id: str) -> str:
    return service_id.strip().lower()


def _fallback_unknown(
    incident_id: str,
    events: list[NormalizedEvent],
    scorer: object,
) -> list[RawHypothesis]:
    services = sorted({e.service_id for e in events})
    max_anomaly = max(scorer.event_score(e) for e in events)
    ids = tuple(e.event_id for e in sorted(events, key=lambda e: (e.timestamp, e.event_id)))
    root = min(ids, key=lambda eid: next(e.timestamp for e in events if e.event_id == eid))
    return [
        RawHypothesis(
            hypothesis_id=f"{incident_id}-unknown",
            cause_type=CauseType.UNKNOWN,
            cause_subtype="anomaly_detected",
            title="Elevated events without a matching template",
            description=f"Elevated WARN+ events detected on {', '.join(services)}",
            root_cause_event_id=root,
            affected_services=tuple(services),
            supporting_events=ids,
            suggested_checks=(
                "Inspect grouped event timeline",
                "Add service topology for stronger correlation",
            ),
            generation_rule="fallback_unknown",
            generation_confidence=max(0.4, min(0.62, max_anomaly)),
        )
    ]


SimpleHypothesisEngine = HypothesisEngine


def hypothesis_dict_to_scored(payload: dict[str, Any]) -> ScoredHypothesis:
    score_fields = {f.name for f in fields(ScoreBreakdown)}
    defaults = {f.name: 0.0 for f in fields(ScoreBreakdown)}
    raw_breakdown = dict(payload.get("score_breakdown") or {})
    score_data = {**defaults, **{k: float(v) for k, v in raw_breakdown.items() if k in score_fields}}
    return ScoredHypothesis(
        hypothesis_id=str(payload["hypothesis_id"]),
        rank=int(payload.get("rank") or 0),
        cause_type=CauseType(str(payload["cause_type"])),
        description=str(payload["description"]),
        total_score=float(payload.get("total_score") or 0.0),
        score_breakdown=ScoreBreakdown(**score_data),
        supporting_events=list(payload.get("supporting_events") or []),
        contradicting_events=list(payload.get("contradicting_events") or []),
        affected_services=sorted(payload.get("affected_services") or []),
        suggested_checks=list(payload.get("suggested_checks") or []),
        confidence_label=str(payload.get("confidence_label") or "low"),
        is_valid=bool(payload.get("is_valid", True)),
        invalidation_reasons=list(payload.get("invalidation_reasons") or []),
    )
