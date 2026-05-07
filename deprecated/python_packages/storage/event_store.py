from __future__ import annotations

import sqlite3
import threading
from collections.abc import Iterable, Iterator
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any, Protocol

from core.enums import EventType, Severity
from core.logging import get_logger
from core.time import parse_datetime, to_iso, utc_now
from events.models import DataQuality, EventFilter, NormalizedEvent, SourceRef, thaw_value
from events.serialization import json_dumps

from .connection import SqliteConnectionPool, transaction
from .migrations import migrate

_log = get_logger(__name__)

FETCH_SIZE = 100


class EventStore(Protocol):
    path: Path

    def insert_batch(self, events: list[NormalizedEvent]) -> int: ...

    def add_event(self, event: NormalizedEvent) -> None: ...

    def add_events(self, events: Iterable[NormalizedEvent]) -> int: ...

    def query_time_range(
        self,
        start: datetime,
        end: datetime,
        filters: EventFilter | None = None,
        limit: int | None = None,
        offset: int = 0,
    ) -> Iterator[NormalizedEvent]: ...

    def query_by_service(
        self,
        service_id: str,
        window: timedelta,
        limit: int | None = None,
    ) -> Iterator[NormalizedEvent]: ...

    def count_by_severity(self, service_id: str, severity: Severity, window: timedelta) -> int: ...

    def get_event(self, event_id: str) -> NormalizedEvent | None: ...

    def fingerprint_exists(self, fingerprint: str) -> bool: ...

    def prune_expired(self, retention_hours: int) -> int: ...

    def count_events(self, service_id: str | None = None, window: timedelta | None = None) -> int: ...

    def latest_events(self, limit: int = 100) -> list[NormalizedEvent]: ...

    def list_services(self) -> list[dict[str, Any]]: ...

    def get_collector_state(self, collector_id: str, state_key: str) -> str | None: ...

    def set_collector_state(self, collector_id: str, state_key: str, state_value: str) -> None: ...

    def close(self) -> None: ...


