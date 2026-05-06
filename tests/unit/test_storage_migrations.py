from __future__ import annotations

import sqlite3
import threading
from datetime import UTC, datetime, timedelta

import pytest

from core.enums import EventType, IncidentState, Severity
from core.models import Incident, ResolutionInfo
from core.time import to_iso
from events.models import DataQuality, NormalizedEvent, SourceRef
from storage.migrations import (
    CURRENT_SCHEMA_VERSION,
    EVENTS_MIGRATIONS,
    INCIDENTS_MIGRATIONS,
    Migration,
    backup_db,
    integrity_check,
    migrate,
    vacuum_db,
)
from storage import SqliteEventStore, SqliteIncidentStore


def _event(
    event_id: str,
    timestamp: datetime,
    *,
    service_id: str = "api",
    severity: Severity = Severity.ERROR,
) -> NormalizedEvent:
    return NormalizedEvent(
        event_id=event_id,
        timestamp=timestamp,
        timestamp_source="parsed",
        service_id=service_id,
        host_id="host-a",
        severity=severity,
        event_type=EventType.LOG,
        message=f"message {event_id}",
        structured_data={},
        tags=frozenset({"timeout"}),
        fingerprint=f"fp-{event_id}",
        quality=DataQuality(1.0, 1.0, 1.0, 1.0, 1.0),
        source_ref=SourceRef(
            source_type="app",
            source_id="app://test",
            raw_offset=None,
            collected_at=timestamp,
        ),
    )


# ---------------------------------------------------------------------------
# Migration framework tests
# ---------------------------------------------------------------------------


class TestMigrationFramework:
    def test_migration_objects_are_ordered_and_complete(self):
        for name, chain in [("events", EVENTS_MIGRATIONS), ("incidents", INCIDENTS_MIGRATIONS)]:
            versions = [m.version for m in chain]
            assert versions == sorted(versions), f"{name} migrations are not sorted"
            assert versions[-1] == CURRENT_SCHEMA_VERSION, f"{name} chain does not reach CURRENT_SCHEMA_VERSION"
            for m in chain:
                assert isinstance(m, Migration)
                assert m.description

    def test_migrate_events_empty_to_current(self, tmp_path):
        db_path = tmp_path / "events.db"
        version = migrate(db_path)
        assert version == CURRENT_SCHEMA_VERSION

        conn = sqlite3.connect(db_path)
        try:
            tables = {
                row[0]
                for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()
            }
            for expected in ("events", "collector_state", "raw_events", "fingerprint_seen", "dedup_window"):
                assert expected in tables, f"Missing table: {expected}"
        finally:
            conn.close()

    def test_migrate_incidents_empty_to_current(self, tmp_path):
        db_path = tmp_path / "incidents.db"
        version = migrate(db_path)
        assert version == CURRENT_SCHEMA_VERSION

        conn = sqlite3.connect(db_path)
        try:
            tables = {
                row[0]
                for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()
            }
            for expected in (
                "incidents", "incident_events", "incident_clusters",
                "hypotheses", "explanations", "feedback", "incident_state_log",
                "incident_ai_traces", "incident_chat_messages",
            ):
                assert expected in tables, f"Missing table: {expected}"

            columns = {row[1] for row in conn.execute("PRAGMA table_info(incidents)").fetchall()}
            for expected_col in ("cluster_ids", "runtime_context", "resolution_info"):
                assert expected_col in columns, f"Missing column: {expected_col}"

            exp_cols = {row[1] for row in conn.execute("PRAGMA table_info(explanations)").fetchall()}
            for expected_col in ("hypotheses_hash", "events_hash_head", "explanation_schema_version", "quality"):
                assert expected_col in exp_cols, f"Missing column: {expected_col}"
        finally:
            conn.close()

    def test_migrate_is_idempotent(self, tmp_path):
        db_path = tmp_path / "events.db"
        v1 = migrate(db_path)
        v2 = migrate(db_path)
        assert v1 == v2 == CURRENT_SCHEMA_VERSION

    def test_migrate_from_v1_to_current(self, tmp_path):
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

        version = migrate(db_path)
        assert version == CURRENT_SCHEMA_VERSION

        conn = sqlite3.connect(db_path)
        try:
            tables = {
                row[0]
                for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()
            }
            assert "raw_events" in tables
            assert "fingerprint_seen" in tables
            assert "dedup_window" in tables
        finally:
            conn.close()

    def test_migrate_from_v2_fixture(self, tmp_path):
        db_path = tmp_path / "incidents.db"

        conn = sqlite3.connect(db_path)
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("PRAGMA foreign_keys=ON")
        conn.executescript(INCIDENTS_MIGRATIONS[0].up_sql)
        conn.executescript(INCIDENTS_MIGRATIONS[1].up_sql)
        conn.execute(
            "INSERT OR REPLACE INTO _schema_version(schema_name, version) VALUES (?, ?)",
            ("incidents", 2),
        )
        conn.commit()
        conn.close()

        version = migrate(db_path)
        assert version == CURRENT_SCHEMA_VERSION

        conn = sqlite3.connect(db_path)
        try:
            tables = {
                row[0]
                for row in conn.execute("SELECT name FROM sqlite_master WHERE type='table'").fetchall()
            }
            assert "feedback" in tables
            assert "incident_state_log" in tables
        finally:
            conn.close()

    def test_downgrade_refused(self, tmp_path):
        db_path = tmp_path / "events.db"
        migrate(db_path)

        conn = sqlite3.connect(db_path)
        conn.execute(
            "UPDATE _schema_version SET version = ? WHERE schema_name = ?",
            (CURRENT_SCHEMA_VERSION + 10, "events"),
        )
        conn.commit()
        conn.close()

        with pytest.raises(Exception, match="Downgrades are not supported"):
            migrate(db_path)


