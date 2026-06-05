from __future__ import annotations

import sqlite3
from datetime import UTC, datetime

import pytest

from core.enums import EventType, Severity
from events.models import DataQuality, NormalizedEvent, SourceRef
from storage.migrations import EVENTS_MIGRATIONS, migrate
from storage.event_store import SqliteEventStore

pytestmark = pytest.mark.legacy_runtime


def _legacy_event(event_id: str, ts: datetime) -> NormalizedEvent:
    return NormalizedEvent(
        event_id=event_id,
        timestamp=ts,
        timestamp_source="parsed",
        service_id="api",
        host_id="h1",
        severity=Severity.ERROR,
        event_type=EventType.LOG,
        message="upgrade path",
        structured_data={},
        tags=frozenset({"t"}),
        fingerprint=f"fp-{event_id}",
        quality=DataQuality(1.0, 1.0, 1.0, 1.0, 1.0),
        source_ref=SourceRef(
            source_type="app",
            source_id="app://upgrade",
            raw_offset=None,
            collected_at=ts,
        ),
    )


@pytest.mark.integration
def test_events_db_from_v1_schema_stays_readable_after_migrations(tmp_path) -> None:
    db_path = tmp_path / "events.db"
    conn = sqlite3.connect(db_path)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    conn.executescript(EVENTS_MIGRATIONS[0].up_sql)
    conn.execute(
        """
        CREATE TABLE IF NOT EXISTS _schema_version (
            schema_name TEXT PRIMARY KEY,
            version INTEGER NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        )
        """
    )
    conn.execute(
        "INSERT OR REPLACE INTO _schema_version(schema_name, version) VALUES (?, ?)",
        ("events", 1),
    )
    conn.commit()
    conn.close()

    migrate(db_path)
    ts = datetime(2026, 4, 1, 10, 0, 0, tzinfo=UTC)
    store = SqliteEventStore(db_path, start_pruner=False)
    store.add_event(_legacy_event("evt-up-1", ts))
    store.close()

    migrate(db_path)
    reopened = SqliteEventStore(db_path, start_pruner=False)
    try:
        loaded = reopened.get_event("evt-up-1")
        assert loaded is not None
        assert loaded.service_id == "api"
        latest = reopened.latest_events(limit=10)
        assert any(e.event_id == "evt-up-1" for e in latest)
        start = ts.replace(hour=0, minute=0, second=0, microsecond=0)
        end = ts.replace(hour=23, minute=59, second=59, microsecond=999999)
        rows = list(reopened.query_time_range(start, end))
        assert len(rows) >= 1
    finally:
        reopened.close()
