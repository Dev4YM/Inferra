from __future__ import annotations

import sqlite3
from datetime import UTC, datetime, timedelta
from types import SimpleNamespace

from core.enums import CauseType, EventType, IncidentState, InferenceEdgeType, Severity
from core.models import (
    CalibrationModel,
    ExplanationResult,
    Incident,
    IncidentFeedback,
    InferenceEdge,
    InferenceGraph,
    InferenceNode,
    ResolutionInfo,
    ScoredHypothesis,
    ScoreBreakdown,
    WeightState,
)
from events.models import DataQuality, NormalizedEvent, SourceRef
from storage import (
    DEFAULT_WEIGHTS,
    JsonBaselineStore,
    JsonCalibrationStore,
    JsonServiceGraphStore,
    JsonWeightStore,
    MetricRingbuffer,
    SqliteEventStore,
    SqliteIncidentStore,
    assign_confidence_label,
    check_calibration_staleness,
    reset_weights,
    update_calibration,
    update_weights,
)


def _event(
    event_id: str,
    timestamp: datetime,
    *,
    service_id: str = "api",
    severity: Severity = Severity.ERROR,
    fingerprint: str | None = None,
    tags: frozenset[str] | None = None,
) -> NormalizedEvent:
    return NormalizedEvent(
        event_id=event_id,
        timestamp=timestamp,
        timestamp_source="parsed",
        service_id=service_id,
        host_id="host-a",
        severity=severity,
        event_type=EventType.LOG,
        message=f"message {event_id}",
        structured_data={},
        tags=tags or frozenset({"timeout"}),
        fingerprint=fingerprint or f"fp-{event_id}",
        quality=DataQuality(1.0, 1.0, 1.0, 1.0, 1.0),
        source_ref=SourceRef(
            source_type="app",
            source_id="app://test",
            raw_offset=None,
            collected_at=timestamp,
        ),
    )


def _hypothesis(hypothesis_id: str, rank: int, score: float, dependency: float) -> ScoredHypothesis:
    return ScoredHypothesis(
        hypothesis_id=hypothesis_id,
        rank=rank,
        cause_type=CauseType.DEPENDENCY_FAILURE,
        description=f"Hypothesis {hypothesis_id}",
        total_score=score,
        score_breakdown=ScoreBreakdown(
            temporal_alignment=0.7 if rank == 1 else 0.4,
            correlation_strength=0.8 if rank == 1 else 0.3,
            frequency_weight=0.5,
            dependency_proximity=dependency,
            evidence_coverage=0.6 if rank == 1 else 0.2,
            anomaly_severity=score,
        ),
        supporting_events=["e1", "e2"],
        contradicting_events=[],
        affected_services=["api", "db"],
        suggested_checks=["check logs"],
        confidence_label="low",
        is_valid=True,
        invalidation_reasons=[],
    )


def test_sqlite_event_store_acceptance(tmp_path):
    path = tmp_path / "events.db"
    store = SqliteEventStore(path, batch_size=100, start_pruner=False)
    try:
        now = datetime.now(tz=UTC) - timedelta(minutes=50)
        events = [
            _event(
                f"evt-{index}",
                now + timedelta(seconds=index),
                service_id="api" if index < 60 else "worker",
                severity=Severity.CRITICAL if index % 25 == 0 else Severity.ERROR if index % 2 == 0 else Severity.WARN,
            )
            for index in range(100)
        ]
        assert store.insert_batch(events) == 100

        queried = list(store.query_time_range(events[0].timestamp, events[-1].timestamp))
        assert [item.event_id for item in queried] == [item.event_id for item in sorted(events, key=lambda item: item.timestamp)]

        service_events = list(store.query_by_service("api", timedelta(hours=1)))
        assert {item.service_id for item in service_events} == {"api"}
        assert len(service_events) == 60

        expected_errorish = sum(1 for item in events[:60] if item.severity >= Severity.ERROR)
        assert store.count_by_severity("api", Severity.ERROR, timedelta(hours=1)) == expected_errorish
        assert store.fingerprint_exists(events[0].fingerprint) is True
        assert store.fingerprint_exists("missing-fingerprint") is False

        with sqlite3.connect(path) as conn:
            conn.execute("UPDATE events SET inserted_at = ? WHERE event_id IN (?, ?, ?, ?, ?)", (
                "2020-01-01T00:00:00.000000Z",
                "evt-0",
                "evt-1",
                "evt-2",
                "evt-3",
                "evt-4",
            ))
            conn.commit()
        assert store.prune_expired(72) == 5
        assert store.count_events() == 95

        with sqlite3.connect(path) as conn:
            mode = conn.execute("PRAGMA journal_mode").fetchone()[0]
        assert str(mode).lower() == "wal"
    finally:
        store.close()


