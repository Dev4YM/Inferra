from __future__ import annotations

import json
import threading
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any

from core.models import AnomalyResult, BaselineMetric
from core.time import parse_datetime, to_iso, utc_now

SERVICE_METRICS = (
    "event_volume",
    "error_rate",
    "warn_rate",
    "unique_fingerprints",
    "new_fingerprint_rate",
    "restart_count",
    "mean_severity",
)

SYSTEM_METRICS = (
    "total_event_volume",
    "active_services",
    "cross_service_error_rate",
)

BUCKET_COUNT = 168
SYSTEM_SERVICE_ID = "__system__"
MAX_BUCKET_HISTORY = 288


@dataclass
class FingerprintEmaState:
    ema: float = 0.0
    ema_dev: float = 0.0
    updates: int = 0
    last_bucket_id: int | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "ema": self.ema,
            "ema_dev": self.ema_dev,
            "updates": self.updates,
            "last_bucket_id": self.last_bucket_id,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> FingerprintEmaState:
        return cls(
            ema=float(data.get("ema") or 0.0),
            ema_dev=float(data.get("ema_dev") or 0.0),
            updates=int(data.get("updates") or 0),
            last_bucket_id=int(data["last_bucket_id"]) if data.get("last_bucket_id") is not None else None,
        )


@dataclass
class ServiceExtraState:
    first_event_at: datetime | None = None
    last_reconciled_event_ts: datetime | None = None
    fingerprints: dict[str, FingerprintEmaState] = field(default_factory=dict)
    bucket_history: list[dict[str, Any]] = field(default_factory=list)
    applied_fp_bucket_counts: dict[str, int] = field(default_factory=dict)

    def prune_applied(self, min_bucket_id: int) -> None:
        stale: list[str] = []
        for key in self.applied_fp_bucket_counts:
            try:
                bucket_part = int(key.split("\t", 1)[0])
            except (TypeError, ValueError):
                stale.append(key)
                continue
            if bucket_part < min_bucket_id:
                stale.append(key)
        for key in stale:
            self.applied_fp_bucket_counts.pop(key, None)


