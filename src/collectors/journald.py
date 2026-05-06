from __future__ import annotations

import asyncio
import json
import shutil
import subprocess
from datetime import UTC, datetime
from typing import Any

from collectors.base import PollingCollector
from core.time import to_iso, utc_now
from events.models import RawEvent

try:
    from systemd import journal as systemd_journal  # type: ignore
except ImportError:  # pragma: no cover
    systemd_journal = None


class JournaldCollector(PollingCollector):
    source_type = "linux_journald"

    def __init__(
        self,
        units: tuple[str, ...] = (),
        exclude_units: tuple[str, ...] = (),
        min_priority: int = 6,
        since: str = "-1 hour",
        limit: int = 200,
        poll_interval_seconds: float = 5.0,
        state_store=None,
        command_runner=subprocess.run,
        journal_reader_factory=None,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(
            poll_interval_seconds=poll_interval_seconds,
            state_store=state_store,
            emit_timeout_seconds=emit_timeout_seconds,
        )
        self.units = units
        self.exclude_units = frozenset(exclude_units)
        self.min_priority = min_priority
        self.since = since
        self.limit = limit
        self.command_runner = command_runner
        self.journal_reader_factory = journal_reader_factory

    @property
    def collector_id(self) -> str:
        unit_key = ",".join(self.units) if self.units else "system"
        return f"linux_journald://{unit_key}"

    async def collect_once(self, sink) -> int:
        if systemd_journal is None and shutil.which("journalctl") is None and self.command_runner is subprocess.run:
            self._record_error("journalctl is not available")
            return 0
        try:
            rows = await asyncio.to_thread(self._read_rows)
        except Exception as exc:  # pragma: no cover
            self._record_error(exc)
            return 0
        emitted = 0
        newest_cursor = None
        observed_at = utc_now()
        for row in rows:
            raw = self._raw_event(row, observed_at)
            emitted += 1 if await self.emit(sink, raw) else 0
            newest_cursor = row.get("__CURSOR") or newest_cursor
        if newest_cursor:
            self.checkpoint_save("cursor", str(newest_cursor))
        return emitted

    def _read_rows(self) -> list[dict[str, Any]]:
        if self.journal_reader_factory is not None or systemd_journal is not None:
            rows = self._read_rows_native()
            if rows:
                return rows
        return self._read_rows_via_journalctl()

    def _read_rows_native(self) -> list[dict[str, Any]]:
        reader = self.journal_reader_factory() if self.journal_reader_factory is not None else systemd_journal.Reader()
        if hasattr(reader, "this_machine"):
            reader.this_machine()
        if hasattr(reader, "log_level"):
            reader.log_level(self.min_priority)
        for unit in self.units:
            if hasattr(reader, "add_match"):
                reader.add_match(_SYSTEMD_UNIT=unit)
        cursor = self.checkpoint_load("cursor")
        if cursor and hasattr(reader, "seek_cursor"):
            reader.seek_cursor(cursor)
            if hasattr(reader, "get_next"):
                reader.get_next()
        elif hasattr(reader, "seek_tail"):
            reader.seek_tail()
        entries: list[dict[str, Any]] = []
        iterator = reader if hasattr(reader, "__iter__") else []
        for row in iterator:
            payload = dict(row)
            if self._skip_row(payload):
                continue
            entries.append(payload)
            if len(entries) >= self.limit:
                break
        return entries

    def _read_rows_via_journalctl(self) -> list[dict[str, Any]]:
        command = ["journalctl", "-o", "json", "--no-pager", "-p", f"0..{self.min_priority}"]
        cursor = self.checkpoint_load("cursor")
        if cursor:
            command.extend(["--after-cursor", cursor])
        else:
            command.extend(["--since", self.since, "-n", str(self.limit)])
        for unit in self.units:
            command.extend(["-u", unit])
        result = self.command_runner(command, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            raise RuntimeError((result.stderr or "journalctl failed").strip())
        rows: list[dict[str, Any]] = []
        for line in result.stdout.splitlines():
            if not line.strip():
                continue
            value = json.loads(line)
            if isinstance(value, dict) and not self._skip_row(value):
                rows.append(value)
        return rows

    def _skip_row(self, row: dict[str, Any]) -> bool:
        unit = str(row.get("_SYSTEMD_UNIT") or "")
        if unit and unit in self.exclude_units:
            return True
        priority = row.get("PRIORITY")
        try:
            return int(str(priority)) > self.min_priority
        except (TypeError, ValueError):
            return False

    def _raw_event(self, row: dict[str, Any], observed_at: datetime) -> RawEvent:
        service = row.get("_SYSTEMD_UNIT") or row.get("SYSLOG_IDENTIFIER") or row.get("_COMM") or "journald"
        timestamp = _timestamp_from_journal(row.get("__REALTIME_TIMESTAMP"))
        priority = _priority_to_level(row.get("PRIORITY"))
        message = str(row.get("MESSAGE") or "<empty journald message>")
        host = row.get("_HOSTNAME")
        payload = {
            "timestamp": timestamp,
            "level": priority,
            "service": service,
            "host": host,
            "message": message,
            "journald": {
                "unit": row.get("_SYSTEMD_UNIT"),
                "identifier": row.get("SYSLOG_IDENTIFIER"),
                "pid": row.get("_PID"),
                "cursor": row.get("__CURSOR"),
                "priority": row.get("PRIORITY"),
            },
        }
        return RawEvent(
            source_type=self.source_type,
            source_id=self.collector_id,
            raw_payload=json.dumps(payload, sort_keys=True),
            collected_at=observed_at,
            metadata={
                "unit": row.get("_SYSTEMD_UNIT"),
                "identifier": row.get("SYSLOG_IDENTIFIER"),
                "cursor": row.get("__CURSOR"),
                "host": host,
            },
        )


def _timestamp_from_journal(value: object) -> str | None:
    try:
        micros = int(str(value))
    except (TypeError, ValueError):
        return None
    return to_iso(datetime.fromtimestamp(micros / 1_000_000, tz=UTC))


def _priority_to_level(value: object) -> str:
    try:
        priority = int(str(value))
    except (TypeError, ValueError):
        return "info"
    if priority <= 2:
        return "critical"
    if priority == 3:
        return "error"
    if priority == 4:
        return "warn"
    if priority >= 7:
        return "debug"
    return "info"
