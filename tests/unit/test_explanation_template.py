from __future__ import annotations

from datetime import UTC, datetime

from core.enums import EventType, Severity
from events.models import DataQuality, NormalizedEvent, SourceRef
from explanation.template import TemplateExplanationEngine


def _event(
    event_id: str,
    *,
    timestamp: datetime,
    service_id: str = "api",
    message: str = "timeout",
) -> NormalizedEvent:
    return NormalizedEvent(
        event_id=event_id,
        timestamp=timestamp,
        timestamp_source="parsed",
        service_id=service_id,
        host_id="host-a",
        severity=Severity.ERROR,
        event_type=EventType.LOG,
        message=message,
        structured_data={},
        tags=frozenset(),
        fingerprint=f"fp-{event_id}",
        quality=DataQuality(1.0, 1.0, 1.0, 1.0, 1.0),
        source_ref=SourceRef(
            source_type="app",
            source_id="app://test",
            raw_offset=None,
            collected_at=timestamp,
        ),
    )


def test_template_explanation_is_deterministic_for_fixture(tmp_path):
    del tmp_path
    ts = datetime(2026, 5, 4, 12, 0, 0, tzinfo=UTC)
    incident = {
        "incident_id": "inc-fix",
        "affected_services": ["api", "postgres"],
        "time_range_start": "2026-05-04T11:55:00+00:00",
        "time_range_end": "2026-05-04T12:10:00+00:00",
    }
    hypotheses = [
        {
            "hypothesis_id": "h1",
            "rank": 1,
            "cause_type": "dependency_failure",
            "description": "postgres became unreachable and api calls failed",
            "total_score": 0.91,
            "supporting_events": ["e1"],
            "contradicting_events": [],
            "affected_services": ["api", "postgres"],
            "suggested_checks": ["Verify postgres health"],
            "confidence_label": "high",
            "is_valid": True,
            "invalidation_reasons": [],
        }
    ]
    events = [
        _event("e1", timestamp=ts, message="connection refused"),
        _event("e2", timestamp=ts.replace(minute=1), service_id="postgres", message="accept failed"),
    ]
    engine = TemplateExplanationEngine()
    first = engine.generate(incident, hypotheses, events)
    second = engine.generate(incident, hypotheses, events)
    assert first == second
    assert first.generation_model == "template_fallback"
    assert first.hypotheses_hash == second.hypotheses_hash
    assert first.events_hash_head == second.events_hash_head
    assert first.explanation_id == second.explanation_id
