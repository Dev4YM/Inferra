from __future__ import annotations

import asyncio
import json
import shutil
import subprocess
from datetime import UTC, datetime
from typing import Protocol

from collectors.base import CollectorHealth
from core.time import to_iso, utc_now
from events.models import RawEvent


class CollectorStateStore(Protocol):
    def get_collector_state(self, collector_id: str, state_key: str) -> str | None: ...

    def set_collector_state(self, collector_id: str, state_key: str, state_value: str) -> None: ...


class JournaldCollector:
    source_type = "linux_journald"

    def __init__(
        self,
        units: tuple[str, ...] = (),
        since: str = "-1 hour",
        limit: int = 200,
        poll_interval_seconds: float = 5.0,
        state_store: CollectorStateStore | None = None,
        command_runner=subprocess.run,
    ) -> None:
        self.units = units
        self.since = since
        self.limit = limit
        self.poll_interval_seconds = poll_interval_seconds
        self.state_store = state_store
        self.command_runner = command_runner
        self._running = False
        self._events = 0
        self._errors = 0
        self._last_error: str | None = None
        self._last_event_at = None

    @property
    def collector_id(self) -> str:
        unit_key = ",".join(self.units) if self.units else "system"
        return f"linux_journald://{unit_key}"

    async def start(self, sink: asyncio.Queue[RawEvent]) -> None:
        self._running = True
        try:
            while self._running:
                await self.collect_once(sink)
                await asyncio.sleep(self.poll_interval_seconds)
        finally:
            self._running = False

    async def collect_once(self, sink: asyncio.Queue[RawEvent]) -> int:
        if shutil.which("journalctl") is None and self.command_runner is subprocess.run:
            self._errors += 1
            self._last_error = "journalctl is not available"
            return 0
        try:
            rows = self._read_rows()
        except Exception as exc:  # pragma: no cover
            self._errors += 1
            self._last_error = str(exc)
            return 0
        emitted = 0
        newest_cursor = None
        for row in rows:
            raw = self._raw_event(row)
            await sink.put(raw)
            emitted += 1
            self._events += 1
            self._last_event_at = raw.collected_at
            newest_cursor = row.get("__CURSOR") or newest_cursor
        if newest_cursor and self.state_store is not None:
            self.state_store.set_collector_state(self.collector_id, "cursor", newest_cursor)
        return emitted

    async def stop(self) -> None:
        self._running = False

    def health_check(self) -> CollectorHealth:
        return CollectorHealth(
            collector_id=self.collector_id,
            source_type=self.source_type,
            is_running=self._running,
            events_emitted=self._events,
            last_event_at=self._last_event_at,
            error_count=self._errors,
            last_error=self._last_error,
        )

    def _read_rows(self) -> list[dict]:
        command = ["journalctl", "-o", "json", "--no-pager"]
        cursor = self.state_store.get_collector_state(self.collector_id, "cursor") if self.state_store else None
        if cursor:
            command.extend(["--after-cursor", cursor])
        else:
            command.extend(["--since", self.since, "-n", str(self.limit)])
        for unit in self.units:
            command.extend(["-u", unit])
        result = self.command_runner(command, capture_output=True, text=True, check=False)
        if result.returncode != 0:
            raise RuntimeError((result.stderr or "journalctl failed").strip())
        rows = []
        for line in result.stdout.splitlines():
            if not line.strip():
                continue
            value = json.loads(line)
            if isinstance(value, dict):
                rows.append(value)
        return rows

    def _raw_event(self, row: dict) -> RawEvent:
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
            collected_at=utc_now(),
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
