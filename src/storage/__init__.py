from __future__ import annotations

from datetime import datetime
from pathlib import Path
from typing import Any, Protocol

from core.models import AnomalyResult, BaselineMetric, CalibrationModel, WeightSnapshot, WeightState

from .baseline_store import BaselineStore as JsonBaselineStore
from .calibration_store import (
    CalibrationStore as JsonCalibrationStore,
    assign_confidence_label,
    check_calibration_staleness,
    label_from_score_thresholds,
    update_calibration,
)
from .connection import SqliteConnectionPool, connect_sqlite, transaction
from .event_store import EventStore, SqliteEventStore, count_events_by_service
from .incident_store import IncidentStore, SqliteIncidentStore
from .metric_ringbuffer import MetricRingbuffer
from .migrations import (
    CURRENT_SCHEMA_VERSION,
    Migration,
    backup_db,
    integrity_check,
    migrate,
    vacuum_db,
)
from .service_graph import ServiceGraphCache as JsonServiceGraphStore
from .weight_store import DEFAULT_WEIGHTS, WeightStore as JsonWeightStore, reset_weights, update_weights


class BaselineStore(Protocol):
    def update_baseline(
        self,
        service_id: str,
        metric_name: str,
        bucket_idx: int,
        observed_value: float,
        alpha: float = 0.1,
    ) -> None: ...

    def get_baseline(self, service_id: str, metric_name: str) -> BaselineMetric: ...

    def get_all_baselines(self) -> dict[str, dict[str, BaselineMetric]]: ...

    def save_service(self, service_id: str) -> None: ...

    def is_learning(self, service_id: str, *, now: datetime | None = None) -> bool: ...

    def touch_first_event(self, service_id: str, at: datetime) -> None: ...

    def reconcile_closed_buckets(
        self,
        service_id: str,
        closed_buckets: list[dict[str, Any]],
        *,
        alpha: float,
        last_event_timestamp: datetime | None,
    ) -> None: ...

    def fingerprint_observation_score(
        self,
        service_id: str,
        fingerprint: str,
        observed_count: float,
        *,
        now: datetime | None = None,
    ) -> AnomalyResult: ...

    def bucket_history_snapshot(self, service_id: str) -> list[dict[str, Any]]: ...


class ServiceGraphStore(Protocol):
    def add_relation(
        self,
        source: str,
        target: str,
        relation_type: str,
        origin: str = "config",
        confidence: str = "high",
    ) -> None: ...

    def get_dependencies(self, service_id: str) -> list[str]: ...

    def get_dependents(self, service_id: str) -> list[str]: ...

    def edges(self) -> list[dict[str, str]]: ...

    def persist(self) -> None: ...


class WeightStore(Protocol):
    def load(self) -> WeightState: ...

    def save(self, state: WeightState) -> None: ...

    def append_history(self, snapshot: WeightSnapshot) -> None: ...


class CalibrationStore(Protocol):
    def load(self) -> CalibrationModel: ...

    def save(self, model: CalibrationModel) -> None: ...


def initialize_storage(
    data_dir: Path,
    *,
    events_db_name: str = "events.db",
    incidents_db_name: str = "incidents.db",
    retention_hours: int = 72,
    prune_interval_seconds: int = 60,
    wal_mode: bool = True,
    mmap_size_bytes: int = 0,
    start_pruner: bool = True,
    archive_after_days: int = 7,
) -> tuple[
    SqliteEventStore,
    SqliteIncidentStore,
    JsonBaselineStore,
    JsonServiceGraphStore,
    JsonWeightStore,
    JsonCalibrationStore,
]:
    data_dir = Path(data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)
    return (
        SqliteEventStore(
            data_dir / events_db_name,
            retention_hours=retention_hours,
            prune_interval_seconds=prune_interval_seconds,
            wal_mode=wal_mode,
            mmap_size_bytes=mmap_size_bytes,
            start_pruner=start_pruner,
        ),
        SqliteIncidentStore(
            data_dir / incidents_db_name,
            wal_mode=wal_mode,
            mmap_size_bytes=mmap_size_bytes,
            archive_after_days=archive_after_days,
        ),
        JsonBaselineStore(data_dir / "baselines"),
        JsonServiceGraphStore(data_dir / "service_graph_cache.json", discover_docker=False),
        JsonWeightStore(data_dir / "scoring_weights.json", data_dir / "weight_history.jsonl"),
        JsonCalibrationStore(data_dir / "calibration.json"),
    )


__all__ = [
    "CURRENT_SCHEMA_VERSION",
    "BaselineStore",
    "CalibrationStore",
    "DEFAULT_WEIGHTS",
    "EventStore",
    "IncidentStore",
    "JsonBaselineStore",
    "JsonCalibrationStore",
    "JsonServiceGraphStore",
    "JsonWeightStore",
    "MetricRingbuffer",
    "Migration",
    "ServiceGraphStore",
    "SqliteConnectionPool",
    "SqliteEventStore",
    "SqliteIncidentStore",
    "WeightStore",
    "backup_db",
    "assign_confidence_label",
    "check_calibration_staleness",
    "label_from_score_thresholds",
    "connect_sqlite",
    "count_events_by_service",
    "initialize_storage",
    "integrity_check",
    "migrate",
    "reset_weights",
    "transaction",
    "update_calibration",
    "update_weights",
    "vacuum_db",
]
