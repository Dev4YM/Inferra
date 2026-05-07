from __future__ import annotations

import re
from collections.abc import Mapping
from dataclasses import dataclass
from datetime import timedelta
from typing import Any, Literal

from config.models import ContradictionHandlingConfig
from core.enums import CauseType, EventType, Severity
from events.models import NormalizedEvent

PenaltyTier = Literal["strong", "weak"]


@dataclass(frozen=True)
class ContradictionRecord:
    event_id: str
    contradiction_type: str
    explanation: str
    penalty_tier: PenaltyTier


class ContradictionHandler:
    def __init__(self, config: ContradictionHandlingConfig) -> None:
        self._config = config

    def detect(self, hypothesis: dict[str, Any], events: list[NormalizedEvent]) -> list[ContradictionRecord]:
        if not self._config.enabled:
            return []
        rules = self._config.rules
        by_id = {event.event_id: event for event in events}
        supporting = [by_id[event_id] for event_id in hypothesis.get("supporting_events", []) if event_id in by_id]
        out: list[ContradictionRecord] = []
        if rules.timeline_violation:
            out.extend(self._timeline_violations(hypothesis, supporting, by_id))
        if rules.health_check:
            out.extend(self._health_check(hypothesis, supporting, events))
        if rules.resource_state:
            out.extend(self._resource_state(hypothesis, events))
        if rules.scope_mismatch:
            out.extend(self._scope_mismatch(hypothesis, supporting))
        if rules.mutual_exclusion:
            out.extend(self._mutual_exclusion(hypothesis, supporting))
        return out

    def penalty_multiplier(self, records: list[ContradictionRecord]) -> float:
        if not records:
            return 1.0
        strong = sum(1 for item in records if item.penalty_tier == "strong")
        weak = sum(1 for item in records if item.penalty_tier == "weak")
        raw = 1.0 - strong * float(self._config.strong_penalty_per_contradiction) - weak * float(
            self._config.weak_penalty_per_contradiction
        )
        floor = float(self._config.min_penalty_multiplier)
        return round(max(floor, raw), 4)

    def _timeline_violations(
        self,
        hypothesis: dict[str, Any],
        supporting: list[NormalizedEvent],
        by_id: dict[str, NormalizedEvent],
    ) -> list[ContradictionRecord]:
        root_id = hypothesis.get("root_cause_event_id")
        if not root_id or root_id not in by_id:
            return []
        root = by_id[root_id]
        tol = timedelta(seconds=float(self._config.timeline_tolerance_seconds))
        out: list[ContradictionRecord] = []
        for event in supporting:
            if event.event_id == root_id:
                continue
            if event.timestamp + tol >= root.timestamp:
                continue
            latency = (root.timestamp - event.timestamp).total_seconds()
            tier: PenaltyTier = "strong" if latency > 30.0 else "weak"
            out.append(
                ContradictionRecord(
                    event.event_id,
                    "timeline_violation",
                    f"Evidence on {event.service_id} precedes the claimed root cause by {latency:.0f}s.",
                    tier,
                )
            )
        return out

    def _health_check(
        self,
        hypothesis: dict[str, Any],
        supporting: list[NormalizedEvent],
        all_events: list[NormalizedEvent],
    ) -> list[ContradictionRecord]:
        if not supporting:
            return []
        affected = set(hypothesis.get("affected_services") or [])
        start = min(event.timestamp for event in supporting)
        end = max(event.timestamp for event in supporting)
        out: list[ContradictionRecord] = []
        for event in all_events:
            if event.service_id not in affected:
                continue
            lower = event.message.lower()
            is_health_signal = event.event_type == EventType.HEALTH_CHECK or "health check" in lower
            if not is_health_signal:
                continue
            if not any(token in lower for token in ("pass", "healthy", "ok", "success")):
                continue
            if not (start <= event.timestamp <= end):
                continue
            window_start = event.timestamp - timedelta(seconds=15)
            window_end = event.timestamp + timedelta(seconds=15)
            concurrent_failures = [
                item
                for item in supporting
                if item.service_id == event.service_id
                and item.severity >= Severity.ERROR
                and window_start <= item.timestamp <= window_end
            ]
            tier: PenaltyTier = "weak" if concurrent_failures else "strong"
            out.append(
                ContradictionRecord(
                    event.event_id,
                    "health_check",
                    f"Health check on {event.service_id} reported healthy during the incident window.",
                    tier,
                )
            )
        return out

    def _resource_state(self, hypothesis: dict[str, Any], all_events: list[NormalizedEvent]) -> list[ContradictionRecord]:
        if hypothesis.get("cause_type") != CauseType.RESOURCE_EXHAUSTION.value:
            return []
        out: list[ContradictionRecord] = []
        for event in all_events:
            metrics = event.structured_data.get("metrics")
            if not isinstance(metrics, Mapping):
                continue
            cpu = _as_float(metrics.get("cpu_percent"))
            memory = _as_float(metrics.get("memory_percent"))
            disk = _as_float(metrics.get("disk_percent"))
            if cpu is None or memory is None or disk is None:
                continue
            if cpu < 50.0 and memory < 60.0 and disk < 75.0:
                out.append(
                    ContradictionRecord(
                        event.event_id,
                        "resource_state",
                        "Host metrics show low CPU, memory, and disk during a resource exhaustion hypothesis.",
                        "strong",
                    )
                )
        return out

    def _scope_mismatch(
        self,
        hypothesis: dict[str, Any],
        supporting: list[NormalizedEvent],
    ) -> list[ContradictionRecord]:
        affected = set(hypothesis.get("affected_services") or [])
        if not affected:
            return []
        out: list[ContradictionRecord] = []
        for event in supporting:
            if event.service_id in affected:
                continue
            out.append(
                ContradictionRecord(
                    event.event_id,
                    "scope_mismatch",
                    f"Supporting event on {event.service_id} is outside declared affected services.",
                    "weak",
                )
            )
        return out

    def _mutual_exclusion(
        self,
        hypothesis: dict[str, Any],
        supporting: list[NormalizedEvent],
    ) -> list[ContradictionRecord]:
        if hypothesis.get("cause_type") != CauseType.CONFIGURATION_ERROR.value:
            return []
        if not supporting:
            return []
        pattern = re.compile(r"\b(config|yaml|json|env|password|secret|setting|misconfig)\b", re.I)
        hits = sum(1 for event in supporting if pattern.search(event.message))
        if hits >= max(1, len(supporting) // 2):
            return []
        sample = supporting[0]
        return [
            ContradictionRecord(
                sample.event_id,
                "mutual_exclusion",
                "Configuration hypothesis lacks configuration-oriented evidence in supporting events.",
                "weak",
            )
        ]


def _as_float(value: Any) -> float | None:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None
