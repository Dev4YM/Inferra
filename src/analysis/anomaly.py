from __future__ import annotations

import statistics
from collections import defaultdict
from collections.abc import Mapping, Sequence
from datetime import datetime, timezone
from typing import Any

from analysis.models import AnomalyBucketStatus, AnomalyServiceStatus
from config.models import AnomalyDetectionConfig
from core.enums import Severity
from core.time import to_iso, utc_now
from events.models import NormalizedEvent
from storage.baseline_store import BaselineStore


class AnomalyScorer:
    def __init__(self, config: AnomalyDetectionConfig | None = None) -> None:
        self._config = config or AnomalyDetectionConfig()

    def event_score(
        self,
        event: NormalizedEvent,
        *,
        fingerprint_anomaly: float | None = None,
        baseline_store: BaselineStore | None = None,
        now: datetime | None = None,
    ) -> float:
        weights = self._config.event_score_weights
        sev = self._severity_score(event.severity)
        resource = max(self._tag_score(event.tags), self._metric_score(event.structured_data))
        if fingerprint_anomaly is not None:
            fp_score = max(0.0, min(1.0, float(fingerprint_anomaly)))
        elif baseline_store is not None:
            fp_res = baseline_store.fingerprint_observation_score(
                event.service_id,
                event.fingerprint,
                1.0,
                now=now,
            )
            fp_score = 0.0 if fp_res.confidence in ("learning", "insufficient_data") else float(fp_res.score)
        else:
            fp_score = 0.0
        total = weights.severity * sev + weights.fingerprint_anomaly * fp_score + weights.resource_tag * resource
        return round(max(0.0, min(1.0, total)), 4)

    def service_scores(self, events: list[NormalizedEvent]) -> dict[str, float]:
        values: dict[str, list[float]] = defaultdict(list)
        for event in events:
            values[event.service_id].append(self.event_score(event))
        return {service_id: round(max(scores), 4) for service_id, scores in values.items() if scores}

    def bucket_score_from_metric_scores(self, metric_scores: dict[str, float]) -> float:
        weights = self._config.weights
        keys = ("error_rate", "event_volume", "new_fingerprint_rate", "restart_count", "warn_rate")
        numerator = 0.0
        denominator = 0.0
        for key in keys:
            weight = float(getattr(weights, key))
            numerator += weight * metric_scores.get(key, 0.0)
            denominator += weight
        if denominator <= 0.0:
            return 0.0
        return round(max(0.0, min(1.0, numerator / denominator)), 6)

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
        if not isinstance(metrics, Mapping):
            process = structured_data.get("process")
            metrics = process if isinstance(process, Mapping) else {}
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


def _z_to_unit_score(z: float) -> float:
    return round(max(0.0, min(1.0, 1.0 - 1.0 / (1.0 + (abs(z) / 3.0) ** 2))), 6)


def _mean_std(values: Sequence[float]) -> tuple[float, float]:
    if len(values) < 2:
        return 0.0, 0.0
    mean = float(statistics.mean(values))
    std = float(statistics.pstdev(values))
    return mean, max(std, 1e-9)


def _absence_windows(config: AnomalyDetectionConfig) -> int:
    sensitivity = float(config.absence_sensitivity)
    if sensitivity >= 1.0:
        return max(1, int(round(sensitivity)))
    return max(1, int(round(1.0 / max(sensitivity, 1e-9))))


def _bucket_end_ts(bucket_id: int, interval_minutes: int) -> datetime:
    return datetime.fromtimestamp((bucket_id + 1) * interval_minutes * 60, tz=timezone.utc)


