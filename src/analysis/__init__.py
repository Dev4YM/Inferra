from __future__ import annotations

from typing import Any

from analysis.anomaly import (
    AnomalyScorer,
    anomaly_service_status_to_json,
    build_anomaly_service_status,
    reconcile_baseline_from_events,
)
from analysis.correlation import CorrelationEngine
from analysis.models import AnomalyBucketStatus, AnomalyServiceStatus, CorrelationEdge, EventCluster

__all__ = [
    "AnomalyBucketStatus",
    "AnomalyScorer",
    "AnomalyServiceStatus",
    "CorrelationEdge",
    "CorrelationEngine",
    "EventCluster",
    "IncidentLifecycleManager",
    "anomaly_service_status_to_json",
    "build_anomaly_service_status",
    "reconcile_baseline_from_events",
]


def __getattr__(name: str) -> Any:
    if name == "IncidentLifecycleManager":
        from analysis.lifecycle import IncidentLifecycleManager

        return IncidentLifecycleManager
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
