from __future__ import annotations

from datetime import UTC, datetime

from core.enums import EventType, Severity
from events.models import DataQuality, NormalizedEvent, SourceRef
from explanation.guardrails import verify_service_names


def test_guardrails_flag_invented_service_name() -> None:
    text = "The failure propagated to contrived-svc-999 before stabilizing."
    violations = verify_service_names(text, ["api", "postgres"])
    assert any("contrived-svc-999" in item for item in violations)


def test_guardrails_allow_known_services() -> None:
    text = "postgres logged errors while api retried."
    violations = verify_service_names(text, ["api", "postgres"])
    assert violations == []


def _event(event_id: str, timestamp: datetime, service_id: str) -> NormalizedEvent:
    return NormalizedEvent(
        event_id=event_id,
        timestamp=timestamp,
        timestamp_source="parsed",
        service_id=service_id,
        host_id="host-a",
        severity=Severity.ERROR,
        event_type=EventType.LOG,
        message="m",
        structured_data={},
        tags=frozenset(),
        fingerprint=f"fp-{event_id}",
        quality=DataQuality(1.0, 1.0, 1.0, 1.0, 1.0),
        source_ref=SourceRef(
            source_type="app",
            source_id="app://t",
            raw_offset=None,
            collected_at=timestamp,
        ),
    )


def test_verify_timestamps_flags_out_of_range_iso() -> None:
    from explanation.guardrails import verify_timestamps

    incident = {
        "time_range_start": "2026-05-04T12:00:00+00:00",
        "time_range_end": "2026-05-04T12:05:00+00:00",
    }
    events = [_event("e1", datetime(2026, 5, 4, 12, 1, 0, tzinfo=UTC), "api")]
    violations = verify_timestamps("At 2020-01-01T00:00:00Z nothing happened.", incident, events)
    assert any("timestamp_after_range" in v or "timestamp_before_range" in v for v in violations)


def test_check_overconfidence_detects_banned_words() -> None:
    from explanation.guardrails import check_overconfidence

    violations = check_overconfidence("This is definitely the root cause and always happens.")
    assert violations