def aggregate_events_into_bucket_rows(
    events: list[NormalizedEvent],
    *,
    interval_minutes: int,
    now: datetime,
) -> tuple[list[dict[str, Any]], datetime | None]:
    if not events:
        return [], None
    interval_minutes = max(1, int(interval_minutes))
    current_bucket = int(now.timestamp()) // (interval_minutes * 60)
    by_bucket: dict[int, list[NormalizedEvent]] = defaultdict(list)
    for event in sorted(events, key=lambda item: item.timestamp):
        bid = int(event.timestamp.timestamp()) // (interval_minutes * 60)
        by_bucket[bid].append(event)

    seen_fp: set[str] = set()
    rows: list[dict[str, Any]] = []
    max_ts: datetime | None = None
    for bucket_id in sorted(by_bucket):
        if bucket_id >= current_bucket:
            continue
        bucket_events = by_bucket[bucket_id]
        max_ts = max((max_ts or bucket_events[0].timestamp), max(event.timestamp for event in bucket_events))
        volume = len(bucket_events)
        errors = sum(1 for event in bucket_events if event.severity >= Severity.ERROR)
        warns = sum(1 for event in bucket_events if event.severity >= Severity.WARN)
        restarts = sum(1 for event in bucket_events if "restart" in event.tags)
        fp_counts: dict[str, int] = defaultdict(int)
        for event in bucket_events:
            fp_counts[event.fingerprint] += 1
        new_fp = 0
        for event in bucket_events:
            if event.fingerprint not in seen_fp:
                new_fp += 1
            seen_fp.add(event.fingerprint)
        new_rate = new_fp / max(1, volume)
        hour_idx = BaselineStore.hour_of_week_index(_bucket_end_ts(bucket_id, interval_minutes))
        rows.append(
            {
                "bucket_id": bucket_id,
                "bucket_end": to_iso(_bucket_end_ts(bucket_id, interval_minutes)),
                "hour_of_week_index": hour_idx,
                "event_volume": float(volume),
                "error_rate": errors / max(1, volume),
                "warn_rate": warns / max(1, volume),
                "new_fingerprint_rate": new_rate,
                "restart_count": float(restarts),
                "fingerprint_counts": dict(sorted(fp_counts.items())),
            }
        )
    return rows, max_ts


def reconcile_baseline_from_events(
    store: BaselineStore,
    service_id: str,
    events: list[NormalizedEvent],
    *,
    config: AnomalyDetectionConfig,
    now: datetime,
) -> None:
    if not events:
        return
    first_ts = min(event.timestamp for event in events)
    store.touch_first_event(service_id, first_ts)
    rows, max_ts = aggregate_events_into_bucket_rows(events, interval_minutes=config.bucket_interval_minutes, now=now)
    if rows:
        store.reconcile_closed_buckets(
            service_id,
            rows,
            alpha=float(config.baseline_alpha),
            last_event_timestamp=max_ts,
        )


def _metric_z_scores(prior_rows: list[dict[str, Any]], current: dict[str, Any]) -> dict[str, float]:
    keys = ("error_rate", "event_volume", "new_fingerprint_rate", "restart_count", "warn_rate")
    result: dict[str, float] = {}
    for key in keys:
        history = [float(row[key]) for row in prior_rows if key in row]
        if len(history) < 2:
            result[key] = 0.0
            continue
        mean, std = _mean_std(history)
        z = (float(current[key]) - mean) / std
        result[key] = _z_to_unit_score(z)
    return result


def _spike_z_for_bucket_scores(prior_scores: list[float], current_score: float) -> float:
    if len(prior_scores) < 2:
        return 0.0
    mean, std = _mean_std(prior_scores)
    return round((current_score - mean) / std, 6)


def _sustained_mean(scores: list[float], lookback: int) -> float:
    if not scores:
        return 0.0
    window = scores[-max(1, lookback) :]
    return round(float(sum(window) / len(window)), 6)


def compute_absence_score(
    history_rows: list[dict[str, Any]],
    expected_prints: Sequence[str],
    *,
    absence_windows: int,
) -> tuple[float, tuple[str, ...]]:
    if not expected_prints or not history_rows:
        return 0.0, ()
    tail = history_rows[-absence_windows:]
    if len(tail) < absence_windows:
        return 0.0, ()
    missing: list[str] = []
    for fp in sorted(expected_prints):
        for row in tail:
            present = set(row.get("fingerprints_present") or [])
            if fp in present:
                break
        else:
            missing.append(fp)
    if not missing:
        return 0.0, ()
    return 1.0, tuple(missing)


