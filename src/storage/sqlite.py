from __future__ import annotations

import json
import sqlite3
from collections.abc import Iterable, Iterator
from dataclasses import dataclass
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any

from analysis.models import EventCluster
from core.enums import EventType, IncidentState, Severity
from core.time import parse_datetime, to_iso, utc_now
from events.models import DataQuality, EventFilter, NormalizedEvent, SourceRef
from events.serialization import json_dumps
from storage.migrations import EVENTS_SCHEMA, INCIDENTS_SCHEMA


def _connect(path: Path) -> sqlite3.Connection:
    path.parent.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(path, check_same_thread=False)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA foreign_keys=ON")
    conn.execute("PRAGMA busy_timeout=5000")
    return conn


def initialize_storage(data_dir: Path) -> tuple["SQLiteEventStore", "SQLiteIncidentStore"]:
    data_dir.mkdir(parents=True, exist_ok=True)
    event_store = SQLiteEventStore(data_dir / "events.db")
    incident_store = SQLiteIncidentStore(data_dir / "incidents.db")
    event_store.initialize()
    incident_store.initialize()
    return event_store, incident_store


class SQLiteEventStore:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.conn = _connect(path)

    def initialize(self) -> None:
        self.conn.executescript(EVENTS_SCHEMA)
        self.conn.execute(
            "INSERT OR REPLACE INTO schema_version(name, version) VALUES (?, ?)",
            ("events", 1),
        )
        self.conn.commit()

    def add_event(self, event: NormalizedEvent) -> None:
        self.add_events([event])

    def add_events(self, events: Iterable[NormalizedEvent]) -> int:
        rows = [self._event_to_row(event) for event in events]
        if not rows:
            return 0
        with self.conn:
            self.conn.executemany(
                """
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
                """,
                rows,
            )
        return len(rows)

    def query_time_range(
        self,
        start: datetime,
        end: datetime,
        filters: EventFilter | None = None,
        limit: int = 1000,
        offset: int = 0,
    ) -> Iterator[NormalizedEvent]:
        where = ["timestamp >= ?", "timestamp <= ?"]
        params: list[Any] = [to_iso(start), to_iso(end)]
        self._apply_filters(where, params, filters)
        query = f"""
            SELECT * FROM events
            WHERE {" AND ".join(where)}
            ORDER BY timestamp ASC
            LIMIT ? OFFSET ?
        """
        params.extend([limit, offset])
        for row in self.conn.execute(query, params):
            yield self._row_to_event(row)

    def query_by_service(self, service_id: str, window: timedelta, limit: int = 1000) -> Iterator[NormalizedEvent]:
        end = utc_now()
        start = end - window
        filters = EventFilter(service_ids={service_id})
        yield from self.query_time_range(start, end, filters=filters, limit=limit)

    def count_by_severity(self, service_id: str, severity: Severity, window: timedelta) -> int:
        end = utc_now()
        start = end - window
        row = self.conn.execute(
            """
            SELECT COUNT(*) AS count FROM events
            WHERE service_id = ? AND severity >= ? AND timestamp >= ? AND timestamp <= ?
            """,
            (service_id, int(severity), to_iso(start), to_iso(end)),
        ).fetchone()
        return int(row["count"])

    def count_by_service(self, service_id: str, window: timedelta) -> int:
        end = utc_now()
        start = end - window
        row = self.conn.execute(
            "SELECT COUNT(*) AS count FROM events WHERE service_id = ? AND timestamp >= ? AND timestamp <= ?",
            (service_id, to_iso(start), to_iso(end)),
        ).fetchone()
        return int(row["count"])

    def get_event(self, event_id: str) -> NormalizedEvent | None:
        row = self.conn.execute("SELECT * FROM events WHERE event_id = ?", (event_id,)).fetchone()
        return self._row_to_event(row) if row else None

    def latest_events(self, limit: int = 100) -> list[NormalizedEvent]:
        rows = self.conn.execute("SELECT * FROM events ORDER BY timestamp DESC LIMIT ?", (limit,)).fetchall()
        return [self._row_to_event(row) for row in reversed(rows)]

    def list_services(self) -> list[dict[str, Any]]:
        rows = self.conn.execute(
            """
            SELECT
                service_id,
                COUNT(*) AS event_count,
                SUM(CASE WHEN severity >= 3 THEN 1 ELSE 0 END) AS error_count,
                MAX(timestamp) AS last_event_at
            FROM events
            GROUP BY service_id
            ORDER BY service_id ASC
            """
        ).fetchall()
        return [
            {
                "service_id": row["service_id"],
                "event_count": int(row["event_count"]),
                "error_count": int(row["error_count"] or 0),
                "last_event_at": row["last_event_at"],
            }
            for row in rows
        ]

    def prune_older_than(self, retention_hours: int) -> int:
        cutoff = utc_now() - timedelta(hours=retention_hours)
        with self.conn:
            cur = self.conn.execute("DELETE FROM events WHERE inserted_at < ?", (to_iso(cutoff),))
        return cur.rowcount

    def get_collector_state(self, collector_id: str, state_key: str) -> str | None:
        row = self.conn.execute(
            "SELECT state_value FROM collector_state WHERE collector_id = ? AND state_key = ?",
            (collector_id, state_key),
        ).fetchone()
        return str(row["state_value"]) if row else None

    def set_collector_state(self, collector_id: str, state_key: str, state_value: str) -> None:
        with self.conn:
            self.conn.execute(
                """
                INSERT INTO collector_state(collector_id, state_key, state_value, updated_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(collector_id, state_key) DO UPDATE SET
                    state_value = excluded.state_value,
                    updated_at = excluded.updated_at
                """,
                (collector_id, state_key, state_value, to_iso(utc_now())),
            )

    def close(self) -> None:
        self.conn.close()

    def _apply_filters(self, where: list[str], params: list[Any], filters: EventFilter | None) -> None:
        if filters is None:
            return
        if filters.service_ids:
            where.append(f"service_id IN ({','.join('?' for _ in filters.service_ids)})")
            params.extend(sorted(filters.service_ids))
        if filters.host_ids:
            where.append(f"host_id IN ({','.join('?' for _ in filters.host_ids)})")
            params.extend(sorted(filters.host_ids))
        if filters.severities:
            where.append(f"severity IN ({','.join('?' for _ in filters.severities)})")
            params.extend(int(s) for s in sorted(filters.severities))
        if filters.event_types:
            where.append(f"event_type IN ({','.join('?' for _ in filters.event_types)})")
            params.extend(int(e) for e in sorted(filters.event_types))
        if filters.message_contains:
            where.append("message LIKE ?")
            params.append(f"%{filters.message_contains}%")
        if filters.tags:
            tag_clauses = []
            for tag in sorted(filters.tags):
                tag_clauses.append("tags LIKE ?")
                params.append(f'%"{tag}"%')
            where.append("(" + " OR ".join(tag_clauses) + ")")

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
            "structured_data": json_dumps(event.structured_data),
            "tags": json_dumps(sorted(event.tags)),
            "fingerprint": event.fingerprint,
            "quality": json_dumps(event.quality.__dict__),
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
        return NormalizedEvent(
            event_id=row["event_id"],
            timestamp=timestamp,
            timestamp_source=row["timestamp_source"],
            service_id=row["service_id"],
            host_id=row["host_id"],
            severity=Severity(row["severity"]),
            event_type=EventType(row["event_type"]),
            message=row["message"],
            structured_data=json.loads(row["structured_data"]),
            tags=frozenset(json.loads(row["tags"])),
            fingerprint=row["fingerprint"],
            quality=DataQuality(**json.loads(row["quality"])),
            source_ref=SourceRef(
                source_type=row["source_type"],
                source_id=row["source_id"],
                raw_offset=row["raw_offset"],
                collected_at=collected_at,
            ),
            schema_version=row["schema_version"],
        )