# ---------------------------------------------------------------------------
# Integrity check tests
# ---------------------------------------------------------------------------


class TestIntegrityCheck:
    def test_passes_on_healthy_db(self, tmp_path):
        db_path = tmp_path / "events.db"
        migrate(db_path)
        assert integrity_check(db_path) is True

    def test_fails_on_missing_db(self, tmp_path):
        with pytest.raises(Exception, match="does not exist"):
            integrity_check(tmp_path / "missing.db")

    def test_fails_on_version_mismatch(self, tmp_path):
        db_path = tmp_path / "events.db"
        migrate(db_path)

        conn = sqlite3.connect(db_path)
        conn.execute("UPDATE _schema_version SET version = 0 WHERE schema_name = 'events'")
        conn.commit()
        conn.close()

        with pytest.raises(Exception, match="version mismatch"):
            integrity_check(db_path)

    def test_fails_on_corrupt_db(self, tmp_path):
        db_path = tmp_path / "events.db"
        db_path.write_bytes(b"not a database at all" * 100)
        with pytest.raises(Exception):
            integrity_check(db_path)


# ---------------------------------------------------------------------------
# Retention tests
# ---------------------------------------------------------------------------


class TestRetention:
    def test_event_prune_keeps_fresh_rows(self, tmp_path):
        store = SqliteEventStore(tmp_path / "events.db", start_pruner=False)
        try:
            now = datetime.now(tz=UTC)
            fresh = _event("fresh-1", now, severity=Severity.WARN)
            store.add_event(fresh)

            with sqlite3.connect(tmp_path / "events.db") as conn:
                conn.execute(
                    "INSERT INTO events (event_id, timestamp, timestamp_source, service_id, host_id, "
                    "severity, event_type, message, fingerprint, source_type, source_id, collected_at, "
                    "schema_version, inserted_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    (
                        "old-1", to_iso(now - timedelta(days=10)), "parsed", "api", "host-a",
                        int(Severity.ERROR), int(EventType.LOG), "old message", "fp-old",
                        "app", "app://test", to_iso(now - timedelta(days=10)), 1,
                        "2020-01-01T00:00:00.000000Z",
                    ),
                )
                conn.commit()

            deleted = store.prune_expired(72)
            assert deleted == 1
            assert store.get_event("fresh-1") is not None
            assert store.get_event("old-1") is None
        finally:
            store.close()

    def test_incident_archive_moves_old_resolved(self, tmp_path):
        store = SqliteIncidentStore(tmp_path / "incidents.db")
        try:
            created_at = datetime(2026, 1, 1, 12, 0, tzinfo=UTC)
            incident = Incident(
                incident_id="inc-archive",
                state=IncidentState.RESOLVED,
                created_at=created_at,
                updated_at=created_at,
                clusters=[],
                events=["e1"],
                affected_services={"api"},
                primary_service="api",
                time_range=(created_at, created_at + timedelta(minutes=5)),
                severity=Severity.ERROR,
            )
            store.create_incident(incident)

            with sqlite3.connect(tmp_path / "incidents.db") as conn:
                conn.execute(
                    "UPDATE incidents SET updated_at = ? WHERE incident_id = ?",
                    ("2020-01-01T00:00:00.000000Z", "inc-archive"),
                )
                conn.commit()

            archived = store.archive_old_incidents(archive_after_days=7)
            assert archived == 1
            assert store.get_incident("inc-archive") is None
            assert list((tmp_path / "archive").glob("incidents_*.db"))
        finally:
            store.close()