def test_sqlite_incident_store_roundtrip_and_archive(tmp_path):
    path = tmp_path / "incidents.db"
    store = SqliteIncidentStore(path)
    try:
        created_at = datetime(2026, 5, 4, 12, 0, tzinfo=UTC)
        graph = InferenceGraph(
            nodes={
                "e1": InferenceNode(
                    event_id="e1",
                    service_id="api",
                    timestamp=created_at,
                    severity=Severity.ERROR,
                    summary="db timeout",
                    node_type="root",
                )
            },
            edges=[
                InferenceEdge(
                    source_event_id="e1",
                    target_event_id="e2",
                    edge_type=InferenceEdgeType.DEPENDENCY_PROPAGATION,
                    plausibility=0.9,
                    latency_ms=150.0,
                    evidence="api failed before worker",
                )
            ],
            root_candidates=["e1"],
            leaf_nodes=["e2"],
        )
        incident = Incident(
            incident_id="inc-1",
            state=IncidentState.INVESTIGATING,
            created_at=created_at,
            updated_at=created_at,
            clusters=["clu-1"],
            events=["e1", "e2"],
            affected_services={"api", "db"},
            primary_service="api",
            time_range=(created_at, created_at + timedelta(minutes=5)),
            severity=Severity.ERROR,
            inference_graph=graph,
        )

        assert store.get_incident("missing") is None
        assert store.list_incidents() == []

        created = store.create_incident(incident)
        assert created.incident_id == incident.incident_id
        assert created.affected_services == {"api", "db"}

        store.add_events_to_incident("inc-1", ["e3"])
        reloaded = store.get_incident("inc-1")
        assert reloaded is not None
        assert reloaded.events == ["e1", "e2", "e3"]

        hypotheses = [_hypothesis("h-top", 1, 0.82, 0.9), _hypothesis("h-alt", 2, 0.35, 0.2)]
        store.add_hypotheses("inc-1", hypotheses)
        loaded_hypotheses = store.get_hypotheses("inc-1")
        assert [item.hypothesis_id for item in loaded_hypotheses] == ["h-top", "h-alt"]
        assert loaded_hypotheses[0].score_breakdown.dependency_proximity == 0.9

        explanation = ExplanationResult(
            incident_id="inc-1",
            summary="API errors were triggered by db failures.",
            primary_hypothesis_text="Database dependency failed first.",
            evidence_narrative="Timeouts started on the database before the API error burst.",
            timeline_narrative="Database warnings appeared before API failures.",
            alternative_explanations=["Transient network issue"],
            suggested_actions=["Check database connectivity"],
            uncertainty_notes=["Limited container metrics"],
            generation_model="template_fallback",
            guardrail_violations=[],
            hypotheses_hash="abc",
            events_hash_head="def",
        )
        store.add_explanation(explanation)
        latest = store.get_latest_explanation("inc-1")
        assert latest is not None
        assert latest.summary == explanation.summary
        cached = store.get_cached_explanation("inc-1", "abc", "def")
        assert cached is not None
        assert cached.hypotheses_hash == "abc"

        loaded_graph = store.get_inference_graph("inc-1")
        assert loaded_graph is not None
        assert loaded_graph.root_candidates == ["e1"]
        assert loaded_graph.edges[0].edge_type == InferenceEdgeType.DEPENDENCY_PROPAGATION

        store.resolve_incident(
            "inc-1",
            ResolutionInfo(
                resolved_by="operator",
                correct_hypothesis_id="h-top",
                feedback_type="confirmed",
                notes="Verified by logs",
                resolved_at=created_at + timedelta(days=10),
            ),
        )
        resolved = store.get_incident("inc-1")
        assert resolved is not None
        assert resolved.state == IncidentState.RESOLVED
        assert store.get_inference_graph("inc-1") is None

        with sqlite3.connect(path) as conn:
            conn.execute(
                "UPDATE incidents SET updated_at = ? WHERE incident_id = ?",
                ("2020-01-01T00:00:00.000000Z", "inc-1"),
            )
            conn.commit()
        assert store.archive_old_incidents(archive_after_days=7) == 1
        assert store.get_incident("inc-1") is None
        assert list((tmp_path / "archive").glob("incidents_*.db"))
    finally:
        store.close()