class SqliteEventStore(EventStore):
    def __init__(
        self,
        path: str | Path,
        *,
        batch_size: int = 100,
        retention_hours: int = 72,
        prune_interval_seconds: int = 60,
        wal_mode: bool = True,
        start_pruner: bool = True,
        mmap_size_bytes: int = 0,
    ) -> None:
        self.path = Path(path)
        self.batch_size = max(1, int(batch_size))
        self.retention_hours = retention_hours
        self.prune_interval_seconds = max(1, int(prune_interval_seconds))
        self.wal_mode = wal_mode

        migrate(self.path)

        self._pool = SqliteConnectionPool(
            self.path,
            wal_mode=wal_mode,
            mmap_size_bytes=mmap_size_bytes,
        )
        self._write_lock = threading.RLock()
        self._pruner_stop = threading.Event()
        self._pruner: threading.Thread | None = None
        if start_pruner:
            self._start_pruner()

    def close(self) -> None:
        self._pruner_stop.set()
        if self._pruner and self._pruner.is_alive():
            self._pruner.join(timeout=2.0)
        self._final_prune()
        self._pool.close()

    def insert_batch(self, events: list[NormalizedEvent]) -> int:
        if not events:
            return 0
        inserted = 0
        conn = self._pool.writer()
        statement = """
            INSERT OR IGNORE INTO events (
                event_id, timestamp, timestamp_source, service_id, host_id,
                severity, event_type, message, structured_data, tags,
                fingerprint, quality, source_type, source_id, raw_offset,
                collected_at, schema_version
            ) VALUES (
                :event_id, :timestamp, :timestamp_source, :service_id, :host_id,
                :severity, :event_type, :message, :structured_data, :tags,
                :fingerprint, :quality, :source_type, :source_id, :raw_offset,
                :collected_at, :schema_version
            )
        """
        rows = [self._event_to_row(event) for event in events]
        with self._write_lock, transaction(conn):
            for offset in range(0, len(rows), self.batch_size):
                chunk = rows[offset : offset + self.batch_size]
                before = conn.total_changes
                conn.executemany(statement, chunk)
                inserted += conn.total_changes - before
        return inserted

    def query_time_range(
        self,
        start: datetime,
        end: datetime,
        filters: EventFilter | None = None,
        limit: int | None = None,
        offset: int = 0,
    ) -> Iterator[NormalizedEvent]:
        where = ["timestamp >= ?", "timestamp <= ?"]
        params: list[Any] = [to_iso(start), to_iso(end)]
        self._apply_filters(where, params, filters)
        query = "SELECT * FROM events WHERE " + " AND ".join(where) + " ORDER BY timestamp ASC"
        if limit is not None:
            query += " LIMIT ? OFFSET ?"
            params.extend([limit, max(0, offset)])
        yield from self._iter_query(query, tuple(params))

    def query_by_service(
        self,
        service_id: str,
        window: timedelta,
        limit: int | None = None,
    ) -> Iterator[NormalizedEvent]:
        end = utc_now()
        start = end - window
        params: list[Any] = [service_id, to_iso(start), to_iso(end)]
        query = """
            SELECT * FROM events INDEXED BY idx_events_service_ts
            WHERE service_id = ? AND timestamp >= ? AND timestamp <= ?
            ORDER BY timestamp ASC
        """
        if limit is not None:
            query += " LIMIT ?"
            params.append(limit)
        yield from self._iter_query(query, tuple(params))

    def count_by_severity(self, service_id: str, severity: Severity, window: timedelta) -> int:
        end = utc_now()
        start = end - window
        row = self._pool.reader().execute(
            """
            SELECT COUNT(*) AS count FROM events INDEXED BY idx_events_severity_ts
            WHERE service_id = ? AND severity >= ? AND timestamp >= ? AND timestamp <= ?
            """,
            (service_id, int(severity), to_iso(start), to_iso(end)),
        ).fetchone()
        return int(row["count"]) if row else 0

    def get_event(self, event_id: str) -> NormalizedEvent | None:
        row = self._pool.reader().execute("SELECT * FROM events WHERE event_id = ?", (event_id,)).fetchone()
        return self._row_to_event(row) if row is not None else None

    def fingerprint_exists(self, fingerprint: str) -> bool:
        row = self._pool.reader().execute(
            "SELECT 1 FROM events WHERE fingerprint = ? LIMIT 1",
            (fingerprint,),
        ).fetchone()
        return row is not None

    def prune_expired(self, retention_hours: int) -> int:
        cutoff = utc_now() - timedelta(hours=retention_hours)
        conn = self._pool.writer()
        with self._write_lock, transaction(conn):
            cursor = conn.execute("DELETE FROM events WHERE inserted_at < ?", (to_iso(cutoff),))
            deleted = max(0, cursor.rowcount)
            conn.execute("PRAGMA incremental_vacuum")
        if deleted > 0:
            _log.info("Pruned expired events", extra={"deleted": deleted, "retention_hours": retention_hours})
        return deleted

    def count_events(self, service_id: str | None = None, window: timedelta | None = None) -> int:
        where: list[str] = []
        params: list[Any] = []
        if service_id is not None:
            where.append("service_id = ?")
            params.append(service_id)
        if window is not None:
            end = utc_now()
            start = end - window
            where.extend(["timestamp >= ?", "timestamp <= ?"])
            params.extend([to_iso(start), to_iso(end)])
        query = "SELECT COUNT(*) AS count FROM events"
        if where:
            query += " WHERE " + " AND ".join(where)
        row = self._pool.reader().execute(query, tuple(params)).fetchone()
        return int(row["count"]) if row else 0

    def add_event(self, event: NormalizedEvent) -> None:
        self.insert_batch([event])

    def add_events(self, events: Iterable[NormalizedEvent]) -> int:
        return self.insert_batch(list(events))

    def count_by_service(self, service_id: str, window: timedelta) -> int:
        return self.count_events(service_id=service_id, window=window)

    def latest_events(self, limit: int = 100) -> list[NormalizedEvent]:
        rows = self._pool.reader().execute(
            "SELECT * FROM events ORDER BY timestamp DESC LIMIT ?",
            (limit,),
        ).fetchall()
        return [self._row_to_event(row) for row in reversed(rows)]

    def list_services(self) -> list[dict[str, Any]]:
        rows = self._pool.reader().execute(
            """
            SELECT
                service_id,
                COUNT(*) AS event_count,
                SUM(CASE WHEN severity >= ? THEN 1 ELSE 0 END) AS error_count,
                MAX(timestamp) AS last_event_at
            FROM events
            GROUP BY service_id
            ORDER BY service_id ASC
            """,
            (int(Severity.ERROR),),
        ).fetchall()
        return [
            {
                "service_id": row["service_id"],
                "event_count": int(row["event_count"] or 0),
                "error_count": int(row["error_count"] or 0),
                "last_event_at": row["last_event_at"],
            }
            for row in rows
        ]

    def prune_older_than(self, retention_hours: int) -> int:
        return self.prune_expired(retention_hours)

    def get_collector_state(self, collector_id: str, state_key: str) -> str | None:
        row = self._pool.reader().execute(
            "SELECT state_value FROM collector_state WHERE collector_id = ? AND state_key = ?",
            (collector_id, state_key),
        ).fetchone()
        return str(row["state_value"]) if row else None

    def set_collector_state(self, collector_id: str, state_key: str, state_value: str) -> None:
        conn = self._pool.writer()
        with self._write_lock, transaction(conn):
            conn.execute(
                """
                INSERT INTO collector_state(collector_id, state_key, state_value, updated_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(collector_id, state_key) DO UPDATE SET
                    state_value = excluded.state_value,
                    updated_at = excluded.updated_at
                """,
                (collector_id, state_key, state_value, to_iso(utc_now())),
            )

    # -- internal helpers --

    def _iter_query(self, query: str, params: tuple[Any, ...]) -> Iterator[NormalizedEvent]:
        cursor = self._pool.reader().execute(query, params)
        try:
            while True:
                rows = cursor.fetchmany(FETCH_SIZE)
                if not rows:
                    break
                for row in rows:
                    yield self._row_to_event(row)
        finally:
            cursor.close()

    def _apply_filters(self, where: list[str], params: list[Any], filters: EventFilter | None) -> None:
        if filters is None:
            return
        self._add_in_filter(where, params, "service_id", sorted(filters.service_ids) if filters.service_ids else None)
        self._add_in_filter(where, params, "host_id", sorted(filters.host_ids) if filters.host_ids else None)
        if filters.severities:
            self._add_in_filter(where, params, "severity", [int(item) for item in sorted(filters.severities)])
        if filters.event_types:
            self._add_in_filter(where, params, "event_type", [int(item) for item in sorted(filters.event_types)])
        if filters.message_contains:
            where.append("message LIKE ?")
            params.append(f"%{filters.message_contains}%")
        if filters.tags:
            clauses: list[str] = []
            for tag in sorted(filters.tags):
                clauses.append("(tags = ? OR tags LIKE ? OR tags LIKE ? OR tags LIKE ? OR tags LIKE ?)")
                params.extend([tag, f"{tag},%", f"%,{tag},%", f"%,{tag}", f'%"{tag}"%'])
            where.append("(" + " OR ".join(clauses) + ")")

    def _add_in_filter(
        self,
        where: list[str],
        params: list[Any],
        column: str,
        values: list[Any] | None,
    ) -> None:
        if not values:
            return
        placeholders = ",".join("?" for _ in values)
        where.append(f"{column} IN ({placeholders})")
        params.extend(values)

    def _event_to_row(self, event: NormalizedEvent) -> dict[str, Any]:
        return {
            "event_id": event.event_id,
            "timestamp": to_iso(event.timestamp),
            "timestamp_source": event.timestamp_source,
            "service_id": event.service_id,
            "host_id": event.host_id,
            "severity": int(event.severity),
            "event_type": int(event.event_type),
            "message": event.message,
            "structured_data": json_dumps(thaw_value(event.structured_data)),
            "tags": ",".join(sorted(event.tags)),
            "fingerprint": event.fingerprint,
            "quality": json_dumps(
                {
                    "overall": event.quality.overall,
                    "timestamp_confidence": event.quality.timestamp_confidence,
                    "parse_confidence": event.quality.parse_confidence,
                    "identity_confidence": event.quality.identity_confidence,
                    "completeness": event.quality.completeness,
                    "flags": sorted(event.quality.flags),
                }
            ),
            "source_type": event.source_ref.source_type,
            "source_id": event.source_ref.source_id,
            "raw_offset": event.source_ref.raw_offset,
            "collected_at": to_iso(event.source_ref.collected_at),
            "schema_version": event.schema_version,
        }

    def _row_to_event(self, row: sqlite3.Row) -> NormalizedEvent:
        timestamp = parse_datetime(row["timestamp"])
        collected_at = parse_datetime(row["collected_at"])
        if timestamp is None or collected_at is None:
            raise ValueError("Stored event has invalid timestamp")
        tags = self._decode_tags(row["tags"])
        return NormalizedEvent(
            event_id=row["event_id"],
            timestamp=timestamp,
            timestamp_source=row["timestamp_source"],
            service_id=row["service_id"],
            host_id=row["host_id"],
            severity=Severity(row["severity"]),
            event_type=EventType(row["event_type"]),
            message=row["message"],
            structured_data=self._json_loads(row["structured_data"], default={}),
            tags=tags,
            fingerprint=row["fingerprint"],
            quality=DataQuality(**self._json_loads(row["quality"], default={})),
            source_ref=SourceRef(
                source_type=row["source_type"],
                source_id=row["source_id"],
                raw_offset=row["raw_offset"],
                collected_at=collected_at,
            ),
            schema_version=row["schema_version"],
        )

    def _json_loads(self, raw: str | None, default: Any) -> Any:
        if not raw:
            return default
        import json

        return json.loads(raw)

    def _decode_tags(self, raw: str | None) -> frozenset[str]:
        if not raw:
            return frozenset()
        text = raw.strip()
        if not text:
            return frozenset()
        if text.startswith("["):
            loaded = self._json_loads(text, default=[])
            return frozenset(str(item) for item in loaded if item)
        return frozenset(item for item in text.split(",") if item)

    def _start_pruner(self) -> None:
        if self._pruner is not None:
            return

        def _run() -> None:
            while not self._pruner_stop.wait(self.prune_interval_seconds):
                try:
                    self.prune_expired(self.retention_hours)
                except sqlite3.Error:
                    _log.debug("Event pruner cycle failed", exc_info=True)

        self._pruner = threading.Thread(target=_run, name="inferra-event-pruner", daemon=True)
        self._pruner.start()

    def _final_prune(self) -> None:
        try:
            self.prune_expired(self.retention_hours)
        except sqlite3.Error:
            _log.debug("Final event prune on shutdown failed", exc_info=True)


def count_events_by_service(db_path: Path, service_ids: list[str]) -> dict[str, int]:
    counts = {sid: 0 for sid in service_ids}
    if not db_path.exists():
        return counts
    try:
        conn = sqlite3.connect(db_path.resolve().as_uri() + "?mode=ro", uri=True)
    except sqlite3.Error:
        return counts
    try:
        placeholders = ",".join("?" for _ in service_ids)
        rows = conn.execute(
            f"SELECT service_id, COUNT(*) AS event_count FROM events WHERE service_id IN ({placeholders}) GROUP BY service_id",
            tuple(service_ids),
        ).fetchall()
        counts.update({str(row[0]): int(row[1] or 0) for row in rows})
    except sqlite3.Error:
        pass
    finally:
        conn.close()
    return counts
