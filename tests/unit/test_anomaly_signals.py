from __future__ import annotations

import json
from datetime import datetime, timedelta, timezone
from pathlib import Path

from analysis.anomaly import (
    anomaly_service_status_to_json,
    build_anomaly_service_status,
    compute_absence_score,
)
from config.models import AnomalyDetectionConfig
from core.enums import EventType, Severity
from events.models import DataQuality, NormalizedEvent, SourceRef
from storage.baseline_store import BaselineStore

FIXTURE = Path(__file__).resolve().parent.parent / "fixtures" / "anomaly" / "events_spec.json"
QUALITY = DataQuality(0.9, 0.9, 0.9, 0.9, 0.9)


def _sev(name: str) -> Severity:
    return Severity[name]


def _load_fixture_events() -> tuple[list[NormalizedEvent], str, datetime]:
    raw = json.loads(FIXTURE.read_text(encoding="utf-8"))
    anchor = datetime.fromisoformat(raw["anchor_iso"])
    if anchor.tzinfo is None:
        anchor = anchor.replace(tzinfo=timezone.utc)
    service_id = str(raw["service_id"])
    events: list[NormalizedEvent] = []
    for index, row in enumerate(raw["events"]):
        ts = anchor + timedelta(minutes=int(row["offset_minutes"]))
        sev = _sev(str(row["severity"]))
        fp = str(row["fingerprint"])
        tags = frozenset(str(tag) for tag in row.get("tags") or [])
        source_ref = SourceRef(source_type="fixture", source_id="anomaly_fixture", raw_offset=index, collected_at=ts)
        events.append(
            NormalizedEvent(
                event_id=f"fx-{index:04d}",
                timestamp=ts,
                timestamp_source="fixture",
                service_id=service_id,
                host_id="h1",
                severity=sev,
                event_type=EventType.LOG,
                message=str(row.get("message") or fp),
                structured_data={},
                tags=tags,
                fingerprint=fp,
                quality=QUALITY,
                source_ref=source_ref,
            )
        )
    now = anchor + timedelta(hours=2)
    return events, service_id, now


def test_anomaly_fixture_scores_are_deterministic_across_runs(tmp_path):
    events, service_id, now = _load_fixture_events()
    cfg = AnomalyDetectionConfig(cold_start_hours=0, min_samples_for_confidence=2)
    store = BaselineStore(tmp_path / "baselines", cold_start_hours=0, min_samples_for_confidence=2)
    build_anomaly_service_status(
        service_id,
        events,
        store,
        config=cfg,
        now=now,
        reconcile=True,
    )
    first = json.dumps(
        anomaly_service_status_to_json(
            build_anomaly_service_status(
                service_id,
                events,
                store,
                config=cfg,
                now=now,
                reconcile=False,
            )
        ),
        sort_keys=True,
    )
    for _ in range(99):
        dumped = json.dumps(
            anomaly_service_status_to_json(
                build_anomaly_service_status(
                    service_id,
                    events,
                    store,
                    config=cfg,
                    now=now,
                    reconcile=False,
                )
            ),
            sort_keys=True,
        )
        assert dumped == first


def test_cold_start_learning_persists_fingerprint_ema(tmp_path):
    events, service_id, now = _load_fixture_events()
    cfg = AnomalyDetectionConfig(cold_start_hours=72, min_samples_for_confidence=2)
    store = BaselineStore(tmp_path / "baselines", cold_start_hours=72, min_samples_for_confidence=2)
    payload = build_anomaly_service_status(
        service_id,
        events,
        store,
        config=cfg,
        now=now,
        reconcile=True,
    )
    assert payload.status == "learning"
    fp_res = store.fingerprint_observation_score(service_id, "fp-stable", 2.0, now=now)
    assert fp_res.confidence == "learning"
    ema, _, updates = store.fingerprint_expected_count(service_id, "fp-stable")
    assert updates >= 1
    assert ema > 0.0


def test_absence_detector_flags_missing_heartbeat(tmp_path):
    rows = []
    for bucket_id in range(12):
        rows.append(
            {
                "bucket_id": bucket_id,
                "fingerprints_present": ["noise"],
                "event_volume": 3.0,
                "error_rate": 0.0,
                "warn_rate": 0.0,
                "new_fingerprint_rate": 0.0,
                "restart_count": 0.0,
            }
        )
    score, missing = compute_absence_score(rows, ("hb-1",), absence_windows=10)
    assert score == 1.0
    assert missing == ("hb-1",)


def test_absence_cleared_when_heartbeat_present():
    rows = []
    for bucket_id in range(10):
        fps = ["noise", "hb-1"] if bucket_id >= 8 else ["noise"]
        rows.append(
            {
                "bucket_id": bucket_id,
                "fingerprints_present": fps,
                "event_volume": 2.0,
                "error_rate": 0.0,
                "warn_rate": 0.0,
                "new_fingerprint_rate": 0.0,
                "restart_count": 0.0,
            }
        )
    score, missing = compute_absence_score(rows, ("hb-1",), absence_windows=10)
    assert score == 0.0
    assert missing == ()


def test_runtime_context_snapshot_builds_without_error():
    from runtime.context import build_runtime_context_snapshot_sync

    snap = build_runtime_context_snapshot_sync(process_limit=5)
    assert snap.hostname
    assert snap.captured_at.tzinfo is not None or snap.captured_at.year >= 2020
