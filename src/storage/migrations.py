from __future__ import annotations

import shutil
import sqlite3
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from core.errors import StorageError
from core.logging import get_logger

_log = get_logger(__name__)

CURRENT_SCHEMA_VERSION = 5


@dataclass(frozen=True)
class Migration:
    version: int
    description: str
    up_sql: str
    down_sql: str
    up_hook: Callable[[sqlite3.Connection], None] | None = None


# ---------------------------------------------------------------------------
# Events database migrations
# ---------------------------------------------------------------------------

_EVENTS_V1_UP = """
CREATE TABLE IF NOT EXISTS _schema_version (
    schema_name TEXT PRIMARY KEY,
    version INTEGER NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS schema_version (
    name TEXT PRIMARY KEY,
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
    event_id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL,
    timestamp_source TEXT NOT NULL,
    service_id TEXT NOT NULL,
    host_id TEXT NOT NULL,
    severity INTEGER NOT NULL,
    event_type INTEGER NOT NULL,
    message TEXT NOT NULL,
    structured_data TEXT,
    tags TEXT,
    fingerprint TEXT NOT NULL,
    quality TEXT,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    raw_offset INTEGER,
    collected_at TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    inserted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_events_service_ts ON events(service_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_events_severity_ts ON events(severity, timestamp);
CREATE INDEX IF NOT EXISTS idx_events_fingerprint ON events(fingerprint);
CREATE INDEX IF NOT EXISTS idx_events_inserted ON events(inserted_at);

CREATE TABLE IF NOT EXISTS collector_state (
    collector_id TEXT NOT NULL,
    state_key TEXT NOT NULL,
    state_value TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (collector_id, state_key)
);

CREATE INDEX IF NOT EXISTS idx_collector_state_updated ON collector_state(updated_at);
"""

_EVENTS_V1_DOWN = """
DROP TABLE IF EXISTS collector_state;
DROP TABLE IF EXISTS events;
DROP TABLE IF EXISTS schema_version;
DROP TABLE IF EXISTS _schema_version;
"""

_EVENTS_V2_UP = """
CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_events_service_ts ON events(service_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_events_severity_ts ON events(severity, timestamp);
CREATE INDEX IF NOT EXISTS idx_events_fingerprint ON events(fingerprint);
CREATE INDEX IF NOT EXISTS idx_events_inserted ON events(inserted_at);
"""

_EVENTS_V2_DOWN = ""

_EVENTS_V3_UP = """
CREATE TABLE IF NOT EXISTS raw_events (
    raw_event_id TEXT PRIMARY KEY,
    event_id TEXT,
    raw_payload TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    collected_at TEXT NOT NULL,
    inserted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_raw_events_event_id ON raw_events(event_id);
CREATE INDEX IF NOT EXISTS idx_raw_events_inserted ON raw_events(inserted_at);

CREATE TABLE IF NOT EXISTS fingerprint_seen (
    fingerprint TEXT PRIMARY KEY,
    first_seen_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    hit_count INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS dedup_window (
    fingerprint TEXT PRIMARY KEY,
    first_event_id TEXT NOT NULL,
    last_event_id TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 1,
    window_start TEXT NOT NULL,
    window_end TEXT NOT NULL,
    suppressed_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_dedup_window_end ON dedup_window(window_end);

CREATE INDEX IF NOT EXISTS idx_events_service_severity_ts
    ON events(service_id, severity, timestamp);
CREATE INDEX IF NOT EXISTS idx_events_type_ts ON events(event_type, timestamp);
"""

_EVENTS_V3_DOWN = """
DROP INDEX IF EXISTS idx_events_type_ts;
DROP INDEX IF EXISTS idx_events_service_severity_ts;
DROP TABLE IF EXISTS dedup_window;
DROP TABLE IF EXISTS fingerprint_seen;
DROP TABLE IF EXISTS raw_events;
"""

_EVENTS_V4_UP = """
CREATE INDEX IF NOT EXISTS idx_events_host_ts ON events(host_id, timestamp);
"""

_EVENTS_V4_DOWN = """
DROP INDEX IF EXISTS idx_events_host_ts;
"""

_EVENTS_V5_UP = """
SELECT 1;
"""

_EVENTS_V5_DOWN = ""

EVENTS_MIGRATIONS: list[Migration] = [
    Migration(
        version=1,
        description="Initial events schema with base tables and indexes",
        up_sql=_EVENTS_V1_UP,
        down_sql=_EVENTS_V1_DOWN,
    ),
    Migration(
        version=2,
        description="Ensure all read-path indexes exist",
        up_sql=_EVENTS_V2_UP,
        down_sql=_EVENTS_V2_DOWN,
    ),
    Migration(
        version=3,
        description="Add raw_events, fingerprint_seen, dedup_window tables and composite indexes",
        up_sql=_EVENTS_V3_UP,
        down_sql=_EVENTS_V3_DOWN,
    ),
    Migration(
        version=4,
        description="Add host_id+timestamp index for operational queries",
        up_sql=_EVENTS_V4_UP,
        down_sql=_EVENTS_V4_DOWN,
    ),
    Migration(
        version=5,
        description="Keep events schema version aligned with incidents release",
        up_sql=_EVENTS_V5_UP,
        down_sql=_EVENTS_V5_DOWN,
    ),
]


