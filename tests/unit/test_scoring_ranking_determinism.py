from __future__ import annotations

from dataclasses import fields
from datetime import timedelta

import pytest

from config.models import CalibrationDefaultsConfig, InferraConfig, ScoringTuningConfig
from core.enums import CauseType
from core.models import CalibrationModel, IncidentFeedback, ScoredHypothesis, ScoreBreakdown, WeightState
from core.time import utc_now
from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline
from reasoning.engine import HypothesisEngine
from runtime.service_graph import ServiceGraph
from storage.calibration_store import assign_confidence_label, label_from_score_thresholds
from storage.weight_store import DEFAULT_WEIGHTS, update_weights


def _events_for_scenario(seed: int) -> list:
    pipeline = NormalizationPipeline()
    base = utc_now()
    payloads = [
        f'{{"service":"svc-a","level":"error","message":"failure {seed} connection refused"}}',
        f'{{"service":"svc-b","level":"warn","message":"degraded after {seed}"}}',
    ]
    out = []
    for index, pl in enumerate(payloads):
        out.append(
            pipeline.normalize(
                RawEvent(
                    source_type="app",
                    source_id=f"scn-{seed}",
                    raw_payload=pl,
                    collected_at=base + timedelta(seconds=index, microseconds=seed),
                    metadata={},
                )
            )
        )
    return out


@pytest.mark.parametrize("scenario_index", range(50))
def test_hypothesis_ranking_deterministic_per_scenario(scenario_index: int) -> None:
    graph = ServiceGraph()
    graph.add_relation("svc-a", "svc-b")
    cfg = InferraConfig()
    cfg.inference_graph.strategies.shared_fate = False
    engine = HypothesisEngine(graph, cfg)
    events = _events_for_scenario(scenario_index)
    first = [(h["hypothesis_id"], h["rank"], h["total_score"]) for h in engine.generate(f"inc-s{scenario_index}", events)]
    for _ in range(5):
        nxt = [(h["hypothesis_id"], h["rank"], h["total_score"]) for h in engine.generate(f"inc-s{scenario_index}", events)]
        assert nxt == first
    for item in engine.generate(f"inc-s{scenario_index}", events):
        keys = set((item.get("score_breakdown") or {}).keys())
        assert keys == {f.name for f in fields(ScoreBreakdown)}


def test_feedback_updates_respect_weight_bounds() -> None:
    tuning = ScoringTuningConfig(learning_rate=0.2, max_drift_from_default=0.5, min_weight=0.03, enabled=True)
    state = WeightState(weights=dict(DEFAULT_WEIGHTS), default_weights=dict(DEFAULT_WEIGHTS))
    wrong = _scored("w", 1, 0.9, 0.1, 0.1, 0.1, 0.1, 0.1, 0.7)
    right = _scored("r", 2, 0.1, 0.9, 0.9, 0.9, 0.9, 0.9, 0.7)
    for index in range(200):
        fb = IncidentFeedback(
            incident_id=f"inc-{index}",
            resolved_at=utc_now(),
            correct_hypothesis_id="r",
            feedback_type="confirmed",
        )
        update_weights(state, fb, [wrong, right], tuning=tuning)
        total = sum(state.weights.values())
        assert abs(total - 1.0) < 1e-6
        for key, weight in state.weights.items():
            default = state.default_weights[key]
            lower = max(tuning.min_weight, default * (1.0 - tuning.max_drift_from_default))
            upper = default * (1.0 + tuning.max_drift_from_default)
            assert lower - 1e-9 <= weight <= upper + 1e-9


def test_calibration_score_threshold_labels() -> None:
    defaults = CalibrationDefaultsConfig(high_threshold=0.75, medium_threshold=0.4)
    empty = CalibrationModel()
    assert assign_confidence_label(1.0, empty, defaults, min_samples=1000, staleness_days=365) == "high"
    assert assign_confidence_label(0.1, empty, defaults, min_samples=1000, staleness_days=365) == "low"
    assert label_from_score_thresholds(0.5, defaults) == "medium"


def _scored(
    hid: str,
    rank: int,
    ta: float,
    cs: float,
    fw: float,
    dp: float,
    ec: float,
    an: float,
    total: float,
) -> ScoredHypothesis:
    return ScoredHypothesis(
        hypothesis_id=hid,
        rank=rank,
        cause_type=CauseType.UNKNOWN,
        description="x",
        total_score=total,
        score_breakdown=ScoreBreakdown(ta, cs, fw, dp, ec, an),
        supporting_events=["e1"],
        contradicting_events=[],
        affected_services=["a"],
        suggested_checks=[],
        confidence_label="low",
        is_valid=True,
        invalidation_reasons=[],
    )
