from __future__ import annotations

from datetime import timedelta

import pytest

from analysis.lifecycle import IncidentLifecycleManager
from config.models import AnomalyDetectionConfig, InferraConfig, StorageConfig
from core.enums import IncidentState
from core.time import utc_now
from events.models import NormalizedEvent, RawEvent
from normalization.pipeline import NormalizationPipeline
from runtime.service_graph import ServiceGraph
from storage import initialize_storage


def _norm(pipeline: NormalizationPipeline, payload: str, collected_at) -> NormalizedEvent:
    return pipeline.normalize(
        RawEvent(
            source_type="test",
            source_id="unit",
            raw_payload=payload,
            collected_at=collected_at,
            metadata={},
        )
    )


@pytest.fixture
def pipeline() -> NormalizationPipeline:
    return NormalizationPipeline()


def test_cascade_single_incident_postgres_primary_with_topology(tmp_path, pipeline: NormalizationPipeline) -> None:
    t0 = utc_now() - timedelta(seconds=60)
    graph = ServiceGraph()
    graph.add_relation("api-gateway", "user-service")
    graph.add_relation("user-service", "postgres")
    event_store, incident_store, *_ = initialize_storage(tmp_path, start_pruner=False)
    cfg = InferraConfig(
        storage=StorageConfig(data_dir=tmp_path),
        anomaly_detection=AnomalyDetectionConfig(enabled=False),
    )
    lifecycle = IncidentLifecycleManager(
        event_store,
        incident_store,
        graph,
        config=cfg,
        baseline_store=None,
        anomaly_detection=None,
    )
    payloads = [
        (
            '{"service":"postgres","level":"error","message":"connection refused p1","tags":["timeout"]}',
            t0,
        ),
        (
            '{"service":"user-service","level":"error","message":"timeout to postgres u1","tags":["timeout"]}',
            t0 + timedelta(seconds=5),
        ),
        (
            '{"service":"api-gateway","level":"error","message":"timeout to user-service a1","tags":["timeout"]}',
            t0 + timedelta(seconds=10),
        ),
    ]
    for body, ts in payloads:
        event_store.add_event(_norm(pipeline, body, ts))
    ids = []
    for _ in range(100):
        lifecycle.analyze_recent(window_seconds=120)
        rows = incident_store.list_incidents(
            state=[IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED],
            limit=50,
        )
        ids.append(sorted(incident.incident_id for incident in rows))
    assert ids[0] == ids[-1]
    assert len(ids[0]) == 1
    incident = incident_store.get_incident(ids[0][0])
    assert incident is not None
    assert incident.primary_service == "postgres"
    assert sorted(incident.affected_services) == ["api-gateway", "postgres", "user-service"]
    log = incident_store.list_state_log(incident.incident_id)
    states = [entry.new_state for entry in log]
    assert "open" in states
    assert "investigating" in states


def test_no_topology_three_service_isolated_incidents(tmp_path, pipeline: NormalizationPipeline) -> None:
    t0 = utc_now() - timedelta(seconds=60)
    graph = ServiceGraph()
    event_store, incident_store, *_ = initialize_storage(tmp_path, start_pruner=False)
    cfg = InferraConfig(
        storage=StorageConfig(data_dir=tmp_path),
        anomaly_detection=AnomalyDetectionConfig(enabled=False),
    )
    lifecycle = IncidentLifecycleManager(
        event_store,
        incident_store,
        graph,
        config=cfg,
        baseline_store=None,
        anomaly_detection=None,
    )
    pairs = [
        ('{"service":"postgres","level":"error","message":"oom p1","tags":["oom"]}', t0),
        ('{"service":"postgres","level":"error","message":"oom p2","tags":["oom"]}', t0 + timedelta(seconds=2)),
        ('{"service":"user-service","level":"error","message":"disk u1","tags":["disk_full"]}', t0 + timedelta(seconds=1)),
        (
            '{"service":"user-service","level":"error","message":"disk u2","tags":["disk_full"]}',
            t0 + timedelta(seconds=3),
        ),
        ('{"service":"api-gateway","level":"error","message":"crash g1","tags":["crash"]}', t0 + timedelta(seconds=1)),
        (
            '{"service":"api-gateway","level":"error","message":"crash g2","tags":["crash"]}',
            t0 + timedelta(seconds=2),
        ),
    ]
    for body, ts in pairs:
        event_store.add_event(_norm(pipeline, body, ts))
    lifecycle.analyze_recent(window_seconds=120)
    rows = incident_store.list_incidents(
        state=[IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED],
        limit=50,
    )
    assert len(rows) == 3
    primaries = {inc.primary_service for inc in rows}
    assert primaries == {"api-gateway", "postgres", "user-service"}


def test_stale_timeout_resolves_incident(tmp_path, pipeline: NormalizationPipeline, monkeypatch: pytest.MonkeyPatch) -> None:
    clock = {"t": utc_now()}
    monkeypatch.setattr(
        "analysis.lifecycle.utc_now",
        lambda: clock["t"],
    )
    monkeypatch.setattr(
        "storage.incident_store.utc_now",
        lambda: clock["t"],
    )
    t0 = clock["t"] - timedelta(seconds=10)
    event_store, incident_store, *_ = initialize_storage(tmp_path, start_pruner=False)
    cfg = InferraConfig(
        storage=StorageConfig(data_dir=tmp_path),
        anomaly_detection=AnomalyDetectionConfig(enabled=False),
    )
    cfg.correlation.analysis_window_seconds = 600
    cfg.incident_lifecycle.stale_timeout_seconds = 30
    lifecycle = IncidentLifecycleManager(
        event_store,
        incident_store,
        ServiceGraph(),
        config=cfg,
        baseline_store=None,
        anomaly_detection=None,
    )
    event_store.add_event(
        _norm(
            pipeline,
            '{"service":"solo","level":"error","message":"failure s1","tags":["timeout"]}',
            t0,
        )
    )
    event_store.add_event(
        _norm(
            pipeline,
            '{"service":"solo","level":"error","message":"failure s2","tags":["timeout"]}',
            t0 + timedelta(seconds=2),
        )
    )
    lifecycle.analyze_recent(window_seconds=600)
    incident = incident_store.list_incidents(
        state=[IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED],
        limit=5,
    )[0]
    clock["t"] = clock["t"] + timedelta(seconds=120)
    lifecycle.analyze_recent(window_seconds=600)
    resolved = incident_store.get_incident(incident.incident_id)
    assert resolved is not None
    assert resolved.state == IncidentState.RESOLVED
    reasons = [entry.reason for entry in incident_store.list_state_log(incident.incident_id)]
    assert any("stale_timeout" in r for r in reasons)
