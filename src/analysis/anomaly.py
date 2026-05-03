from __future__ import annotations

from collections import defaultdict
from typing import Any

from core.enums import Severity
from events.models import NormalizedEvent


class AnomalyScorer:
    def event_score(self, event: NormalizedEvent) -> float:
        score = self._severity_score(event.severity)
        score = max(score, self._tag_score(event.tags))
        score = max(score, self._metric_score(event.structured_data))
        return round(max(0.0, min(1.0, score)), 4)

    def service_scores(self, events: list[NormalizedEvent]) -> dict[str, float]:
        values: dict[str, list[float]] = defaultdict(list)
        for event in events:
            values[event.service_id].append(self.event_score(event))
        return {service_id: round(max(scores), 4) for service_id, scores in values.items() if scores}

    def _severity_score(self, severity: Severity) -> float:
        if severity >= Severity.CRITICAL:
            return 0.95
        if severity >= Severity.ERROR:
            return 0.75
        if severity >= Severity.WARN:
            return 0.45
        return 0.1

    def _tag_score(self, tags: frozenset[str]) -> float:
        if tags & {"oom", "disk_full", "crash"}:
            return 0.9
        if tags & {"resource_pressure", "timeout", "connection_refused"}:
            return 0.75
        if tags & {"restart", "deployment", "config_change"}:
            return 0.55
        return 0.0

    def _metric_score(self, structured_data: dict[str, Any]) -> float:
        metrics = structured_data.get("metrics")
        if not isinstance(metrics, dict):
            process = structured_data.get("process")
            metrics = process if isinstance(process, dict) else {}
        score = 0.0
        cpu = _float(metrics.get("cpu_percent"))
        memory_percent = _float(metrics.get("memory_percent"))
        disk_percent = _float(metrics.get("disk_percent"))
        memory_mb = _float(metrics.get("memory_mb"))
        if cpu is not None:
            score = max(score, _scaled(cpu, warning=75.0, critical=95.0))
        if memory_percent is not None:
            score = max(score, _scaled(memory_percent, warning=80.0, critical=95.0))
        if disk_percent is not None:
            score = max(score, _scaled(disk_percent, warning=85.0, critical=98.0))
        if memory_mb is not None:
            score = max(score, _scaled(memory_mb, warning=512.0, critical=4096.0))
        return score


def _scaled(value: float, warning: float, critical: float) -> float:
    if value < warning:
        return 0.0
    if value >= critical:
        return 0.95
    return 0.55 + ((value - warning) / (critical - warning)) * 0.4


def _float(value: Any) -> float | None:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None