# ---------------------------------------------------------------------------
# Incidents database migrations
# ---------------------------------------------------------------------------

_INCIDENTS_V1_UP = """
CREATE TABLE IF NOT EXISTS _schema_version (
    schema_name TEXT PRIMARY KEY,
    version INTEGER NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS schema_version (
    name TEXT PRIMARY KEY,
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS incidents (
    incident_id TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    severity INTEGER NOT NULL,
    primary_service TEXT,
    affected_services TEXT NOT NULL,
    time_range_start TEXT NOT NULL,
    time_range_end TEXT NOT NULL,
    event_count INTEGER NOT NULL DEFAULT 0,
    schema_version INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS incident_events (
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    event_id TEXT NOT NULL,
    added_at TEXT NOT NULL,
    PRIMARY KEY (incident_id, event_id)
);

CREATE TABLE IF NOT EXISTS incident_clusters (
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    cluster_id TEXT NOT NULL,
    cluster_data TEXT NOT NULL,
    PRIMARY KEY (incident_id, cluster_id)
);

CREATE TABLE IF NOT EXISTS hypotheses (
    hypothesis_id TEXT PRIMARY KEY,
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    rank INTEGER,
    cause_type TEXT NOT NULL,
    description TEXT NOT NULL,
    total_score REAL,
    score_breakdown TEXT NOT NULL,
    supporting_events TEXT NOT NULL,
    contradicting_events TEXT NOT NULL,
    affected_services TEXT NOT NULL,
    suggested_checks TEXT NOT NULL,
    confidence_label TEXT,
    is_valid INTEGER NOT NULL DEFAULT 1,
    invalidation_reasons TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS explanations (
    explanation_id TEXT PRIMARY KEY,
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    summary TEXT NOT NULL,
    primary_text TEXT NOT NULL,
    evidence_text TEXT,
    timeline_text TEXT,
    alternatives TEXT NOT NULL,
    actions TEXT NOT NULL,
    uncertainty TEXT NOT NULL,
    model_used TEXT NOT NULL,
    guardrail_flags TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS inference_graph_snapshots (
    incident_id TEXT PRIMARY KEY REFERENCES incidents(incident_id),
    graph_data TEXT NOT NULL,
    created_at TEXT NOT NULL,
    event_count INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_incidents_state ON incidents(state);
CREATE INDEX IF NOT EXISTS idx_incidents_updated ON incidents(updated_at);
CREATE INDEX IF NOT EXISTS idx_ie_incident ON incident_events(incident_id);
CREATE INDEX IF NOT EXISTS idx_ie_event ON incident_events(event_id);
CREATE INDEX IF NOT EXISTS idx_clusters_incident ON incident_clusters(incident_id);
CREATE INDEX IF NOT EXISTS idx_hypotheses_incident ON hypotheses(incident_id);
CREATE INDEX IF NOT EXISTS idx_hyp_incident_rank ON hypotheses(incident_id, rank);
CREATE INDEX IF NOT EXISTS idx_explanations_incident ON explanations(incident_id);
CREATE INDEX IF NOT EXISTS idx_incidents_state_severity ON incidents(state, severity DESC);
"""

_INCIDENTS_V1_DOWN = """
DROP TABLE IF EXISTS inference_graph_snapshots;
DROP TABLE IF EXISTS explanations;
DROP TABLE IF EXISTS hypotheses;
DROP TABLE IF EXISTS incident_clusters;
DROP TABLE IF EXISTS incident_events;
DROP TABLE IF EXISTS incidents;
DROP TABLE IF EXISTS schema_version;
DROP TABLE IF EXISTS _schema_version;
"""

_INCIDENTS_V2_UP = """
CREATE INDEX IF NOT EXISTS idx_incidents_state ON incidents(state);
CREATE INDEX IF NOT EXISTS idx_incidents_updated ON incidents(updated_at);
CREATE INDEX IF NOT EXISTS idx_ie_incident ON incident_events(incident_id);
CREATE INDEX IF NOT EXISTS idx_ie_event ON incident_events(event_id);
CREATE INDEX IF NOT EXISTS idx_clusters_incident ON incident_clusters(incident_id);
CREATE INDEX IF NOT EXISTS idx_hypotheses_incident ON hypotheses(incident_id);
CREATE INDEX IF NOT EXISTS idx_hyp_incident_rank ON hypotheses(incident_id, rank);
CREATE INDEX IF NOT EXISTS idx_explanations_incident ON explanations(incident_id);
CREATE INDEX IF NOT EXISTS idx_incidents_state_severity ON incidents(state, severity DESC);
"""