class BaselineStore:
    def __init__(
        self,
        base_dir: str | Path = "./data/baselines",
        *,
        cold_start_hours: int = 6,
        min_samples_for_confidence: int = 4,
    ) -> None:
        self.base_dir = Path(base_dir)
        self.cold_start_hours = cold_start_hours
        self.min_samples_for_confidence = min_samples_for_confidence
        self._lock = threading.RLock()
        self._baselines: dict[str, dict[str, BaselineMetric]] = {}
        self._extras: dict[str, ServiceExtraState] = {}
        self._load_all()

    def is_learning(self, service_id: str, *, now: datetime | None = None) -> bool:
        now = now or utc_now()
        with self._lock:
            first = self._extras.get(service_id, ServiceExtraState()).first_event_at
            if first is None:
                return True
            return now < first + timedelta(hours=self.cold_start_hours)

    def touch_first_event(self, service_id: str, at: datetime) -> None:
        with self._lock:
            extras = self._extras.setdefault(service_id, ServiceExtraState())
            if extras.first_event_at is None:
                extras.first_event_at = at
                self._save_service(service_id)

    def reconcile_closed_buckets(
        self,
        service_id: str,
        closed_buckets: list[dict[str, Any]],
        *,
        alpha: float,
        last_event_timestamp: datetime | None,
    ) -> None:
        if not closed_buckets:
            return
        with self._lock:
            extras = self._extras.setdefault(service_id, ServiceExtraState())
            for row in sorted(closed_buckets, key=lambda item: int(item["bucket_id"])):
                self._merge_one_bucket_row(service_id, extras, row, alpha=alpha)
            if last_event_timestamp is not None:
                if extras.last_reconciled_event_ts is None or last_event_timestamp > extras.last_reconciled_event_ts:
                    extras.last_reconciled_event_ts = last_event_timestamp
            min_bucket = min(int(row["bucket_id"]) for row in closed_buckets)
            extras.prune_applied(min_bucket - MAX_BUCKET_HISTORY)
            self._save_service(service_id)

    def fingerprint_expected_count(self, service_id: str, fingerprint: str) -> tuple[float, float, int]:
        with self._lock:
            state = self._extras.get(service_id, ServiceExtraState()).fingerprints.get(fingerprint)
            if state is None:
                return 0.0, 0.0, 0
            return state.ema, state.ema_dev, state.updates

    def bucket_history_snapshot(self, service_id: str) -> list[dict[str, Any]]:
        with self._lock:
            return [dict(row) for row in self._extras.get(service_id, ServiceExtraState()).bucket_history]

    def update_baseline(
        self,
        service_id: str,
        metric_name: str,
        bucket_idx: int,
        observed_value: float,
        alpha: float = 0.1,
    ) -> None:
        metric = self.get_baseline(service_id, metric_name)
        index = bucket_idx % BUCKET_COUNT
        with self._lock:
            self._touch_first_seen_locked(service_id, utc_now())
            if metric.sample_counts[index] == 0:
                metric.buckets[index] = observed_value
                metric.stddev[index] = 0.0
            else:
                old_mean = metric.buckets[index]
                old_std = metric.stddev[index]
                new_mean = alpha * observed_value + (1.0 - alpha) * old_mean
                deviation = abs(observed_value - old_mean)
                new_std = alpha * deviation + (1.0 - alpha) * old_std
                metric.buckets[index] = new_mean
                metric.stddev[index] = new_std
            metric.sample_counts[index] += 1
            metric.last_updated = utc_now()
            self._save_service(service_id)

    def get_baseline(self, service_id: str, metric_name: str) -> BaselineMetric:
        with self._lock:
            service_metrics = self._baselines.setdefault(service_id, {})
            metric = service_metrics.get(metric_name)
            if metric is None:
                metric = BaselineMetric(
                    metric_name=metric_name,
                    service_id=service_id,
                    buckets=[0.0] * BUCKET_COUNT,
                    stddev=[0.0] * BUCKET_COUNT,
                    sample_counts=[0] * BUCKET_COUNT,
                    min_samples_for_confidence=self.min_samples_for_confidence,
                )
                service_metrics[metric_name] = metric
            self._extras.setdefault(service_id, ServiceExtraState())
            return metric

    def get_all_baselines(self) -> dict[str, dict[str, BaselineMetric]]:
        with self._lock:
            return {
                service_id: {metric_name: self._clone_metric(metric) for metric_name, metric in metrics.items()}
                for service_id, metrics in self._baselines.items()
            }

    def load_service(self, service_id: str) -> dict[str, BaselineMetric]:
        with self._lock:
            self._load_service(service_id)
            return {
                metric_name: self._clone_metric(metric)
                for metric_name, metric in self._baselines.get(service_id, {}).items()
            }

    def save_service(self, service_id: str) -> None:
        with self._lock:
            self._save_service(service_id)

    def compute_anomaly_score(
        self,
        observed: float,
        baseline: BaselineMetric,
        bucket_idx: int,
        *,
        service_id: str | None = None,
        now: datetime | None = None,
    ) -> AnomalyResult:
        index = bucket_idx % BUCKET_COUNT
        expected = baseline.buckets[index]
        std = baseline.stddev[index]
        samples = baseline.sample_counts[index]
        now = now or utc_now()
        with self._lock:
            learning = bool(service_id) and self._is_learning_locked(service_id, now)
        if learning:
            return AnomalyResult(
                score=0.0,
                confidence="learning",
                z_score=0.0,
                expected=expected,
                observed=observed,
                std=std,
            )
        if samples < baseline.min_samples_for_confidence:
            return AnomalyResult(
                score=0.0,
                confidence="insufficient_data",
                z_score=0.0,
                expected=expected,
                observed=observed,
                std=std,
            )
        if std < 1e-6:
            if abs(observed - expected) < 1e-6:
                return AnomalyResult(
                    score=0.0,
                    confidence="normal",
                    z_score=0.0,
                    expected=expected,
                    observed=observed,
                    std=std,
                )
            return AnomalyResult(
                score=1.0,
                confidence="high",
                z_score=float("inf"),
                expected=expected,
                observed=observed,
                std=std,
            )

        z_score = (observed - expected) / std
        raw_score = 1.0 - 1.0 / (1.0 + (abs(z_score) / 3.0) ** 2)
        confidence = "high" if samples >= 20 else "medium" if samples >= 8 else "low"
        return AnomalyResult(
            score=raw_score,
            confidence=confidence,
            z_score=z_score,
            expected=expected,
            observed=observed,
            std=std,
        )

    def fingerprint_observation_score(
        self,
        service_id: str,
        fingerprint: str,
        observed_count: float,
        *,
        now: datetime | None = None,
    ) -> AnomalyResult:
        now = now or utc_now()
        with self._lock:
            if self._is_learning_locked(service_id, now):
                return AnomalyResult(0.0, "learning", 0.0, 0.0, observed_count, 0.0)
            state = self._extras.setdefault(service_id, ServiceExtraState()).fingerprints.get(fingerprint)
            if state is None or state.updates < self.min_samples_for_confidence:
                return AnomalyResult(0.0, "insufficient_data", 0.0, 0.0, observed_count, 0.0)
            expected = state.ema
            std = max(state.ema_dev, 1e-6)
            if std < 1e-6:
                if abs(observed_count - expected) < 1e-6:
                    return AnomalyResult(0.0, "normal", 0.0, expected, observed_count, std)
                return AnomalyResult(1.0, "high", float("inf"), expected, observed_count, std)
            z_score = (observed_count - expected) / std
            raw_score = 1.0 - 1.0 / (1.0 + (abs(z_score) / 3.0) ** 2)
            confidence = "high" if state.updates >= 20 else "medium" if state.updates >= 8 else "low"
            return AnomalyResult(
                score=raw_score,
                confidence=confidence,
                z_score=z_score,
                expected=expected,
                observed=observed_count,
                std=std,
            )

    def composite_anomaly_score(self, service_id: str, metric_results: dict[str, AnomalyResult]) -> float:
        del service_id
        weights = {
            "error_rate": 0.35,
            "event_volume": 0.20,
            "new_fingerprint_rate": 0.20,
            "restart_count": 0.15,
            "warn_rate": 0.10,
        }
        weighted_sum = 0.0
        total_weight = 0.0
        for metric_name, result in metric_results.items():
            if result.confidence in ("insufficient_data", "learning"):
                continue
            weight = weights.get(metric_name, 0.1)
            weighted_sum += weight * result.score
            total_weight += weight
        if total_weight == 0.0:
            return 0.0
        return weighted_sum / total_weight

    def _touch_first_seen_locked(self, service_id: str, at: datetime) -> None:
        extras = self._extras.setdefault(service_id, ServiceExtraState())
        if extras.first_event_at is None:
            extras.first_event_at = at

    def _is_learning_locked(self, service_id: str, now: datetime) -> bool:
        first = self._extras.get(service_id, ServiceExtraState()).first_event_at
        if first is None:
            return True
        return now < first + timedelta(hours=self.cold_start_hours)

    def _merge_one_bucket_row(self, service_id: str, extras: ServiceExtraState, row: dict[str, Any], *, alpha: float) -> None:
        bucket_id = int(row["bucket_id"])
        fp_counts: dict[str, int] = {str(k): int(v) for k, v in sorted((row.get("fingerprint_counts") or {}).items())}
        for fingerprint, count in fp_counts.items():
            key = f"{bucket_id}\t{fingerprint}"
            previous = extras.applied_fp_bucket_counts.get(key)
            if previous == count:
                continue
            extras.applied_fp_bucket_counts[key] = count
            self._apply_fingerprint_ema(extras, fingerprint, bucket_id, float(count), alpha=alpha)

        history_row = {
            "bucket_id": bucket_id,
            "event_volume": float(row.get("event_volume") or 0.0),
            "error_rate": float(row.get("error_rate") or 0.0),
            "warn_rate": float(row.get("warn_rate") or 0.0),
            "new_fingerprint_rate": float(row.get("new_fingerprint_rate") or 0.0),
            "restart_count": float(row.get("restart_count") or 0.0),
            "fingerprints_present": sorted(fp_counts.keys()),
        }
        if extras.bucket_history and extras.bucket_history[-1]["bucket_id"] == bucket_id:
            extras.bucket_history[-1] = history_row
        else:
            extras.bucket_history.append(history_row)
        while len(extras.bucket_history) > MAX_BUCKET_HISTORY:
            extras.bucket_history.pop(0)

        hour_idx = int(row.get("hour_of_week_index") or 0) % BUCKET_COUNT
        self._touch_first_seen_locked(service_id, parse_datetime(str(row["bucket_end"])) if row.get("bucket_end") else utc_now())
        for metric_name in ("event_volume", "error_rate", "warn_rate", "new_fingerprint_rate", "restart_count"):
            if metric_name in row:
                self._update_baseline_locked(service_id, metric_name, hour_idx, float(row[metric_name]), alpha=alpha)

    def _apply_fingerprint_ema(
        self,
        extras: ServiceExtraState,
        fingerprint: str,
        bucket_id: int,
        count: float,
        *,
        alpha: float,
    ) -> None:
        state = extras.fingerprints.setdefault(fingerprint, FingerprintEmaState())
        bucket_changed = state.last_bucket_id != bucket_id
        if state.updates == 0:
            state.ema = count
            state.ema_dev = 0.0
        else:
            old_mean = state.ema
            state.ema = alpha * count + (1.0 - alpha) * old_mean
            deviation = abs(count - old_mean)
            state.ema_dev = alpha * deviation + (1.0 - alpha) * state.ema_dev
        if bucket_changed:
            state.updates += 1
        state.last_bucket_id = bucket_id

    def _update_baseline_locked(
        self,
        service_id: str,
        metric_name: str,
        bucket_idx: int,
        observed_value: float,
        *,
        alpha: float,
    ) -> None:
        service_metrics = self._baselines.setdefault(service_id, {})
        metric = service_metrics.get(metric_name)
        if metric is None:
            metric = BaselineMetric(
                metric_name=metric_name,
                service_id=service_id,
                buckets=[0.0] * BUCKET_COUNT,
                stddev=[0.0] * BUCKET_COUNT,
                sample_counts=[0] * BUCKET_COUNT,
                min_samples_for_confidence=self.min_samples_for_confidence,
            )
            service_metrics[metric_name] = metric
        index = bucket_idx % BUCKET_COUNT
        if metric.sample_counts[index] == 0:
            metric.buckets[index] = observed_value
            metric.stddev[index] = 0.0
        else:
            old_mean = metric.buckets[index]
            old_std = metric.stddev[index]
            new_mean = alpha * observed_value + (1.0 - alpha) * old_mean
            deviation = abs(observed_value - old_mean)
            new_std = alpha * deviation + (1.0 - alpha) * old_std
            metric.buckets[index] = new_mean
            metric.stddev[index] = new_std
        metric.sample_counts[index] += 1
        metric.last_updated = utc_now()

    def _load_all(self) -> None:
        self.base_dir.mkdir(parents=True, exist_ok=True)
        for path in sorted(self.base_dir.glob("*.json")):
            self._load_service(path.stem)

    def _load_service(self, service_id: str) -> None:
        path = self.base_dir / f"{service_id}.json"
        if not path.exists():
            self._baselines.setdefault(service_id, {})
            self._extras.setdefault(service_id, ServiceExtraState())
            return
        data = json.loads(path.read_text(encoding="utf-8"))
        metrics: dict[str, BaselineMetric] = {}
        for metric_name, metric_data in (data.get("hourly_profiles") or {}).items():
            metrics[metric_name] = BaselineMetric(
                metric_name=metric_name,
                service_id=service_id,
                buckets=list(metric_data.get("buckets") or [0.0] * BUCKET_COUNT),
                stddev=list(metric_data.get("stddev") or [0.0] * BUCKET_COUNT),
                sample_counts=list(
                    metric_data.get("sample_counts")
                    or metric_data.get("sample_count")
                    or [0] * BUCKET_COUNT
                ),
                min_samples_for_confidence=int(
                    metric_data.get("min_samples_for_confidence", self.min_samples_for_confidence)
                ),
                last_updated=parse_datetime(data.get("updated_at")) if data.get("updated_at") else None,
            )
        self._baselines[service_id] = metrics
        extras = ServiceExtraState(
            first_event_at=parse_datetime(data["first_event_at"]) if data.get("first_event_at") else None,
            last_reconciled_event_ts=parse_datetime(data["last_reconciled_event_ts"])
            if data.get("last_reconciled_event_ts")
            else None,
            fingerprints={
                fp: FingerprintEmaState.from_dict(payload)
                for fp, payload in sorted((data.get("fingerprint_baselines") or {}).items())
            },
            bucket_history=list(data.get("bucket_history") or []),
            applied_fp_bucket_counts={str(k): int(v) for k, v in (data.get("applied_fp_bucket_counts") or {}).items()},
        )
        self._extras[service_id] = extras

    def _save_service(self, service_id: str) -> None:
        metrics = self._baselines.get(service_id, {})
        extras = self._extras.setdefault(service_id, ServiceExtraState())
        payload: dict[str, Any] = {
            "service_id": service_id,
            "schema_version": 2,
            "updated_at": to_iso(utc_now()),
            "first_event_at": to_iso(extras.first_event_at) if extras.first_event_at else None,
            "last_reconciled_event_ts": to_iso(extras.last_reconciled_event_ts) if extras.last_reconciled_event_ts else None,
            "hourly_profiles": {
                metric_name: {
                    "buckets": metric.buckets,
                    "stddev": metric.stddev,
                    "sample_counts": metric.sample_counts,
                    "min_samples_for_confidence": metric.min_samples_for_confidence,
                }
                for metric_name, metric in metrics.items()
            },
            "fingerprint_baselines": {fp: state.to_dict() for fp, state in sorted(extras.fingerprints.items())},
            "bucket_history": extras.bucket_history,
            "applied_fp_bucket_counts": dict(sorted(extras.applied_fp_bucket_counts.items())),
        }
        path = self.base_dir / f"{service_id}.json"
        path.parent.mkdir(parents=True, exist_ok=True)
        temp_path = path.with_suffix(".json.tmp")
        temp_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
        temp_path.replace(path)

    def _clone_metric(self, metric: BaselineMetric) -> BaselineMetric:
        return BaselineMetric(
            metric_name=metric.metric_name,
            service_id=metric.service_id,
            buckets=list(metric.buckets),
            stddev=list(metric.stddev),
            sample_counts=list(metric.sample_counts),
            min_samples_for_confidence=metric.min_samples_for_confidence,
            last_updated=metric.last_updated,
        )

    @staticmethod
    def hour_of_week_index(dt: datetime) -> int:
        return dt.weekday() * 24 + dt.hour

    @staticmethod
    def wall_bucket_id(timestamp: datetime, interval_minutes: int) -> int:
        return int(timestamp.timestamp()) // max(1, interval_minutes * 60)

    @staticmethod
    def known_metric_names() -> tuple[str, ...]:
        return SERVICE_METRICS + SYSTEM_METRICS