def test_metric_ringbuffer_wraparound_and_persistence(tmp_path):
    ring = MetricRingbuffer(service_id="api", metric_name="error_rate", capacity=3)
    base = datetime(2026, 5, 4, 10, 0, tzinfo=UTC)
    ring.append(base, 5.0)
    ring.append(base + timedelta(minutes=5), 10.0)
    ring.append(base + timedelta(minutes=10), 15.0)
    ring.append(base + timedelta(minutes=15), 20.0)

    assert ring.last_n(2) == [
        (base + timedelta(minutes=10), 15.0),
        (base + timedelta(minutes=15), 20.0),
    ]
    assert ring.query_range(base + timedelta(minutes=6), base + timedelta(minutes=15)) == [
        (base + timedelta(minutes=10), 15.0),
        (base + timedelta(minutes=15), 20.0),
    ]
    assert ring.mean_per_minute() == (10.0 + 15.0 + 20.0) / 3.0 / 5.0
    assert ring.coefficient_of_variation() > 0.0

    saved_path = ring.save_to_json(tmp_path / "metrics" / "api_error_rate.json")
    loaded = MetricRingbuffer.load_from_json(saved_path)
    assert loaded.last_n(3) == ring.last_n(3)


def test_baseline_store_ema_and_anomaly_score(tmp_path):
    store = JsonBaselineStore(tmp_path / "baselines", cold_start_hours=0, min_samples_for_confidence=2)
    store.update_baseline("api", "event_volume", 3, 10.0, alpha=0.1)
    store.update_baseline("api", "event_volume", 3, 20.0, alpha=0.1)
    baseline = store.get_baseline("api", "event_volume")

    assert baseline.buckets[3] == 11.0
    assert baseline.stddev[3] == 1.0
    assert baseline.sample_counts[3] == 2

    anomaly = store.compute_anomaly_score(20.0, baseline, 3)
    assert anomaly.confidence == "low"
    assert anomaly.expected == 11.0
    assert anomaly.score > 0.8

    cloned = JsonBaselineStore(tmp_path / "baselines", cold_start_hours=0, min_samples_for_confidence=2)
    assert cloned.get_baseline("api", "event_volume").buckets[3] == 11.0


def test_service_graph_cache_persist_and_load(tmp_path):
    config = SimpleNamespace(
        topology=SimpleNamespace(edges=[SimpleNamespace(source="api", target="db", type="depends_on")])
    )
    path = tmp_path / "service_graph.json"
    graph = JsonServiceGraphStore(path, config=config, discover_docker=False, auto_persist=False)
    graph.add_relation("api", "cache", "calls", origin="config", confidence="high")
    graph.add_relation("api", "worker", "colocated_with", origin="config", confidence="medium")
    graph.persist()

    loaded = JsonServiceGraphStore(path, config=config, discover_docker=False, auto_persist=False)
    assert loaded.get_dependencies("api") == ["cache", "db"]
    assert loaded.get_dependents("db") == ["api"]
    assert loaded.get_colocated("api") == ["worker"]
    assert loaded.shortest_path("db", "cache") == ["db", "api", "cache"]
    assert loaded.shortest_path_length("db", "cache") == 2
    assert set(loaded.subgraph_around("api", depth=1).nodes()) == {"api", "db", "cache", "worker"}


def test_calibration_store_updates_and_roundtrip(tmp_path):
    model = CalibrationModel()
    hypotheses = [_hypothesis("correct", 1, 0.85, 0.8), _hypothesis("other", 2, 0.35, 0.2)]
    for index in range(20):
        update_calibration(
            model,
            IncidentFeedback(
                incident_id=f"inc-{index}",
                resolved_at=datetime.now(tz=UTC),
                correct_hypothesis_id="correct",
                feedback_type="confirmed",
            ),
            hypotheses,
        )
    assert assign_confidence_label(0.85, model) == "high"
    assert check_calibration_staleness(model) == "current"

    store = JsonCalibrationStore(tmp_path / "calibration.json")
    store.save(model)
    loaded = store.load()
    assert loaded.total_feedback_count == 20
    assert assign_confidence_label(0.85, loaded) == "high"


def test_weight_store_update_reset_and_history(tmp_path):
    state = WeightState(weights=dict(DEFAULT_WEIGHTS), default_weights=dict(DEFAULT_WEIGHTS))
    wrong_top = _hypothesis("wrong", 1, 0.7, 0.9)
    correct = _hypothesis("correct", 2, 0.6, 0.1)
    update_weights(
        state,
        IncidentFeedback(
            incident_id="inc-weights",
            resolved_at=datetime.now(tz=UTC),
            correct_hypothesis_id="correct",
            feedback_type="confirmed",
        ),
        [wrong_top, correct],
    )
    assert abs(sum(state.weights.values()) - 1.0) < 1e-9
    assert state.history

    store = JsonWeightStore(tmp_path / "scoring_weights.json", tmp_path / "weight_history.jsonl")
    store.save(state)
    loaded = store.load()
    assert loaded.update_count == state.update_count
    assert loaded.history
    reset_weights(state)
    assert state.weights == dict(DEFAULT_WEIGHTS)
    assert (tmp_path / "weight_history.jsonl").read_text(encoding="utf-8").strip()
