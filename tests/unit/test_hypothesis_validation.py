from datetime import timedelta

from core.enums import CauseType
from core.time import utc_now
from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline
from reasoning import SimpleHypothesisEngine
from runtime.service_graph import ServiceGraph


def _event(payload: str, offset_seconds: int = 0):
    now = utc_now() + timedelta(seconds=offset_seconds)
    return NormalizationPipeline().normalize(
        RawEvent(source_type="app", source_id="test", raw_payload=payload, collected_at=now, metadata={})
    )


def test_dependency_hypothesis_uses_service_graph_for_root_cause():
    graph = ServiceGraph()
    graph.add_relation("api", "postgres")
    postgres_error = _event('{"service":"postgres","level":"error","message":"connection refused on database"}')
    api_timeout = _event('{"service":"api","level":"error","message":"timeout calling postgres"}', offset_seconds=2)

    hypotheses = SimpleHypothesisEngine(graph).generate("inc-test", [api_timeout, postgres_error])
    top = hypotheses[0]

    assert top["cause_type"] == CauseType.DEPENDENCY_FAILURE.value
    assert top["root_cause_event_id"] == postgres_error.event_id
    assert "likely upstream/root service: postgres" in top["description"]
    assert top["score_breakdown"]["dependency_proximity"] >= 0.99


def test_health_check_pass_penalizes_conflicting_dependency_hypothesis():
    graph = ServiceGraph()
    graph.add_relation("api", "postgres")
    postgres_health = _event(
        '{"service":"postgres","level":"warn","message":"health check passed healthy"}',
        offset_seconds=1,
    )
    api_timeout = _event('{"service":"api","level":"error","message":"timeout calling postgres"}', offset_seconds=1)

    hypotheses = SimpleHypothesisEngine(graph).generate("inc-test", [api_timeout, postgres_health])
    top = hypotheses[0]

    assert postgres_health.event_id in top["contradicting_events"]
    assert any("health check" in reason.lower() for reason in top["invalidation_reasons"])
    assert top["is_valid"] is True


def test_low_resource_metrics_contradict_resource_exhaustion():
    pressure = _event(
        '{"service":"api","level":"warn","message":"resource pressure caused timeout",'
        '"metrics":{"memory_percent":96}}',
        offset_seconds=0,
    )
    low_host_metrics = _event(
        '{"service":"host","level":"info","message":"host metrics snapshot",'
        '"metrics":{"cpu_percent":12,"memory_percent":40,"disk_percent":50}}',
        offset_seconds=0,
    )

    hypotheses = SimpleHypothesisEngine().generate("inc-test", [pressure, low_host_metrics])
    resource = next(item for item in hypotheses if item["cause_type"] == CauseType.RESOURCE_EXHAUSTION.value)

    assert low_host_metrics.event_id in resource["contradicting_events"]
    assert resource["invalidation_reasons"]