def build_anomaly_service_status(
    service_id: str,
    events: list[NormalizedEvent],
    store: BaselineStore,
    *,
    config: AnomalyDetectionConfig,
    now: datetime | None = None,
    reconcile: bool = True,
) -> AnomalyServiceStatus:
    now = now or utc_now()
    if reconcile:
        reconcile_baseline_from_events(store, service_id, events, config=config, now=now)
    learning = store.is_learning(service_id, now=now)
    status = "learning" if learning else "active"
    history = store.bucket_history_snapshot(service_id)
    scorer = AnomalyScorer(config)
    absence_windows = _absence_windows(config)
    expected = tuple(sorted(config.expected_heartbeats.get(service_id, [])))
    absence_score, absence_missing = compute_absence_score(history, expected, absence_windows=absence_windows)

    bucket_statuses: list[AnomalyBucketStatus] = []
    prior_scores: list[float] = []
    for index, row in enumerate(history):
        prior = history[:index]
        if learning:
            metric_scores = {key: 0.0 for key in ("error_rate", "event_volume", "new_fingerprint_rate", "restart_count", "warn_rate")}
        else:
            metric_scores = _metric_z_scores(prior, row)
        bucket_score = 0.0 if learning else scorer.bucket_score_from_metric_scores(metric_scores)
        spike_z = 0.0 if learning else _spike_z_for_bucket_scores(prior_scores, bucket_score)
        spike = (not learning) and spike_z >= float(config.spike_z_threshold)
        prior_scores.append(bucket_score)
        sustained = _sustained_mean(prior_scores, int(config.sustained_lookback_buckets))
        bucket_statuses.append(
            AnomalyBucketStatus(
                bucket_id=int(row["bucket_id"]),
                event_volume=int(row["event_volume"]),
                error_rate=round(float(row["error_rate"]), 6),
                warn_rate=round(float(row["warn_rate"]), 6),
                new_fingerprint_rate=round(float(row["new_fingerprint_rate"]), 6),
                restart_count=int(row["restart_count"]),
                bucket_score=bucket_score,
                spike_z=spike_z,
                spike=spike,
                sustained_mean=sustained,
            )
        )

    return AnomalyServiceStatus(
        service_id=service_id,
        status=status,
        bucket_interval_minutes=int(config.bucket_interval_minutes),
        absence_score=round(absence_score, 6),
        absence_missing_fingerprints=absence_missing,
        buckets=tuple(bucket_statuses),
    )


def anomaly_service_status_to_json(payload: AnomalyServiceStatus) -> dict[str, Any]:
    return {
        "service_id": payload.service_id,
        "status": payload.status,
        "bucket_interval_minutes": payload.bucket_interval_minutes,
        "absence_score": payload.absence_score,
        "absence_missing_fingerprints": list(payload.absence_missing_fingerprints),
        "buckets": [
            {
                "bucket_id": bucket.bucket_id,
                "event_volume": bucket.event_volume,
                "error_rate": bucket.error_rate,
                "warn_rate": bucket.warn_rate,
                "new_fingerprint_rate": bucket.new_fingerprint_rate,
                "restart_count": bucket.restart_count,
                "bucket_score": bucket.bucket_score,
                "spike_z": bucket.spike_z,
                "spike": bucket.spike,
                "sustained_mean": bucket.sustained_mean,
            }
            for bucket in payload.buckets
        ],
    }


def fingerprint_anomaly_for_event(
    store: BaselineStore,
    event: NormalizedEvent,
    *,
    now: datetime | None = None,
) -> float:
    result = store.fingerprint_observation_score(event.service_id, event.fingerprint, 1.0, now=now)
    if result.confidence in ("learning", "insufficient_data"):
        return 0.0
    return round(float(result.score), 6)