# ---------------------------------------------------------------------------
# Vacuum and backup tests
# ---------------------------------------------------------------------------


class TestVacuumAndBackup:
    def test_vacuum_runs_without_error(self, tmp_path):
        db_path = tmp_path / "events.db"
        migrate(db_path)
        vacuum_db(db_path)

    def test_backup_creates_copy(self, tmp_path):
        db_path = tmp_path / "events.db"
        migrate(db_path)

        conn = sqlite3.connect(db_path)
        conn.execute("INSERT INTO collector_state(collector_id, state_key, state_value, updated_at) VALUES (?, ?, ?, ?)",
                      ("c1", "k1", "v1", "2026-01-01T00:00:00Z"))
        conn.commit()
        conn.close()

        dest = tmp_path / "backups" / "events.db"
        result = backup_db(db_path, dest)
        assert result == dest
        assert dest.exists()

        conn = sqlite3.connect(dest)
        row = conn.execute("SELECT state_value FROM collector_state WHERE collector_id = 'c1'").fetchone()
        conn.close()
        assert row[0] == "v1"


# ---------------------------------------------------------------------------
# WAL tuning tests
# ---------------------------------------------------------------------------


class TestWALTuning:
    def test_wal_mode_enabled(self, tmp_path):
        store = SqliteEventStore(tmp_path / "events.db", start_pruner=False)
        try:
            with sqlite3.connect(tmp_path / "events.db") as conn:
                mode = conn.execute("PRAGMA journal_mode").fetchone()[0]
            assert str(mode).lower() == "wal"
        finally:
            store.close()

    def test_mmap_size_applied(self, tmp_path):
        from storage.connection import connect_sqlite

        db_path = tmp_path / "test_mmap.db"
        conn = connect_sqlite(db_path, mmap_size_bytes=128 * 1024 * 1024)
        try:
            mmap_val = conn.execute("PRAGMA mmap_size").fetchone()[0]
            assert mmap_val >= 128 * 1024 * 1024
        finally:
            conn.close()


# ---------------------------------------------------------------------------
# Concurrent insert test
# ---------------------------------------------------------------------------


class TestConcurrency:
    def test_concurrent_insert_batch_no_corruption(self, tmp_path):
        store = SqliteEventStore(tmp_path / "events.db", start_pruner=False, batch_size=50)
        now = datetime.now(tz=UTC)

        errors: list[Exception] = []

        def _writer(thread_idx: int) -> None:
            try:
                events = [
                    _event(
                        f"t{thread_idx}-e{i}",
                        now + timedelta(seconds=thread_idx * 100 + i),
                        service_id=f"svc-{thread_idx}",
                    )
                    for i in range(50)
                ]
                store.insert_batch(events)
            except Exception as exc:
                errors.append(exc)

        threads = [threading.Thread(target=_writer, args=(i,)) for i in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=30)

        store.close()

        assert not errors, f"Concurrent inserts raised errors: {errors}"

        conn = sqlite3.connect(tmp_path / "events.db")
        count = conn.execute("SELECT COUNT(*) FROM events").fetchone()[0]
        conn.close()
        assert count == 200

        assert integrity_check(tmp_path / "events.db") is True


# ---------------------------------------------------------------------------
# Store-level round-trip with new schema
# ---------------------------------------------------------------------------


