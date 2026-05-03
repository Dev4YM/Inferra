from core.enums import EventType, Severity
from core.time import utc_now
from events.models import RawEvent
from normalization.fingerprint import compute_fingerprint
from normalization.pipeline import NormalizationPipeline


def test_fingerprint_is_stable_for_variable_tokens():
    first = compute_fingerprint("api", "Connection to 10.0.0.5:5432 refused", 3)
    second = compute_fingerprint("api", "Connection to 10.0.0.6:5432 refused", 3)

    assert first == second


def test_pipeline_normalizes_json_event():
    raw = RawEvent(
        source_type="app",
        source_id="app://test",
        raw_payload='{"timestamp":"2026-05-02T10:00:00Z","service":"api","level":"error","message":"connection refused"}',
        collected_at=utc_now(),
        metadata={},
    )

    event = NormalizationPipeline().normalize(raw)

    assert event.service_id == "api"
    assert event.severity == Severity.ERROR
    assert event.event_type == EventType.LOG
    assert "connection_refused" in event.tags
    assert event.timestamp_source == "parsed"
    assert event.quality.overall > 0.7