_INCIDENTS_V2_DOWN = ""


def _incidents_v3_add_columns(conn: sqlite3.Connection) -> None:
    for column, definition in [
        ("cluster_ids", "TEXT NOT NULL DEFAULT '[]'"),
        ("runtime_context", "TEXT"),
        ("resolution_info", "TEXT"),
    ]:
        try:
            conn.execute(f"ALTER TABLE incidents ADD COLUMN {column} {definition}")
        except sqlite3.OperationalError as exc:
            if "duplicate column name" not in str(exc).lower():
                raise


_INCIDENTS_V3_UP = """
CREATE TABLE IF NOT EXISTS feedback (
    feedback_id TEXT PRIMARY KEY,
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    correct_hypothesis_id TEXT,
    feedback_type TEXT NOT NULL,
    operator_notes TEXT NOT NULL DEFAULT '',
    resolved_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_feedback_incident ON feedback(incident_id);
CREATE INDEX IF NOT EXISTS idx_feedback_created ON feedback(created_at);

CREATE TABLE IF NOT EXISTS incident_state_log (
    log_id INTEGER PRIMARY KEY AUTOINCREMENT,
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    old_state TEXT NOT NULL,
    new_state TEXT NOT NULL,
    changed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    reason TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_state_log_incident ON incident_state_log(incident_id);
CREATE INDEX IF NOT EXISTS idx_state_log_changed ON incident_state_log(changed_at);

CREATE INDEX IF NOT EXISTS idx_incidents_created ON incidents(created_at);
CREATE INDEX IF NOT EXISTS idx_incidents_severity ON incidents(severity DESC);
"""

_INCIDENTS_V3_DOWN = """
DROP TABLE IF EXISTS incident_state_log;
DROP TABLE IF EXISTS feedback;
DROP INDEX IF EXISTS idx_incidents_severity;
DROP INDEX IF EXISTS idx_incidents_created;
"""

_INCIDENTS_V4_UP = """
ALTER TABLE explanations ADD COLUMN explanation_schema_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE explanations ADD COLUMN hypotheses_hash TEXT NOT NULL DEFAULT '';
ALTER TABLE explanations ADD COLUMN events_hash_head TEXT NOT NULL DEFAULT '';
CREATE INDEX IF NOT EXISTS idx_explanations_cache ON explanations(incident_id, hypotheses_hash, events_hash_head);
"""

_INCIDENTS_V4_DOWN = """
DROP INDEX IF EXISTS idx_explanations_cache;
"""

_INCIDENTS_V5_UP = """
CREATE TABLE IF NOT EXISTS incident_ai_traces (
    trace_id TEXT PRIMARY KEY,
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    trace_kind TEXT NOT NULL,
    sanitized_system_prompt TEXT NOT NULL,
    sanitized_user_prompt TEXT NOT NULL,
    allowed_fields TEXT NOT NULL,
    blocked_fields TEXT NOT NULL,
    raw_logs_sent INTEGER NOT NULL DEFAULT 0,
    trace_schema_version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_ai_traces_incident ON incident_ai_traces(incident_id, created_at DESC);

CREATE TABLE IF NOT EXISTS incident_chat_messages (
    message_id TEXT PRIMARY KEY,
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    message_schema_version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_chat_incident ON incident_chat_messages(incident_id, created_at ASC);

ALTER TABLE explanations ADD COLUMN quality TEXT NOT NULL DEFAULT 'ok';
"""

_INCIDENTS_V5_DOWN = """
DROP TABLE IF EXISTS incident_chat_messages;
DROP TABLE IF EXISTS incident_ai_traces;
"""


INCIDENTS_MIGRATIONS: list[Migration] = [
    Migration(
        version=1,
        description="Initial incidents schema with base tables and indexes",
        up_sql=_INCIDENTS_V1_UP,
        down_sql=_INCIDENTS_V1_DOWN,
    ),
    Migration(
        version=2,
        description="Ensure all read-path indexes exist",
        up_sql=_INCIDENTS_V2_UP,
        down_sql=_INCIDENTS_V2_DOWN,
    ),
    Migration(
        version=3,
        description="Add feedback, incident_state_log tables, extra incident columns, and composite indexes",
        up_sql=_INCIDENTS_V3_UP,
        down_sql=_INCIDENTS_V3_DOWN,
        up_hook=_incidents_v3_add_columns,
    ),
    Migration(
        version=4,
        description="Explanation cache key columns on explanations",
        up_sql=_INCIDENTS_V4_UP,
        down_sql=_INCIDENTS_V4_DOWN,
    ),
    Migration(
        version=5,
        description="AI audit traces, persisted chat, explanation quality flag",
        up_sql=_INCIDENTS_V5_UP,
        down_sql=_INCIDENTS_V5_DOWN,
    ),
]


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def _schema_name(path: Path) -> str:
    lowered = path.name.lower()
    if lowered == "events.db" or lowered.startswith("events_"):
        return "events"
    if lowered == "incidents.db" or lowered.startswith("incidents_"):
        return "incidents"
    raise StorageError(f"Cannot determine schema type for database path: {path}")


