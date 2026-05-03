from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta
from typing import Any

from core.enums import CauseType, EventType, Severity
from events.models import NormalizedEvent


@dataclass(frozen=True)
class Contradiction:
    event_id: str
    contradiction_type: str
    explanation: str
    severity: str


class HypothesisValidator:
    def validate(self, hypothesis: dict[str, Any], events: list[NormalizedEvent]) -> dict[str, Any]:
        by_id = {event.event_id: event for event in events}
        supporting = [by_id[event_id] for event_id in hypothesis.get("supporting_events", []) if event_id in by_id]
        contradictions = self._detect_contradictions(hypothesis, supporting, events)
        penalty = self._contradiction_penalty(contradictions)
        score = round(float(hypothesis.get("total_score") or 0.0) * penalty, 4)
        invalidation_reasons = list(hypothesis.get("invalidation_reasons", []))
        invalidation_reasons.extend(item.explanation for item in contradictions)
        contradiction_ratio = len(contradictions) / max(1, len(supporting) + len(contradictions))
        is_valid = bool(supporting) and contradiction_ratio <= 0.6
        if not supporting:
            invalidation_reasons.append("No supporting evidence exists for this hypothesis.")
        if contradiction_ratio > 0.3:
            invalidation_reasons.append(f"High contradiction ratio: {contradiction_ratio:.0%}.")
        updated = dict(hypothesis)
        updated["total_score"] = score
        updated["contradicting_events"] = [item.event_id for item in contradictions if item.event_id]
        updated["is_valid"] = is_valid
        updated["invalidation_reasons"] = invalidation_reasons
        breakdown = dict(updated.get("score_breakdown", {}))
        breakdown["contradiction_penalty"] = penalty
        breakdown["contradiction_count"] = len(contradictions)
        updated["score_breakdown"] = breakdown
        return updated

    def _detect_contradictions(
        self,
        hypothesis: dict[str, Any],
        supporting: list[NormalizedEvent],
        all_events: list[NormalizedEvent],
    ) -> list[Contradiction]:
        contradictions: list[Contradiction] = []
        contradictions.extend(self._health_check_contradictions(hypothesis, supporting, all_events))
        contradictions.extend(self._resource_state_contradictions(hypothesis, all_events))
        contradictions.extend(self._timeline_contradictions(hypothesis, supporting))
        return contradictions

    def _health_check_contradictions(
        self,
        hypothesis: dict[str, Any],
        supporting: list[NormalizedEvent],
        all_events: list[NormalizedEvent],
    ) -> list[Contradiction]:
        if not supporting:
            return []
        affected = set(hypothesis.get("affected_services") or [])
        start = min(event.timestamp for event in supporting)
        end = max(event.timestamp for event in supporting)
        contradictions: list[Contradiction] = []
        for event in all_events:
            if event.event_type != EventType.HEALTH_CHECK or event.service_id not in affected:
                continue
            lower = event.message.lower()
            if not any(token in lower for token in ("pass", "healthy", "ok", "success")):
                continue
            if not (start <= event.timestamp <= end):
                continue
            window_start = event.timestamp - timedelta(seconds=15)
            window_end = event.timestamp + timedelta(seconds=15)
            concurrent_failures = [
                item
                for item in supporting
                if item.service_id == event.service_id and item.severity >= Severity.ERROR and window_start <= item.timestamp <= window_end
            ]
            severity = "weak" if concurrent_failures else "strong"
            contradictions.append(
                Contradiction(
                    event.event_id,
                    "health_check",
                    f"Health check on {event.service_id} passed during the incident window.",
                    severity,
                )
            )
        return contradictions

    def _resource_state_contradictions(
        self,
        hypothesis: dict[str, Any],
        all_events: list[NormalizedEvent],
    ) -> list[Contradiction]:
        if hypothesis.get("cause_type") != CauseType.RESOURCE_EXHAUSTION.value:
            return []
        contradictions: list[Contradiction] = []
        for event in all_events:
            metrics = event.structured_data.get("metrics")
            if not isinstance(metrics, dict):
                continue
            cpu = _as_float(metrics.get("cpu_percent"))
            memory = _as_float(metrics.get("memory_percent"))
            disk = _as_float(metrics.get("disk_percent"))
            if cpu is not None and memory is not None and disk is not None and cpu < 50 and memory < 60 and disk < 75:
                contradictions.append(
                    Contradiction(
                        event.event_id,
                        "resource_state",
                        "Host metrics show low CPU, memory, and disk usage during a resource exhaustion hypothesis.",
                        "strong",
                    )
                )
        return contradictions

    def _timeline_contradictions(
        self,
        hypothesis: dict[str, Any],
        supporting: list[NormalizedEvent],
    ) -> list[Contradiction]:
        root_id = hypothesis.get("root_cause_event_id")
        if not root_id:
            return []
        by_id = {event.event_id: event for event in supporting}
        root = by_id.get(root_id)
        if root is None:
            return []
        contradictions: list[Contradiction] = []
        for event in supporting:
            if event.event_id == root_id:
                continue
            latency = (root.timestamp - event.timestamp).total_seconds()
            if latency > 5.0:
                contradictions.append(
                    Contradiction(
                        event.event_id,
                        "timeline_violation",
                        f"Evidence event on {event.service_id} occurred {latency:.0f}s before the claimed root cause.",
                        "strong" if latency > 30 else "weak",
                    )
                )
        return contradictions

    def _contradiction_penalty(self, contradictions: list[Contradiction]) -> float:
        strong = sum(1 for item in contradictions if item.severity == "strong")
        weak = sum(1 for item in contradictions if item.severity == "weak")
        return round(max(0.5, 1.0 - strong * 0.15 - weak * 0.05), 4)


def _as_float(value: Any) -> float | None:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None
