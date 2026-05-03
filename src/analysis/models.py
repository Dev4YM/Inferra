from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime

from core.enums import Severity


@dataclass(frozen=True)
class CorrelationEdge:
    source_event_id: str
    target_event_id: str
    edge_type: str
    weight: float
    evidence: str


@dataclass(frozen=True)
class EventCluster:
    cluster_id: str
    events: list[str]
    time_range: tuple[datetime, datetime]
    affected_services: set[str]
    primary_severity: Severity
    trigger_event_id: str
    correlation_edges: list[CorrelationEdge]
    anomaly_scores: dict[str, float]