def _get_migrations(path: Path) -> list[Migration]:
    name = _schema_name(path)
    if name == "events":
        return EVENTS_MIGRATIONS
    return INCIDENTS_MIGRATIONS


def migrate(db_path: str | Path) -> int:
    path = Path(db_path)
    path.parent.mkdir(parents=True, exist_ok=True)
    schema_name = _schema_name(path)
    migrations = _get_migrations(path)

    conn = sqlite3.connect(path)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    try:
        conn.execute(
            """
            CREATE TABLE IF NOT EXISTS _schema_version (
                schema_name TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            )
            """
        )

        current = _current_version(conn, schema_name)
        target = migrations[-1].version if migrations else 0

        if current > target:
            raise StorageError(
                f"Database {path} is at schema version {current} which is ahead of "
                f"code version {target}. Downgrades are not supported."
            )

        pending = [m for m in migrations if m.version > current]
        if not pending:
            return current

        conn.execute("BEGIN IMMEDIATE")
        try:
            for m in pending:
                if m.version <= current:
                    raise StorageError(
                        f"Migration version {m.version} would lower or repeat "
                        f"schema_version (current={current}). Aborting."
                    )
                _log.info(
                    "Applying migration",
                    extra={"db": path.name, "version": m.version, "description": m.description},
                )
                conn.executescript(m.up_sql)
                if m.up_hook is not None:
                    m.up_hook(conn)
                current = m.version

            conn.execute(
                """
                INSERT INTO _schema_version(schema_name, version, applied_at)
                VALUES (?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                ON CONFLICT(schema_name) DO UPDATE SET
                    version = excluded.version,
                    applied_at = excluded.applied_at
                """,
                (schema_name, current),
            )
            conn.execute(
                "CREATE TABLE IF NOT EXISTS schema_version (name TEXT PRIMARY KEY, version INTEGER NOT NULL)"
            )
            conn.execute(
                "INSERT OR REPLACE INTO schema_version(name, version) VALUES (?, ?)",
                (schema_name, current),
            )
            conn.commit()
        except Exception:
            conn.rollback()
            raise

        _log.info("Migration complete", extra={"db": path.name, "version": current})
        return current
    finally:
        conn.close()


def integrity_check(db_path: str | Path) -> bool:
    path = Path(db_path)
    if not path.exists():
        raise StorageError(f"Database does not exist: {path}")

    conn = sqlite3.connect(path)
    try:
        row = conn.execute("PRAGMA integrity_check").fetchone()
        if row is None or row[0] != "ok":
            detail = row[0] if row else "no result"
            raise StorageError(f"SQLite integrity check failed for {path}: {detail}")

        schema_name = _schema_name(path)
        version = _current_version(conn, schema_name)
        if version != CURRENT_SCHEMA_VERSION:
            raise StorageError(
                f"Schema version mismatch for {path}: database={version}, code={CURRENT_SCHEMA_VERSION}"
            )
        return True
    finally:
        conn.close()


def vacuum_db(db_path: str | Path) -> None:
    path = Path(db_path)
    if not path.exists():
        raise StorageError(f"Database does not exist: {path}")
    conn = sqlite3.connect(path)
    try:
        conn.execute("PRAGMA incremental_vacuum")
    finally:
        conn.close()


def backup_db(db_path: str | Path, dest_path: str | Path) -> Path:
    src = Path(db_path)
    dest = Path(dest_path)
    if not src.exists():
        raise StorageError(f"Database does not exist: {src}")
    dest.parent.mkdir(parents=True, exist_ok=True)

    source_conn = sqlite3.connect(src)
    target_conn = sqlite3.connect(dest)
    try:
        source_conn.backup(target_conn)
    finally:
        target_conn.close()
        source_conn.close()

    wal_file = src.parent / f"{src.name}-wal"
    if wal_file.exists():
        shutil.copy2(wal_file, dest.parent / f"{dest.name}-wal")

    _log.info("Database backup created", extra={"source": str(src), "destination": str(dest)})
    return dest


def _current_version(conn: sqlite3.Connection, schema_name: str) -> int:
    try:
        row = conn.execute(
            "SELECT version FROM _schema_version WHERE schema_name = ?",
            (schema_name,),
        ).fetchone()
    except sqlite3.OperationalError:
        return 0
    return int(row[0]) if row else 0
