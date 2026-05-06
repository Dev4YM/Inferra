from __future__ import annotations

import json
import sqlite3
import threading
from dataclasses import asdict
from datetime import timedelta
from pathlib import Path
from typing import TYPE_CHECKING, Any, Protocol
from uuid import uuid4

from core.enums import CauseType, IncidentState, InferenceEdgeType, Severity
from core.logging import get_logger
from core.models import (
    ContainerContext,
    DiskUsage,
    ExplanationResult,
    Incident,
    IncidentAiTrace,
    IncidentChatMessage,
    IncidentStateLogEntry,
    InferenceEdge,
    InferenceGraph,
    InferenceNode,
    ResolutionInfo,
    ResourceSummary,
    RuntimeContext,
    ScoredHypothesis,
    ScoreBreakdown,
    ServiceContext,
    TopologyEdge,
    TopologySnapshot,
)
from core.time import parse_datetime, to_iso, utc_now
from events.serialization import json_dumps

from .connection import SqliteConnectionPool, connect_sqlite, transaction
from .migrations import CURRENT_SCHEMA_VERSION, migrate

if TYPE_CHECKING:
    from analysis.models import EventCluster

_log = get_logger(__name__)

CHAT_MESSAGE_SCHEMA_VERSION = 1

_VALID_INCIDENT_TRANSITIONS: dict[str, frozenset[str]] = {
    IncidentState.OPEN.value: frozenset(
        {
            IncidentState.INVESTIGATING.value,
            IncidentState.RESOLVED.value,
            IncidentState.MERGED.value,
        }
    ),
    IncidentState.INVESTIGATING.value: frozenset(
        {
            IncidentState.EXPLAINED.value,
            IncidentState.INVESTIGATING.value,
            IncidentState.RESOLVED.value,
            IncidentState.MERGED.value,
        }
    ),
    IncidentState.EXPLAINED.value: frozenset(
        {
            IncidentState.INVESTIGATING.value,
            IncidentState.RESOLVED.value,
            IncidentState.MERGED.value,
        }
    ),
    IncidentState.RESOLVED.value: frozenset(),
    IncidentState.STALE.value: frozenset(),
    IncidentState.MERGED.value: frozenset(),
    IncidentState.ARCHIVED.value: frozenset(),
}


class IncidentStore(Protocol):
    path: Path

    def create_incident(self, incident: Incident) -> Incident: ...

    def get_incident(self, incident_id: str) -> Incident | None: ...

    def update_incident(self, incident: Incident) -> Incident: ...

    def list_incidents(
        self,
        state: list[IncidentState] | None = None,
        limit: int = 50,
        offset: int = 0,
    ) -> list[Incident]: ...

    def add_events_to_incident(self, incident_id: str, event_ids: list[str]) -> bool: ...

    def add_hypotheses(self, incident_id: str, hypotheses: list[ScoredHypothesis]) -> None: ...

    def get_hypotheses(self, incident_id: str) -> list[ScoredHypothesis]: ...

    def add_explanation(self, explanation: ExplanationResult) -> None: ...

    def get_latest_explanation(self, incident_id: str) -> ExplanationResult | None: ...

    def get_cached_explanation(
        self, incident_id: str, hypotheses_hash: str, events_hash_head: str
    ) -> ExplanationResult | None: ...

    def save_inference_graph(self, incident_id: str, graph: InferenceGraph) -> None: ...

    def get_inference_graph(self, incident_id: str) -> InferenceGraph | None: ...

    def save_cluster(self, incident_id: str, cluster: EventCluster) -> None: ...

    def get_clusters(self, incident_id: str) -> list[dict[str, Any]]: ...

    def resolve_incident(self, incident_id: str, resolution: ResolutionInfo) -> None: ...

    def archive_old_incidents(self, archive_after_days: int) -> int: ...

    def record_state_log(self, incident_id: str, old_state: str, new_state: str, reason: str) -> None: ...

    def transition_state(self, incident_id: str, new_state: IncidentState, reason: str) -> Incident: ...

    def merge_incidents(self, merged_canonical: Incident, absorbed_id: str, reason: str) -> None: ...

    def list_state_log(self, incident_id: str) -> list[IncidentStateLogEntry]: ...

    def add_ai_trace(self, trace: IncidentAiTrace) -> None: ...

    def get_latest_ai_trace(self, incident_id: str, trace_kind: str) -> IncidentAiTrace | None: ...

    def append_chat_message(
        self, incident_id: str, role: str, content: str, *, schema_version: int = CHAT_MESSAGE_SCHEMA_VERSION
    ) -> str: ...

    def list_chat_messages(self, incident_id: str) -> list[IncidentChatMessage]: ...

    def close(self) -> None: ...


