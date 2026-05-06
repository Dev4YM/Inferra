from analysis.anomaly import AnomalyScorer
from analysis.correlation import CorrelationEngine
from core.enums import CauseType
from core.time import utc_now
from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline
from reasoning import SimpleHypothesisEngine


def _event(payload: str):
    return NormalizationPipeline().normalize(
        RawEvent(source_type="app", source_id="test", raw_payload=payload, collected_at=utc_now(), metadata={})
    )


def test_anomaly_scorer_detects_resource_pressure_metrics():
    event = _event(
        '{"service":"host","level":"warn","message":"host resource pressure detected: cpu",'
        '"metrics":{"cpu_percent":96,"memory_percent":70}}'
    )

    expected = round(0.12 * 0.45 + 0.75 * 0.95, 4)
    assert AnomalyScorer().event_score(event) == expected


def test_correlation_clusters_include_service_anomaly_scores():
    first = _event(
        '{"service":"host","level":"warn","message":"host resource pressure detected: cpu",'
        '"metrics":{"cpu_percent":96}}'
    )
    second = _event(
        '{"service":"api","level":"error","message":"resource pressure caused timeout calling postgres"}'
    )

    clusters = CorrelationEngine().build_clusters([first, second])

    assert len(clusters) == 1
    assert clusters[0].anomaly_scores["host"] == round(0.12 * 0.45 + 0.75 * 0.95, 4)
    assert clusters[0].anomaly_scores["api"] >= 0.65


def test_hypothesis_engine_promotes_resource_exhaustion_from_metrics():
    first = _event(
        '{"service":"host","level":"warn","message":"host resource pressure detected: memory",'
        '"metrics":{"memory_percent":96}}'
    )
    second = _event(
        '{"service":"api","level":"warn","message":"high memory pressure near timeout"}'
    )

    hypotheses = SimpleHypothesisEngine().generate("inc-test", [first, second])

    assert hypotheses[0]["cause_type"] == CauseType.RESOURCE_EXHAUSTION.value
    assert hypotheses[0]["confidence_label"] in ("high", "medium")
    assert hypotheses[0]["score_breakdown"]["anomaly_severity"] >= 0.76
