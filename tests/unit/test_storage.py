from datetime import timedelta

from core.time import utc_now
from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline
from storage import initialize_storage


def test_sqlite_event_store_roundtrip(tmp_path):
    event_store, incident_store = initialize_storage(tmp_path)
    try:
        raw = RawEvent(
            source_type="app",
            source_id="app://test",
            raw_payload='{"service":"api","level":"warn","message":"timeout calling postgres"}',
            collected_at=utc_now(),
            metadata={},
        )
        event = NormalizationPipeline().normalize(raw)
        event_store.add_event(event)

        stored = event_store.get_event(event.event_id)
        assert stored is not None
        assert stored.event_id == event.event_id
        assert stored.service_id == "api"

        recent = list(event_store.query_by_service("api", timedelta(minutes=1)))
        assert [item.event_id for item in recent] == [event.event_id]
    finally:
        event_store.close()
        incident_store.close()


def test_sqlite_collector_state_roundtrip(tmp_path):
    event_store, incident_store = initialize_storage(tmp_path)
    try:
        event_store.set_collector_state("collector://one", "bookmark", "42")

        assert event_store.get_collector_state("collector://one", "bookmark") == "42"
        assert event_store.get_collector_state("collector://missing", "bookmark") is None
    finally:
        event_store.close()
        incident_store.close()
