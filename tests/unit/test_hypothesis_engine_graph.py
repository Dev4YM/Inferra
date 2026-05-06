from __future__ import annotations

import json
from datetime import timedelta
from pathlib import Path

import pytest

from config.models import InferraConfig
from core.enums import CauseType
from core.time import utc_now
from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline
from reasoning.engine import HypothesisEngine
from runtime.service_graph import ServiceGraph


def _engine(graph: ServiceGraph) -> HypothesisEngine:
    cfg = InferraConfig()
    cfg.inference_graph.strategies.shared_fate = False
    return HypothesisEngine(graph, cfg)


def _events_from_fixture(path: Path) -> list:
    data = json.loads(path.read_text(encoding="utf-8"))
    pipeline = NormalizationPipeline()
    base = utc_now()
    events = []
    for index, payload in enumerate(data["raw_payloads"]):
        raw = RawEvent(
            source_type="app",
            source_id="fixture",
            raw_payload=payload,
            collected_at=base + timedelta(seconds=index),
            metadata={},
        )
        events.append(pipeline.normalize(raw))
    return data["expected_cause_type"], events


@pytest.mark.parametrize(
    "fixture_name",
    sorted(
        p.name
        for p in (Path(__file__).resolve().parent.parent / "fixtures" / "incidents").glob("*.json")
    ),
)
def test_incident_fixture_covers_expected_cause_type(fixture_name: str) -> None:
    root = Path(__file__).resolve().parent.parent / "fixtures" / "incidents"
    expected, events = _events_from_fixture(root / fixture_name)
    graph = ServiceGraph()
    graph.add_relation("api", "postgres")
    engine = _engine(graph)
    hypotheses = engine.generate("inc-fixture", events)
    types = {h["cause_type"] for h in hypotheses}
    assert expected in types, f"{fixture_name}: got {types}"


def test_hypothesis_ranking_is_identical_across_runs() -> None:
    graph = ServiceGraph()
    graph.add_relation("api", "postgres")
    pipeline = NormalizationPipeline()
    base = utc_now()
    payloads = [
        '{"service":"postgres","level":"error","message":"connection refused on database"}',
        '{"service":"api","level":"error","message":"timeout calling postgres"}',
    ]
    events = []
    for index, pl in enumerate(payloads):
        events.append(
            pipeline.normalize(
                RawEvent(
                    source_type="app",
                    source_id="t",
                    raw_payload=pl,
                    collected_at=base + timedelta(seconds=index),
                    metadata={},
                )
            )
        )
    engine = _engine(graph)
    first = [(h["hypothesis_id"], h["cause_type"], h["rank"]) for h in engine.generate("inc-d", events)]
    for _ in range(99):
        nxt = [(h["hypothesis_id"], h["cause_type"], h["rank"]) for h in engine.generate("inc-d", events)]
        assert nxt == first

    assert first[0][1] == CauseType.DEPENDENCY_FAILURE.value
