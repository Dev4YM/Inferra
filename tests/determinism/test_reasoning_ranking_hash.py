from __future__ import annotations

import hashlib
import json
from datetime import UTC, datetime, timedelta
from pathlib import Path

import pytest

from config.models import InferraConfig
from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline
from reasoning.engine import HypothesisEngine
from runtime.service_graph import ServiceGraph

pytestmark = pytest.mark.determinism

_FIXED_BASE = datetime(2026, 5, 4, 12, 0, 0, tzinfo=UTC)


def _rank_fingerprint(events_path: Path | None = None) -> str:
    graph = ServiceGraph()
    graph.add_relation("api", "postgres")
    cfg = InferraConfig()
    cfg.inference_graph.strategies.shared_fate = False
    engine = HypothesisEngine(graph, cfg)
    pipeline = NormalizationPipeline()
    payloads = [
        '{"service":"postgres","level":"error","message":"connection refused on database"}',
        '{"service":"api","level":"error","message":"timeout calling postgres"}',
    ]
    incident_key = "inc-determinism-hash"
    if events_path is not None:
        incident_key = f"inc-{events_path.stem}"
        data = json.loads(events_path.read_text(encoding="utf-8"))
        payloads = list(data["raw_payloads"])
    events = []
    for index, pl in enumerate(payloads):
        events.append(
            pipeline.normalize(
                RawEvent(
                    source_type="app",
                    source_id="determinism-fixture",
                    raw_payload=pl,
                    collected_at=_FIXED_BASE + timedelta(seconds=index),
                    metadata={},
                )
            )
        )
    rows = sorted(
        (
            int(h["rank"]),
            str(h["cause_type"]),
            round(float(h["total_score"]), 6),
        )
        for h in engine.generate(incident_key, events)
    )
    blob = json.dumps(rows, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(blob.encode("utf-8")).hexdigest()


def test_hypothesis_ranking_sha_identical_across_100_runs() -> None:
    expected = _rank_fingerprint()
    for _ in range(99):
        assert _rank_fingerprint() == expected


@pytest.mark.parametrize(
    "fixture_name",
    sorted(p.name for p in (Path(__file__).resolve().parents[1] / "fixtures" / "incidents").glob("*.json")),
)
def test_incident_fixtures_ranking_sha_stable(fixture_name: str) -> None:
    root = Path(__file__).resolve().parents[1] / "fixtures" / "incidents"
    expected = _rank_fingerprint(root / fixture_name)
    for _ in range(2):
        assert _rank_fingerprint(root / fixture_name) == expected
