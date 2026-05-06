from __future__ import annotations

import json
import math
import os
from datetime import UTC, datetime, timedelta
from pathlib import Path
from time import perf_counter

import pytest

from analysis.models import CorrelationEdge, EventCluster
from core.enums import IncidentState, Severity
from core.models import Incident
from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline
from reasoning.scoring import compute_score_breakdown
from runtime.service_graph import ServiceGraph


def _p99_ms(samples: list[float]) -> float:
    if not samples:
        return 0.0
    s = sorted(samples)
    idx = min(len(s) - 1, max(0, int(math.ceil(0.99 * len(s))) - 1))
    return s[idx] * 1000.0


def _write_merged_report(path: Path, key: str, section: dict[str, object], overall: bool) -> None:
    data: dict[str, object] = {}
    if path.is_file():
        data = json.loads(path.read_text(encoding="utf-8"))
    data[key] = section
    if "overall_passed" in data:
        data["overall_passed"] = bool(data["overall_passed"]) and overall
    else:
        data["overall_passed"] = overall
    path.write_text(json.dumps(data, indent=2, sort_keys=True), encoding="utf-8")


def _report_path() -> Path:
    raw = os.environ.get("PERF_REPORT_PATH", "").strip()
    if raw:
        p = Path(raw)
    else:
        p = Path.cwd() / "perf_report.json"
    p.parent.mkdir(parents=True, exist_ok=True)
    return p


@pytest.mark.perf
def test_performance_budgets_emit_report(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    report_path = _report_path()
    if report_path.is_file():
        report_path.unlink()

    monkeypatch.setattr("normalization.pipeline.new_id", lambda prefix: f"{prefix}-perf")
    log_line = (
        '{"timestamp":"2026-05-02T10:00:00Z","service":"api","level":"error",'
        '"message":"connection refused to 10.0.0.5:5432","request_id":"r1"}'
    )
    pipeline = NormalizationPipeline()
    norm_samples: list[float] = []
    for index in range(400):
        raw = RawEvent(
            source_type="file",
            source_id=f"file://{index}",
            raw_payload=log_line,
            collected_at=datetime(2026, 5, 4, 12, 0, tzinfo=UTC),
            metadata={"path": "/tmp/a.log"},
        )
        t0 = perf_counter()
        pipeline.normalize(raw)
        norm_samples.append(perf_counter() - t0)
    norm_p99 = _p99_ms(norm_samples)
    norm_ok = norm_p99 <= 2.0
    _write_merged_report(
        report_path,
        "normalization",
        {"budget_ms": 2.0, "metric": "normalize_event_p99_ms", "p99_ms": round(norm_p99, 4), "passed": norm_ok},
        norm_ok,
    )

    base = datetime(2026, 5, 4, 12, 0, tzinfo=UTC)
    events_500 = []
    for index in range(500):
        payload = json.dumps(
            {"service": "api", "level": "error", "message": f"timeout from downstream {index % 40}"}
        )
        events_500.append(
            pipeline.normalize(
                RawEvent(
                    source_type="app",
                    source_id="perf",
                    raw_payload=payload,
                    collected_at=base.replace(microsecond=index % 999999),
                    metadata={},
                )
            )
        )

    from analysis.anomaly import aggregate_events_into_bucket_rows

    t0 = perf_counter()
    rows, _ = aggregate_events_into_bucket_rows(events_500, interval_minutes=5, now=base)
    del rows
    analysis_ms = (perf_counter() - t0) * 1000.0
    analysis_ok = analysis_ms <= 50.0
    _write_merged_report(
        report_path,
        "analysis_anomaly",
        {
            "budget_ms": 50.0,
            "elapsed_ms": round(analysis_ms, 4),
            "event_count": len(events_500),
            "metric": "aggregate_events_into_bucket_rows_500_events_ms",
            "passed": analysis_ok,
        },
        analysis_ok,
    )

    ev_a = pipeline.normalize(
        RawEvent(
            source_type="app",
            source_id="s",
            raw_payload='{"service":"api","level":"error","message":"root"}',
            collected_at=base,
            metadata={},
        )
    )
    ev_b = pipeline.normalize(
        RawEvent(
            source_type="app",
            source_id="s",
            raw_payload='{"service":"worker","level":"error","message":"follow"}',
            collected_at=base + timedelta(seconds=1),
            metadata={},
        )
    )
    events_by_id = {ev_a.event_id: ev_a, ev_b.event_id: ev_b}
    cluster = EventCluster(
        cluster_id="c1",
        events=[ev_a.event_id, ev_b.event_id],
        time_range=(ev_a.timestamp, ev_b.timestamp),
        affected_services={"api", "worker"},
        primary_severity=Severity.ERROR,
        trigger_event_id=ev_a.event_id,
        correlation_edges=[
            CorrelationEdge(
                source_event_id=ev_a.event_id,
                target_event_id=ev_b.event_id,
                edge_type="temporal",
                weight=0.9,
                evidence="x",
                reason_codes=("t",),
            )
        ],
        anomaly_scores={},
    )
    hypothesis = {
        "hypothesis_id": "h1",
        "root_cause_event_id": ev_a.event_id,
        "supporting_events": [ev_a.event_id, ev_b.event_id],
        "affected_services": ["api", "worker"],
    }
    graph = ServiceGraph()
    graph.add_relation("api", "worker")
    incident = Incident(
        incident_id="inc1",
        state=IncidentState.INVESTIGATING,
        created_at=base,
        updated_at=base + timedelta(seconds=2),
        clusters=[],
        events=[ev_a.event_id, ev_b.event_id],
        affected_services={"api", "worker"},
        primary_service="api",
        time_range=(base, base + timedelta(seconds=2)),
        severity=Severity.ERROR,
    )
    score_times: list[float] = []
    for _ in range(200):
        t0 = perf_counter()
        compute_score_breakdown(
            hypothesis,
            events_by_id,
            cluster=cluster,
            incident=incident,
            incident_event_ids=list(incident.events),
            service_graph=graph,
            anomaly_by_service={},
            anomaly_event_scores={},
        )
        score_times.append(perf_counter() - t0)
    score_p99 = _p99_ms(score_times)
    score_ok = score_p99 <= 5.0
    _write_merged_report(
        report_path,
        "scoring",
        {
            "budget_ms": 5.0,
            "iterations": len(score_times),
            "metric": "compute_score_breakdown_p99_ms",
            "p99_ms": round(score_p99, 4),
            "passed": score_ok,
        },
        score_ok,
    )

    body = json.loads(report_path.read_text(encoding="utf-8"))
    assert body.get("overall_passed") is True
    assert norm_ok and analysis_ok and score_ok