class SqliteIncidentStore(IncidentStore):
    def __init__(
        self,
        path: str | Path,
        *,
        wal_mode: bool = True,
        mmap_size_bytes: int = 0,
        archive_after_days: int = 7,
        archive_interval_seconds: int = 3600,
        start_archiver: bool = False,
    ) -> None:
        self.path = Path(path)
        self._archive_after_days = archive_after_days
        self._archive_interval_seconds = max(60, int(archive_interval_seconds))

        migrate(self.path)

        self._pool = SqliteConnectionPool(
            self.path,
            wal_mode=wal_mode,
            mmap_size_bytes=mmap_size_bytes,
        )
        self._write_lock = threading.RLock()
        self._archiver_stop = threading.Event()
        self._archiver: threading.Thread | None = None
        if start_archiver:
            self._start_archiver()

    def close(self) -> None:
        self._archiver_stop.set()
        if self._archiver and self._archiver.is_alive():
            self._archiver.join(timeout=2.0)
        self._pool.close()

    def create_incident(self, incident: Incident) -> Incident:
        self._upsert_incident_row(incident, replace_events=True)
        if incident.inference_graph is not None:
            self.save_inference_graph(incident.incident_id, incident.inference_graph)
        return self.get_incident(incident.incident_id) or incident

    def get_incident(self, incident_id: str) -> Incident | None:
        row = self._pool.reader().execute("SELECT * FROM incidents WHERE incident_id = ?", (incident_id,)).fetchone()
        if row is None:
            return None
        return self._row_to_incident(row)

    def update_incident(self, incident: Incident) -> Incident:
        self._upsert_incident_row(incident, replace_events=True)
        if incident.inference_graph is not None:
            self.save_inference_graph(incident.incident_id, incident.inference_graph)
        return self.get_incident(incident.incident_id) or incident

    def list_incidents(
        self,
        state: list[IncidentState] | None = None,
        limit: int = 50,
        offset: int = 0,
    ) -> list[Incident]:
        query = "SELECT * FROM incidents"
        params: list[Any] = []
        if state:
            placeholders = ",".join("?" for _ in state)
            query += f" WHERE state IN ({placeholders})"
            params.extend(item.value for item in state)
        query += " ORDER BY severity DESC, updated_at DESC LIMIT ? OFFSET ?"
        params.extend([limit, offset])
        rows = self._pool.reader().execute(query, tuple(params)).fetchall()
        return [self._row_to_incident(row) for row in rows]

    def add_events_to_incident(self, incident_id: str, event_ids: list[str]) -> bool:
        if not event_ids:
            return self.get_incident(incident_id) is not None
        incident = self.get_incident(incident_id)
        if incident is None:
            return False
        unique_event_ids = list(dict.fromkeys([*incident.events, *event_ids]))
        conn = self._pool.writer()
        now = to_iso(utc_now())
        with self._write_lock, transaction(conn):
            conn.executemany(
                "INSERT OR IGNORE INTO incident_events(incident_id, event_id, added_at) VALUES (?, ?, ?)",
                [(incident_id, event_id, now) for event_id in event_ids],
            )
            conn.execute(
                "UPDATE incidents SET event_count = ?, updated_at = ? WHERE incident_id = ?",
                (len(unique_event_ids), now, incident_id),
            )
        return True

    def add_hypotheses(self, incident_id: str, hypotheses: list[ScoredHypothesis]) -> None:
        conn = self._pool.writer()
        now = to_iso(utc_now())
        with self._write_lock, transaction(conn):
            conn.execute("DELETE FROM hypotheses WHERE incident_id = ?", (incident_id,))
            conn.executemany(
                """
                INSERT INTO hypotheses (
                    hypothesis_id, incident_id, rank, cause_type, description,
                    total_score, score_breakdown, supporting_events,
                    contradicting_events, affected_services, suggested_checks,
                    confidence_label, is_valid, invalidation_reasons,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                [self._hypothesis_to_row(incident_id, hypothesis, now) for hypothesis in hypotheses],
            )

    def get_hypotheses(self, incident_id: str) -> list[ScoredHypothesis]:
        rows = self._pool.reader().execute(
            "SELECT * FROM hypotheses WHERE incident_id = ? ORDER BY rank ASC, total_score DESC",
            (incident_id,),
        ).fetchall()
        return [self._row_to_hypothesis(row) for row in rows]

    def add_explanation(self, explanation: ExplanationResult) -> None:
        conn = self._pool.writer()
        created_at = to_iso(utc_now())
        explanation_id = explanation.explanation_id or f"exp-{uuid4().hex}"
        with self._write_lock, transaction(conn):
            conn.execute(
                """
                INSERT INTO explanations (
                    explanation_id, incident_id, summary, primary_text,
                    evidence_text, timeline_text, alternatives, actions,
                    uncertainty, model_used, guardrail_flags, created_at,
                    explanation_schema_version, hypotheses_hash, events_hash_head, quality
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    explanation_id,
                    explanation.incident_id,
                    explanation.summary,
                    explanation.primary_hypothesis_text,
                    explanation.evidence_narrative,
                    explanation.timeline_narrative,
                    json_dumps(explanation.alternative_explanations),
                    json_dumps(explanation.suggested_actions),
                    json_dumps(explanation.uncertainty_notes),
                    explanation.generation_model,
                    json_dumps(explanation.guardrail_violations),
                    created_at,
                    int(explanation.schema_version),
                    explanation.hypotheses_hash,
                    explanation.events_hash_head,
                    explanation.quality,
                ),
            )

    def get_latest_explanation(self, incident_id: str) -> ExplanationResult | None:
        row = self._pool.reader().execute(
            "SELECT * FROM explanations WHERE incident_id = ? ORDER BY created_at DESC LIMIT 1",
            (incident_id,),
        ).fetchone()
        return self._row_to_explanation(row) if row is not None else None

    def get_cached_explanation(
        self, incident_id: str, hypotheses_hash: str, events_hash_head: str
    ) -> ExplanationResult | None:
        if not hypotheses_hash or not events_hash_head:
            return None
        row = self._pool.reader().execute(
            """
            SELECT * FROM explanations
            WHERE incident_id = ?
              AND hypotheses_hash = ?
              AND events_hash_head = ?
            ORDER BY created_at DESC
            LIMIT 1
            """,
            (incident_id, hypotheses_hash, events_hash_head),
        ).fetchone()
        return self._row_to_explanation(row) if row is not None else None

    def add_ai_trace(self, trace: IncidentAiTrace) -> None:
        conn = self._pool.writer()
        created_at = trace.created_at or to_iso(utc_now())
        with self._write_lock, transaction(conn):
            conn.execute(
                """
                INSERT INTO incident_ai_traces (
                    trace_id, incident_id, trace_kind,
                    sanitized_system_prompt, sanitized_user_prompt,
                    allowed_fields, blocked_fields, raw_logs_sent,
                    trace_schema_version, created_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    trace.trace_id,
                    trace.incident_id,
                    trace.trace_kind,
                    trace.sanitized_system_prompt,
                    trace.sanitized_user_prompt,
                    json_dumps(list(trace.allowed_fields)),
                    json_dumps(list(trace.blocked_fields)),
                    1 if trace.raw_logs_sent else 0,
                    int(trace.schema_version),
                    created_at,
                ),
            )

    def get_latest_ai_trace(self, incident_id: str, trace_kind: str) -> IncidentAiTrace | None:
        row = self._pool.reader().execute(
            """
            SELECT * FROM incident_ai_traces
            WHERE incident_id = ? AND trace_kind = ?
            ORDER BY created_at DESC
            LIMIT 1
            """,
            (incident_id, trace_kind),
        ).fetchone()
        return self._row_to_ai_trace(row) if row is not None else None

    def append_chat_message(
        self,
        incident_id: str,
        role: str,
        content: str,
        *,
        schema_version: int = CHAT_MESSAGE_SCHEMA_VERSION,
    ) -> str:
        conn = self._pool.writer()
        message_id = f"msg-{uuid4().hex}"
        created_at = to_iso(utc_now())
        with self._write_lock, transaction(conn):
            conn.execute(
                """
                INSERT INTO incident_chat_messages (
                    message_id, incident_id, role, content, message_schema_version, created_at
                ) VALUES (?, ?, ?, ?, ?, ?)
                """,
                (message_id, incident_id, role, content, int(schema_version), created_at),
            )
        return message_id

    def list_chat_messages(self, incident_id: str) -> list[IncidentChatMessage]:
        rows = self._pool.reader().execute(
            """
            SELECT message_id, incident_id, role, content, message_schema_version, created_at
            FROM incident_chat_messages
            WHERE incident_id = ?
            ORDER BY created_at ASC
            """,
            (incident_id,),
        ).fetchall()
        return [self._row_to_chat_message(row) for row in rows]

    def save_inference_graph(self, incident_id: str, graph: InferenceGraph) -> None:
        conn = self._pool.writer()
        graph_data = json_dumps(self._graph_to_dict(graph))
        with self._write_lock, transaction(conn):
            conn.execute(
                """
                INSERT INTO inference_graph_snapshots(incident_id, graph_data, created_at, event_count)
                VALUES (?, ?, ?, ?)
                ON CONFLICT(incident_id) DO UPDATE SET
                    graph_data = excluded.graph_data,
                    created_at = excluded.created_at,
                    event_count = excluded.event_count
                """,
                (incident_id, graph_data, to_iso(utc_now()), len(graph.nodes)),
            )

    def get_inference_graph(self, incident_id: str) -> InferenceGraph | None:
        row = self._pool.reader().execute(
            "SELECT graph_data FROM inference_graph_snapshots WHERE incident_id = ?",
            (incident_id,),
        ).fetchone()
        if row is None:
            return None
        return self._graph_from_dict(self._json_loads(row["graph_data"], default={}))

    def save_cluster(self, incident_id: str, cluster: EventCluster) -> None:
        conn = self._pool.writer()
        with self._write_lock, transaction(conn):
            conn.execute(
                """
                INSERT INTO incident_clusters(incident_id, cluster_id, cluster_data)
                VALUES (?, ?, ?)
                ON CONFLICT(incident_id, cluster_id) DO UPDATE SET
                    cluster_data = excluded.cluster_data
                """,
                (incident_id, cluster.cluster_id, json_dumps(self._cluster_to_dict(cluster))),
            )

    def get_clusters(self, incident_id: str) -> list[dict[str, Any]]:
        rows = self._pool.reader().execute(
            "SELECT cluster_data FROM incident_clusters WHERE incident_id = ? ORDER BY cluster_id",
            (incident_id,),
        ).fetchall()
        return [self._json_loads(row["cluster_data"], default={}) for row in rows]

    def resolve_incident(self, incident_id: str, resolution: ResolutionInfo) -> None:
        conn = self._pool.writer()
        now = to_iso(resolution.resolved_at)
        with self._write_lock, transaction(conn):
            old_row = conn.execute(
                "SELECT state FROM incidents WHERE incident_id = ?", (incident_id,)
            ).fetchone()
            old_state = old_row["state"] if old_row else "unknown"

            conn.execute(
                """
                UPDATE incidents
                SET state = ?, updated_at = ?, resolution_info = ?
                WHERE incident_id = ?
                """,
                (
                    IncidentState.RESOLVED.value,
                    now,
                    json_dumps(self._resolution_to_dict(resolution)),
                    incident_id,
                ),
            )
            conn.execute("DELETE FROM inference_graph_snapshots WHERE incident_id = ?", (incident_id,))

            self._log_state_change(
                conn, incident_id, old_state, IncidentState.RESOLVED.value, "resolved by operator"
            )

    def archive_old_incidents(self, archive_after_days: int) -> int:
        cutoff = utc_now() - timedelta(days=archive_after_days)
        rows = self._pool.reader().execute(
            """
            SELECT incident_id FROM incidents
            WHERE state IN (?, ?, ?) AND updated_at < ?
            ORDER BY updated_at ASC
            """,
            (
                IncidentState.RESOLVED.value,
                IncidentState.STALE.value,
                IncidentState.MERGED.value,
                to_iso(cutoff),
            ),
        ).fetchall()
        incident_ids = [row["incident_id"] for row in rows]
        if not incident_ids:
            return 0

        archive_dir = self.path.parent / "archive"
        archive_dir.mkdir(parents=True, exist_ok=True)
        archive_path = archive_dir / f"incidents_{utc_now():%Y%m%d}.db"

        migrate(archive_path)

        archive_conn = connect_sqlite(archive_path)
        try:
            with self._write_lock, transaction(self._pool.writer()), transaction(archive_conn):
                writer = self._pool.writer()
                for incident_id in incident_ids:
                    self._copy_incident_to_archive(writer, archive_conn, incident_id)
                    self._delete_incident(writer, incident_id)
        finally:
            archive_conn.close()

        _log.info("Archived incidents", extra={"count": len(incident_ids), "archive": str(archive_path)})
        return len(incident_ids)

    def record_state_log(self, incident_id: str, old_state: str, new_state: str, reason: str) -> None:
        conn = self._pool.writer()
        with self._write_lock, transaction(conn):
            self._log_state_change(conn, incident_id, old_state, new_state, reason)

    def transition_state(self, incident_id: str, new_state: IncidentState, reason: str) -> Incident:
        incident = self.get_incident(incident_id)
        if incident is None:
            raise ValueError(f"Unknown incident_id={incident_id}")
        old_value = incident.state.value
        new_value = new_state.value
        if old_value == new_value:
            return incident
        allowed = _VALID_INCIDENT_TRANSITIONS.get(old_value, frozenset())
        if new_value not in allowed:
            raise ValueError(f"Invalid incident state transition {old_value} -> {new_value}")
        conn = self._pool.writer()
        now = utc_now()
        with self._write_lock, transaction(conn):
            conn.execute(
                "UPDATE incidents SET state = ?, updated_at = ? WHERE incident_id = ?",
                (new_value, to_iso(now), incident_id),
            )
            self._log_state_change(conn, incident_id, old_value, new_value, reason)
        return self.get_incident(incident_id) or incident

    def merge_incidents(self, merged_canonical: Incident, absorbed_id: str, reason: str) -> None:
        if self.get_incident(merged_canonical.incident_id) is None or self.get_incident(absorbed_id) is None:
            raise ValueError("merge_incidents requires two existing incidents")
        if absorbed_id == merged_canonical.incident_id:
            raise ValueError("merge_incidents absorbed_id must differ from canonical incident_id")
        conn = self._pool.writer()
        with self._write_lock, transaction(conn):
            rows = conn.execute(
                "SELECT cluster_id, cluster_data FROM incident_clusters WHERE incident_id = ?",
                (absorbed_id,),
            ).fetchall()
            for row in rows:
                conn.execute(
                    """
                    INSERT INTO incident_clusters(incident_id, cluster_id, cluster_data)
                    VALUES (?, ?, ?)
                    ON CONFLICT(incident_id, cluster_id) DO UPDATE SET cluster_data = excluded.cluster_data
                    """,
                    (merged_canonical.incident_id, row["cluster_id"], row["cluster_data"]),
                )
        self.update_incident(merged_canonical)
        self.transition_state(
            absorbed_id,
            IncidentState.MERGED,
            reason,
        )
        conn2 = self._pool.writer()
        with self._write_lock, transaction(conn2):
            conn2.execute(
                "UPDATE incidents SET resolution_info = ? WHERE incident_id = ?",
                (json_dumps({"merged_into": merged_canonical.incident_id}), absorbed_id),
            )

    def list_state_log(self, incident_id: str) -> list[IncidentStateLogEntry]:
        rows = self._pool.reader().execute(
            "SELECT log_id, incident_id, old_state, new_state, changed_at, reason "
            "FROM incident_state_log WHERE incident_id = ? ORDER BY log_id ASC",
            (incident_id,),
        ).fetchall()
        entries: list[IncidentStateLogEntry] = []
        for row in rows:
            changed_at = parse_datetime(row["changed_at"])
            if changed_at is None:
                raise ValueError("Stored state log has invalid timestamp")
            entries.append(
                IncidentStateLogEntry(
                    log_id=int(row["log_id"]),
                    incident_id=row["incident_id"],
                    old_state=row["old_state"],
                    new_state=row["new_state"],
                    changed_at=changed_at,
                    reason=row["reason"] or "",
                )
            )
        return entries

    # -- internal helpers --

    def _upsert_incident_row(self, incident: Incident, replace_events: bool) -> None:
        conn = self._pool.writer()
        now = to_iso(utc_now())
        with self._write_lock, transaction(conn):
            conn.execute(
                """
                INSERT INTO incidents (
                    incident_id, state, created_at, updated_at, severity,
                    primary_service, affected_services, time_range_start,
                    time_range_end, event_count, schema_version, cluster_ids,
                    runtime_context, resolution_info
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(incident_id) DO UPDATE SET
                    state = excluded.state,
                    updated_at = excluded.updated_at,
                    severity = excluded.severity,
                    primary_service = excluded.primary_service,
                    affected_services = excluded.affected_services,
                    time_range_start = excluded.time_range_start,
                    time_range_end = excluded.time_range_end,
                    event_count = excluded.event_count,
                    cluster_ids = excluded.cluster_ids,
                    runtime_context = excluded.runtime_context,
                    resolution_info = excluded.resolution_info
                """,
                (
                    incident.incident_id,
                    incident.state.value,
                    to_iso(incident.created_at),
                    now if incident.updated_at is None else to_iso(incident.updated_at),
                    int(incident.severity),
                    incident.primary_service,
                    json_dumps(sorted(incident.affected_services)),
                    to_iso(incident.time_range[0]),
                    to_iso(incident.time_range[1]),
                    len(incident.events),
                    CURRENT_SCHEMA_VERSION,
                    json_dumps(list(incident.clusters)),
                    json_dumps(self._runtime_context_to_dict(incident.runtime_context))
                    if incident.runtime_context is not None
                    else None,
                    None,
                ),
            )
            if replace_events:
                conn.execute("DELETE FROM incident_events WHERE incident_id = ?", (incident.incident_id,))
            if incident.events:
                conn.executemany(
                    "INSERT OR IGNORE INTO incident_events(incident_id, event_id, added_at) VALUES (?, ?, ?)",
                    [(incident.incident_id, event_id, now) for event_id in dict.fromkeys(incident.events)],
                )

    def _log_state_change(
        self, conn: sqlite3.Connection, incident_id: str, old_state: str, new_state: str, reason: str
    ) -> None:
        conn.execute(
            """
            INSERT INTO incident_state_log(incident_id, old_state, new_state, changed_at, reason)
            VALUES (?, ?, ?, ?, ?)
            """,
            (incident_id, old_state, new_state, to_iso(utc_now()), reason),
        )

    def _row_to_incident(self, row: sqlite3.Row) -> Incident:
        created_at = parse_datetime(row["created_at"])
        updated_at = parse_datetime(row["updated_at"])
        start = parse_datetime(row["time_range_start"])
        end = parse_datetime(row["time_range_end"])
        if created_at is None or updated_at is None or start is None or end is None:
            raise ValueError("Stored incident has invalid timestamp")
        event_rows = self._pool.reader().execute(
            "SELECT event_id FROM incident_events WHERE incident_id = ? ORDER BY added_at ASC",
            (row["incident_id"],),
        ).fetchall()
        runtime_context = None
        if row["runtime_context"]:
            runtime_context = self._runtime_context_from_dict(self._json_loads(row["runtime_context"], default={}))
        return Incident(
            incident_id=row["incident_id"],
            state=IncidentState(row["state"]),
            created_at=created_at,
            updated_at=updated_at,
            clusters=self._json_loads(row["cluster_ids"], default=[]),
            events=[event_row["event_id"] for event_row in event_rows],
            affected_services=set(self._json_loads(row["affected_services"], default=[])),
            primary_service=row["primary_service"],
            time_range=(start, end),
            severity=Severity(row["severity"]),
            runtime_context=runtime_context,
            inference_graph=self.get_inference_graph(row["incident_id"]),
        )

    def _hypothesis_to_row(
        self,
        incident_id: str,
        hypothesis: ScoredHypothesis,
        timestamp: str,
    ) -> tuple[Any, ...]:
        return (
            hypothesis.hypothesis_id,
            incident_id,
            hypothesis.rank,
            hypothesis.cause_type.value if isinstance(hypothesis.cause_type, CauseType) else str(hypothesis.cause_type),
            hypothesis.description,
            hypothesis.total_score,
            json_dumps(asdict(hypothesis.score_breakdown)),
            json_dumps(hypothesis.supporting_events),
            json_dumps(hypothesis.contradicting_events),
            json_dumps(hypothesis.affected_services),
            json_dumps(hypothesis.suggested_checks),
            hypothesis.confidence_label,
            1 if hypothesis.is_valid else 0,
            json_dumps(hypothesis.invalidation_reasons),
            timestamp,
            timestamp,
        )

    def _row_to_hypothesis(self, row: sqlite3.Row) -> ScoredHypothesis:
        return ScoredHypothesis(
            hypothesis_id=row["hypothesis_id"],
            rank=int(row["rank"] or 0),
            cause_type=CauseType(row["cause_type"]),
            description=row["description"],
            total_score=float(row["total_score"] or 0.0),
            score_breakdown=ScoreBreakdown(**self._json_loads(row["score_breakdown"], default={})),
            supporting_events=self._json_loads(row["supporting_events"], default=[]),
            contradicting_events=self._json_loads(row["contradicting_events"], default=[]),
            affected_services=self._json_loads(row["affected_services"], default=[]),
            suggested_checks=self._json_loads(row["suggested_checks"], default=[]),
            confidence_label=row["confidence_label"] or "",
            is_valid=bool(row["is_valid"]),
            invalidation_reasons=self._json_loads(row["invalidation_reasons"], default=[]),
        )

    def _row_to_explanation(self, row: sqlite3.Row) -> ExplanationResult:
        keys = row.keys()
        return ExplanationResult(
            incident_id=row["incident_id"],
            summary=row["summary"],
            primary_hypothesis_text=row["primary_text"],
            evidence_narrative=row["evidence_text"] or "",
            timeline_narrative=row["timeline_text"] or "",
            alternative_explanations=self._json_loads(row["alternatives"], default=[]),
            suggested_actions=self._json_loads(row["actions"], default=[]),
            uncertainty_notes=self._json_loads(row["uncertainty"], default=[]),
            generation_model=row["model_used"],
            guardrail_violations=self._json_loads(row["guardrail_flags"], default=[]),
            explanation_id=str(row["explanation_id"]),
            hypotheses_hash=str(row["hypotheses_hash"]) if "hypotheses_hash" in keys else "",
            events_hash_head=str(row["events_hash_head"]) if "events_hash_head" in keys else "",
            schema_version=int(row["explanation_schema_version"] if "explanation_schema_version" in keys else 1),
            quality=str(row["quality"]) if "quality" in keys and row["quality"] else "ok",
        )

    def _row_to_ai_trace(self, row: sqlite3.Row) -> IncidentAiTrace:
        allowed = self._json_loads(row["allowed_fields"], default=[])
        blocked = self._json_loads(row["blocked_fields"], default=[])
        return IncidentAiTrace(
            trace_id=row["trace_id"],
            incident_id=row["incident_id"],
            trace_kind=row["trace_kind"],
            sanitized_system_prompt=row["sanitized_system_prompt"],
            sanitized_user_prompt=row["sanitized_user_prompt"],
            allowed_fields=tuple(str(item) for item in allowed),
            blocked_fields=tuple(str(item) for item in blocked),
            raw_logs_sent=bool(row["raw_logs_sent"]),
            schema_version=int(row["trace_schema_version"] or 1),
            created_at=row["created_at"],
        )

    def _row_to_chat_message(self, row: sqlite3.Row) -> IncidentChatMessage:
        return IncidentChatMessage(
            message_id=row["message_id"],
            incident_id=row["incident_id"],
            role=row["role"],
            content=row["content"],
            schema_version=int(row["message_schema_version"] or 1),
            created_at=row["created_at"],
        )

    def _cluster_to_dict(self, cluster: EventCluster) -> dict[str, Any]:
        return {
            "cluster_id": cluster.cluster_id,
            "events": list(cluster.events),
            "time_range": [to_iso(cluster.time_range[0]), to_iso(cluster.time_range[1])],
            "affected_services": sorted(cluster.affected_services),
            "primary_severity": int(cluster.primary_severity),
            "trigger_event_id": cluster.trigger_event_id,
            "correlation_edges": [asdict(edge) for edge in cluster.correlation_edges],
            "anomaly_scores": dict(cluster.anomaly_scores),
        }

    def _graph_to_dict(self, graph: InferenceGraph) -> dict[str, Any]:
        return {
            "nodes": {
                node_id: {
                    "event_id": node.event_id,
                    "service_id": node.service_id,
                    "timestamp": to_iso(node.timestamp),
                    "severity": int(node.severity),
                    "summary": node.summary,
                    "node_type": node.node_type,
                    "in_degree": node.in_degree,
                    "out_degree": node.out_degree,
                }
                for node_id, node in graph.nodes.items()
            },
            "edges": [
                {
                    "source_event_id": edge.source_event_id,
                    "target_event_id": edge.target_event_id,
                    "edge_type": edge.edge_type.value,
                    "plausibility": edge.plausibility,
                    "latency_ms": edge.latency_ms,
                    "evidence": edge.evidence,
                    "requires": list(edge.requires),
                }
                for edge in graph.edges
            ],
            "root_candidates": list(graph.root_candidates),
            "leaf_nodes": list(graph.leaf_nodes),
        }

    def _graph_from_dict(self, data: dict[str, Any]) -> InferenceGraph:
        nodes: dict[str, InferenceNode] = {}
        for node_id, node_data in (data.get("nodes") or {}).items():
            timestamp = parse_datetime(node_data["timestamp"])
            if timestamp is None:
                raise ValueError("Stored inference node has invalid timestamp")
            nodes[node_id] = InferenceNode(
                event_id=node_data["event_id"],
                service_id=node_data["service_id"],
                timestamp=timestamp,
                severity=Severity(node_data["severity"]),
                summary=node_data["summary"],
                node_type=node_data["node_type"],
                in_degree=int(node_data.get("in_degree", 0)),
                out_degree=int(node_data.get("out_degree", 0)),
            )
        edges = [
            InferenceEdge(
                source_event_id=edge["source_event_id"],
                target_event_id=edge["target_event_id"],
                edge_type=InferenceEdgeType(edge["edge_type"]),
                plausibility=float(edge["plausibility"]),
                latency_ms=float(edge["latency_ms"]),
                evidence=edge["evidence"],
                requires=list(edge.get("requires") or []),
            )
            for edge in (data.get("edges") or [])
        ]
        return InferenceGraph(
            nodes=nodes,
            edges=edges,
            root_candidates=list(data.get("root_candidates") or []),
            leaf_nodes=list(data.get("leaf_nodes") or []),
        )

    def _runtime_context_to_dict(self, runtime_context: RuntimeContext | None) -> dict[str, Any] | None:
        if runtime_context is None:
            return None
        return {
            "captured_at": to_iso(runtime_context.captured_at),
            "incident_id": runtime_context.incident_id,
            "host_context": {
                "hostname": runtime_context.host_context.hostname,
                "os_info": runtime_context.host_context.os_info,
                "cpu_count": runtime_context.host_context.cpu_count,
                "total_memory_mb": runtime_context.host_context.total_memory_mb,
                "load_average": list(runtime_context.host_context.load_average),
                "cpu_percent": runtime_context.host_context.cpu_percent,
                "memory_used_percent": runtime_context.host_context.memory_used_percent,
                "disk_usage": {
                    key: asdict(value) for key, value in runtime_context.host_context.disk_usage.items()
                },
                "uptime_seconds": runtime_context.host_context.uptime_seconds,
                "open_file_descriptors": runtime_context.host_context.open_file_descriptors,
                "max_file_descriptors": runtime_context.host_context.max_file_descriptors,
            },
            "container_contexts": {
                key: {
                    **asdict(value),
                    "started_at": to_iso(value.started_at) if value.started_at is not None else None,
                }
                for key, value in runtime_context.container_contexts.items()
            },
            "service_contexts": {key: asdict(value) for key, value in runtime_context.service_contexts.items()},
            "topology": {
                "services": list(runtime_context.topology.services),
                "edges": [asdict(edge) for edge in runtime_context.topology.edges],
                "isolated_services": list(runtime_context.topology.isolated_services),
            },
            "resource_summary": asdict(runtime_context.resource_summary),
        }

    def _runtime_context_from_dict(self, data: dict[str, Any]) -> RuntimeContext:
        captured_at = parse_datetime(data["captured_at"])
        if captured_at is None:
            raise ValueError("Stored runtime context has invalid timestamp")
        host = data["host_context"]
        return RuntimeContext(
            captured_at=captured_at,
            incident_id=data["incident_id"],
            host_context=__import__("core.models", fromlist=["HostContext"]).HostContext(
                hostname=host["hostname"],
                os_info=host["os_info"],
                cpu_count=int(host["cpu_count"]),
                total_memory_mb=int(host["total_memory_mb"]),
                load_average=tuple(host["load_average"]),
                cpu_percent=float(host["cpu_percent"]),
                memory_used_percent=float(host["memory_used_percent"]),
                disk_usage={key: DiskUsage(**value) for key, value in (host.get("disk_usage") or {}).items()},
                uptime_seconds=float(host["uptime_seconds"]),
                open_file_descriptors=int(host["open_file_descriptors"]),
                max_file_descriptors=int(host["max_file_descriptors"]),
            ),
            container_contexts={
                key: ContainerContext(
                    container_id=value["container_id"],
                    container_name=value["container_name"],
                    image=value["image"],
                    state=value["state"],
                    started_at=parse_datetime(value["started_at"]) if value.get("started_at") else None,
                    restart_count=int(value["restart_count"]),
                    cpu_percent=float(value["cpu_percent"]),
                    memory_usage_mb=float(value["memory_usage_mb"]),
                    memory_limit_mb=float(value["memory_limit_mb"]) if value.get("memory_limit_mb") is not None else None,
                    memory_percent=float(value["memory_percent"]),
                    network_rx_bytes=int(value["network_rx_bytes"]),
                    network_tx_bytes=int(value["network_tx_bytes"]),
                    pids=int(value["pids"]),
                    health_status=value.get("health_status"),
                    labels=dict(value.get("labels") or {}),
                    environment_keys=list(value.get("environment_keys") or []),
                    ports=list(value.get("ports") or []),
                )
                for key, value in (data.get("container_contexts") or {}).items()
            },
            service_contexts={
                key: ServiceContext(
                    service_id=value["service_id"],
                    containers=list(value.get("containers") or []),
                    event_rate_current=float(value["event_rate_current"]),
                    error_rate_current=float(value["error_rate_current"]),
                    anomaly_score=float(value["anomaly_score"]),
                    last_restart=parse_datetime(value["last_restart"]) if value.get("last_restart") else None,
                    restart_count_24h=int(value["restart_count_24h"]),
                    active_connections=int(value["active_connections"]) if value.get("active_connections") is not None else None,
                    dependency_health=dict(value.get("dependency_health") or {}),
                )
                for key, value in (data.get("service_contexts") or {}).items()
            },
            topology=TopologySnapshot(
                services=list((data.get("topology") or {}).get("services") or []),
                edges=[TopologyEdge(**edge) for edge in ((data.get("topology") or {}).get("edges") or [])],
                isolated_services=list((data.get("topology") or {}).get("isolated_services") or []),
            ),
            resource_summary=ResourceSummary(**(data.get("resource_summary") or {})),
        )

    def _resolution_to_dict(self, resolution: ResolutionInfo) -> dict[str, Any]:
        return {
            "resolved_by": resolution.resolved_by,
            "correct_hypothesis_id": resolution.correct_hypothesis_id,
            "feedback_type": resolution.feedback_type,
            "notes": resolution.notes,
            "resolved_at": to_iso(resolution.resolved_at),
        }

    def _copy_incident_to_archive(
        self,
        writer: sqlite3.Connection,
        archive_conn: sqlite3.Connection,
        incident_id: str,
    ) -> None:
        incident_row = writer.execute("SELECT * FROM incidents WHERE incident_id = ?", (incident_id,)).fetchone()
        if incident_row is not None:
            archive_conn.execute(
                """
                INSERT OR REPLACE INTO incidents (
                    incident_id, state, created_at, updated_at, severity,
                    primary_service, affected_services, time_range_start,
                    time_range_end, event_count, schema_version, cluster_ids,
                    runtime_context, resolution_info
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                tuple(incident_row[key] for key in incident_row.keys()),
            )
        for table, columns in [
            ("incident_events", ("incident_id", "event_id", "added_at")),
            (
                "hypotheses",
                (
                    "hypothesis_id", "incident_id", "rank", "cause_type", "description",
                    "total_score", "score_breakdown", "supporting_events",
                    "contradicting_events", "affected_services", "suggested_checks",
                    "confidence_label", "is_valid", "invalidation_reasons",
                    "created_at", "updated_at",
                ),
            ),
            (
                "explanations",
                (
                    "explanation_id", "incident_id", "summary", "primary_text",
                    "evidence_text", "timeline_text", "alternatives", "actions",
                    "uncertainty", "model_used", "guardrail_flags", "created_at",
                    "explanation_schema_version", "hypotheses_hash", "events_hash_head",
                    "quality",
                ),
            ),
            ("inference_graph_snapshots", ("incident_id", "graph_data", "created_at", "event_count")),
            ("feedback", ("feedback_id", "incident_id", "correct_hypothesis_id", "feedback_type", "operator_notes", "resolved_at", "created_at")),
            ("incident_state_log", ("log_id", "incident_id", "old_state", "new_state", "changed_at", "reason")),
            (
                "incident_ai_traces",
                (
                    "trace_id",
                    "incident_id",
                    "trace_kind",
                    "sanitized_system_prompt",
                    "sanitized_user_prompt",
                    "allowed_fields",
                    "blocked_fields",
                    "raw_logs_sent",
                    "trace_schema_version",
                    "created_at",
                ),
            ),
            (
                "incident_chat_messages",
                (
                    "message_id",
                    "incident_id",
                    "role",
                    "content",
                    "message_schema_version",
                    "created_at",
                ),
            ),
        ]:
            rows = writer.execute(f"SELECT * FROM {table} WHERE incident_id = ?", (incident_id,)).fetchall()
            if not rows:
                continue
            placeholders = ",".join("?" for _ in columns)
            archive_conn.executemany(
                f"INSERT OR REPLACE INTO {table} ({','.join(columns)}) VALUES ({placeholders})",
                [tuple(row[column] for column in columns) for row in rows],
            )

    def _delete_incident(self, writer: sqlite3.Connection, incident_id: str) -> None:
        for table in (
            "incident_state_log", "feedback", "incident_events",
            "hypotheses", "explanations", "inference_graph_snapshots",
            "incident_ai_traces", "incident_chat_messages",
        ):
            writer.execute(f"DELETE FROM {table} WHERE incident_id = ?", (incident_id,))
        writer.execute("DELETE FROM incidents WHERE incident_id = ?", (incident_id,))

    def _json_loads(self, raw: str | None, default: Any) -> Any:
        if raw in (None, ""):
            return default
        return json.loads(raw)

    def _start_archiver(self) -> None:
        if self._archiver is not None:
            return

        def _run() -> None:
            while not self._archiver_stop.wait(self._archive_interval_seconds):
                try:
                    self.archive_old_incidents(self._archive_after_days)
                except sqlite3.Error:
                    _log.debug("Incident archiver cycle failed", exc_info=True)

        self._archiver = threading.Thread(target=_run, name="inferra-incident-archiver", daemon=True)
        self._archiver.start()
