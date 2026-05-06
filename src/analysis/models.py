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
    reason_codes: tuple[str, ...] = ()


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


@dataclass(frozen=True)
class AnomalyBucketStatus:
    bucket_id: int
    event_volume: int
    error_rate: float
    warn_rate: float
    new_fingerprint_rate: float
    restart_count: int
    bucket_score: float
    spike_z: float
    spike: bool
    sustained_mean: float


@dataclass(frozen=True)
class AnomalyServiceStatus:
    service_id: str
    status: str
    bucket_interval_minutes: int
    absence_score: float
    absence_missing_fingerprints: tuple[str, ...]
    buckets: tuple[AnomalyBucketStatus, ...]