@dataclass(frozen=True)
class IncidentRecord:
    incident_id: str
    state: IncidentState
    created_at: datetime
    updated_at: datetime
    severity: Severity
    primary_service: str | None
    affected_services: tuple[str, ...]
    time_range_start: datetime
    time_range_end: datetime
    event_count: int


class SQLiteIncidentStore:
    def __init__(self, path: Path) -> None:
        self.path = path
        self.conn = _connect(path)

    def initialize(self) -> None:
        self.conn.executescript(INCIDENTS_SCHEMA)
        self.conn.execute(
            "INSERT OR REPLACE INTO schema_version(name, version) VALUES (?, ?)",
            ("incidents", 1),
        )
        self.conn.commit()

    def upsert_incident(
        self,
        incident_id: str,
        state: IncidentState,
        severity: Severity,
        affected_services: Iterable[str],
        time_range_start: datetime,
        time_range_end: datetime,
        event_ids: Iterable[str],
        primary_service: str | None = None,
    ) -> None:
        now = utc_now()
        event_ids_tuple = tuple(dict.fromkeys(event_ids))
        services = tuple(sorted(set(affected_services)))
        with self.conn:
            self.conn.execute(
                """
                INSERT INTO incidents (
                    incident_id, state, created_at, updated_at, severity,
                    primary_service, affected_services, time_range_start,
                    time_range_end, event_count, schema_version
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
                ON CONFLICT(incident_id) DO UPDATE SET
                    state = excluded.state,
                    updated_at = excluded.updated_at,
                    severity = excluded.severity,
                    primary_service = excluded.primary_service,
                    affected_services = excluded.affected_services,
                    time_range_start = excluded.time_range_start,
                    time_range_end = excluded.time_range_end,
                    event_count = excluded.event_count
                """,
                (
                    incident_id,
                    state.value,
                    to_iso(now),
                    to_iso(now),
                    int(severity),
                    primary_service,
                    json_dumps(services),
                    to_iso(time_range_start),
                    to_iso(time_range_end),
                    len(event_ids_tuple),
                ),
            )
            self.conn.executemany(
                "INSERT OR IGNORE INTO incident_events(incident_id, event_id, added_at) VALUES (?, ?, ?)",
                [(incident_id, event_id, to_iso(now)) for event_id in event_ids_tuple],
            )

    def list_active(self) -> list[IncidentRecord]:
        rows = self.conn.execute(
            """
            SELECT * FROM incidents
            WHERE state IN ('open', 'investigating', 'explained')
            ORDER BY severity DESC, updated_at DESC
            """
        ).fetchall()
        return [self._row_to_incident(row) for row in rows]

    def get_incident(self, incident_id: str) -> IncidentRecord | None:
        row = self.conn.execute("SELECT * FROM incidents WHERE incident_id = ?", (incident_id,)).fetchone()
        return self._row_to_incident(row) if row else None

    def update_incident_state(self, incident_id: str, state: IncidentState) -> bool:
        with self.conn:
            cur = self.conn.execute(
                "UPDATE incidents SET state = ?, updated_at = ? WHERE incident_id = ?",
                (state.value, to_iso(utc_now()), incident_id),
            )
        return cur.rowcount > 0

    def event_ids_for_incident(self, incident_id: str) -> list[str]:
        rows = self.conn.execute(
            "SELECT event_id FROM incident_events WHERE incident_id = ? ORDER BY added_at ASC",
            (incident_id,),
        ).fetchall()
        return [row["event_id"] for row in rows]

    def upsert_cluster(self, incident_id: str, cluster: EventCluster) -> None:
        data = {
            "cluster_id": cluster.cluster_id,
            "events": cluster.events,
            "time_range": [to_iso(cluster.time_range[0]), to_iso(cluster.time_range[1])],
            "affected_services": sorted(cluster.affected_services),
            "primary_severity": int(cluster.primary_severity),
            "trigger_event_id": cluster.trigger_event_id,
            "correlation_edges": [edge.__dict__ for edge in cluster.correlation_edges],
            "anomaly_scores": cluster.anomaly_scores,
        }
        with self.conn:
            self.conn.execute(
                """
                INSERT INTO incident_clusters(incident_id, cluster_id, cluster_data)
                VALUES (?, ?, ?)
                ON CONFLICT(incident_id, cluster_id) DO UPDATE SET
                    cluster_data = excluded.cluster_data
                """,
                (incident_id, cluster.cluster_id, json_dumps(data)),
            )

    def clusters_for_incident(self, incident_id: str) -> list[dict[str, Any]]:
        rows = self.conn.execute(
            "SELECT cluster_data FROM incident_clusters WHERE incident_id = ? ORDER BY cluster_id",
            (incident_id,),
        ).fetchall()
        return [json.loads(row["cluster_data"]) for row in rows]

    def replace_hypotheses(self, incident_id: str, hypotheses: list[dict[str, Any]]) -> None:
        now = to_iso(utc_now())
        with self.conn:
            self.conn.execute("DELETE FROM hypotheses WHERE incident_id = ?", (incident_id,))
            self.conn.executemany(
                """
                INSERT INTO hypotheses (
                    hypothesis_id, incident_id, rank, cause_type, description,
                    total_score, score_breakdown, supporting_events,
                    contradicting_events, affected_services, suggested_checks,
                    confidence_label, is_valid, invalidation_reasons,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                [
                    (
                        hyp["hypothesis_id"],
                        incident_id,
                        hyp.get("rank"),
                        hyp["cause_type"],
                        hyp["description"],
                        hyp.get("total_score"),
                        json_dumps(hyp.get("score_breakdown", {})),
                        json_dumps(hyp.get("supporting_events", [])),
                        json_dumps(hyp.get("contradicting_events", [])),
                        json_dumps(hyp.get("affected_services", [])),
                        json_dumps(hyp.get("suggested_checks", [])),
                        hyp.get("confidence_label"),
                        1 if hyp.get("is_valid", True) else 0,
                        json_dumps(hyp.get("invalidation_reasons", [])),
                        now,
                        now,
                    )
                    for hyp in hypotheses
                ],
            )

    def hypotheses_for_incident(self, incident_id: str) -> list[dict[str, Any]]:
        rows = self.conn.execute(
            "SELECT * FROM hypotheses WHERE incident_id = ? ORDER BY rank ASC",
            (incident_id,),
        ).fetchall()
        return [
            {
                "hypothesis_id": row["hypothesis_id"],
                "incident_id": row["incident_id"],
                "rank": row["rank"],
                "cause_type": row["cause_type"],
                "description": row["description"],
                "total_score": row["total_score"],
                "score_breakdown": json.loads(row["score_breakdown"]),
                "supporting_events": json.loads(row["supporting_events"]),
                "contradicting_events": json.loads(row["contradicting_events"]),
                "affected_services": json.loads(row["affected_services"]),
                "suggested_checks": json.loads(row["suggested_checks"]),
                "confidence_label": row["confidence_label"],
                "is_valid": bool(row["is_valid"]),
                "invalidation_reasons": json.loads(row["invalidation_reasons"]),
            }
            for row in rows
        ]

    def save_explanation(self, incident_id: str, explanation: dict[str, Any]) -> None:
        with self.conn:
            self.conn.execute(
                """
                INSERT INTO explanations (
                    explanation_id, incident_id, summary, primary_text,
                    evidence_text, timeline_text, alternatives, actions,
                    uncertainty, model_used, guardrail_flags, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    explanation["explanation_id"],
                    incident_id,
                    explanation["summary"],
                    explanation["primary_hypothesis_text"],
                    explanation.get("evidence_narrative", ""),
                    explanation.get("timeline_narrative", ""),
                    json_dumps(explanation.get("alternative_explanations", [])),
                    json_dumps(explanation.get("suggested_actions", [])),
                    json_dumps(explanation.get("uncertainty_notes", [])),
                    explanation.get("generation_model", "template_fallback"),
                    json_dumps(explanation.get("guardrail_violations", [])),
                    to_iso(utc_now()),
                ),
            )

    def latest_explanation(self, incident_id: str) -> dict[str, Any] | None:
        row = self.conn.execute(
            "SELECT * FROM explanations WHERE incident_id = ? ORDER BY created_at DESC LIMIT 1",
            (incident_id,),
        ).fetchone()
        if row is None:
            return None
        return {
            "explanation_id": row["explanation_id"],
            "incident_id": row["incident_id"],
            "summary": row["summary"],
            "primary_hypothesis_text": row["primary_text"],
            "evidence_narrative": row["evidence_text"],
            "timeline_narrative": row["timeline_text"],
            "alternative_explanations": json.loads(row["alternatives"]),
            "suggested_actions": json.loads(row["actions"]),
            "uncertainty_notes": json.loads(row["uncertainty"]),
            "generation_model": row["model_used"],
            "guardrail_violations": json.loads(row["guardrail_flags"]),
            "created_at": row["created_at"],
        }

    def close(self) -> None:
        self.conn.close()

    def _row_to_incident(self, row: sqlite3.Row) -> IncidentRecord:
        created_at = parse_datetime(row["created_at"])
        updated_at = parse_datetime(row["updated_at"])
        start = parse_datetime(row["time_range_start"])
        end = parse_datetime(row["time_range_end"])
        if created_at is None or updated_at is None or start is None or end is None:
            raise ValueError("Stored incident has invalid timestamp")
        return IncidentRecord(
            incident_id=row["incident_id"],
            state=IncidentState(row["state"]),
            created_at=created_at,
            updated_at=updated_at,
            severity=Severity(row["severity"]),
            primary_service=row["primary_service"],
            affected_services=tuple(json.loads(row["affected_services"])),
            time_range_start=start,
            time_range_end=end,
            event_count=row["event_count"],
        )