class TestStoreRoundTrip:
    def test_event_store_full_lifecycle(self, tmp_path):
        store = SqliteEventStore(tmp_path / "events.db", start_pruner=False)
        try:
            now = datetime.now(tz=UTC)
            events = [_event(f"e-{i}", now + timedelta(seconds=i)) for i in range(10)]
            inserted = store.insert_batch(events)
            assert inserted == 10

            queried = list(store.query_time_range(events[0].timestamp, events[-1].timestamp))
            assert len(queried) == 10

            assert store.fingerprint_exists("fp-e-0") is True
            assert store.fingerprint_exists("missing") is False

            store.set_collector_state("coll-1", "bookmark", "42")
            assert store.get_collector_state("coll-1", "bookmark") == "42"
        finally:
            store.close()

    def test_incident_store_with_feedback_and_state_log(self, tmp_path):
        store = SqliteIncidentStore(tmp_path / "incidents.db")
        try:
            created_at = datetime(2026, 5, 4, 12, 0, tzinfo=UTC)
            incident = Incident(
                incident_id="inc-state-test",
                state=IncidentState.INVESTIGATING,
                created_at=created_at,
                updated_at=created_at,
                clusters=[],
                events=["e1"],
                affected_services={"api"},
                primary_service="api",
                time_range=(created_at, created_at + timedelta(minutes=5)),
                severity=Severity.ERROR,
            )
            store.create_incident(incident)

            store.resolve_incident(
                "inc-state-test",
                ResolutionInfo(
                    resolved_by="operator",
                    correct_hypothesis_id=None,
                    feedback_type="confirmed",
                    resolved_at=created_at + timedelta(hours=1),
                ),
            )

            resolved = store.get_incident("inc-state-test")
            assert resolved is not None
            assert resolved.state == IncidentState.RESOLVED

            conn = sqlite3.connect(tmp_path / "incidents.db")
            try:
                log_rows = conn.execute(
                    "SELECT * FROM incident_state_log WHERE incident_id = ?",
                    ("inc-state-test",),
                ).fetchall()
                assert len(log_rows) >= 1
            finally:
                conn.close()
        finally:
            store.close()

    def test_new_tables_accessible(self, tmp_path):
        events_path = tmp_path / "events.db"
        incidents_path = tmp_path / "incidents.db"
        migrate(events_path)
        migrate(incidents_path)

        conn = sqlite3.connect(events_path)
        try:
            conn.execute(
                "INSERT INTO fingerprint_seen(fingerprint, first_seen_at, last_seen_at, hit_count) "
                "VALUES ('fp1', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1)"
            )
            conn.execute(
                "INSERT INTO dedup_window(fingerprint, first_event_id, last_event_id, count, "
                "window_start, window_end, suppressed_count) "
                "VALUES ('fp1', 'e1', 'e2', 5, '2026-01-01T00:00:00Z', '2026-01-01T00:01:00Z', 3)"
            )
            conn.commit()
            row = conn.execute("SELECT hit_count FROM fingerprint_seen WHERE fingerprint = 'fp1'").fetchone()
            assert row[0] == 1
        finally:
            conn.close()

        conn = sqlite3.connect(incidents_path)
        try:
            conn.execute(
                "INSERT INTO incidents(incident_id, state, created_at, updated_at, severity, "
                "affected_services, time_range_start, time_range_end, event_count, schema_version) "
                "VALUES ('i1', 'investigating', '2026-01-01', '2026-01-01', 3, '[]', '2026-01-01', '2026-01-01', 0, 3)"
            )
            conn.execute(
                "INSERT INTO feedback(feedback_id, incident_id, feedback_type, resolved_at) "
                "VALUES ('fb1', 'i1', 'confirmed', '2026-01-01T00:00:00Z')"
            )
            conn.execute(
                "INSERT INTO incident_state_log(incident_id, old_state, new_state, reason) "
                "VALUES ('i1', 'new', 'investigating', 'auto')"
            )
            conn.commit()
            fb = conn.execute("SELECT feedback_type FROM feedback WHERE feedback_id = 'fb1'").fetchone()
            assert fb[0] == "confirmed"
        finally:
            conn.close()
