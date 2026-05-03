from __future__ import annotations

import asyncio
import json
import re
from datetime import UTC, datetime
from pathlib import Path

from collectors.base import CollectorHealth
from core.time import to_iso, utc_now
from events.models import RawEvent

SYSLOG_RE = re.compile(
    r"^(?P<month>[A-Z][a-z]{2})\s+(?P<day>\d{1,2})\s+"
    r"(?P<time>\d{2}:\d{2}:\d{2})\s+(?P<host>\S+)\s+"
    r"(?P<program>[^:\[]+)(?:\[(?P<pid>\d+)\])?:\s*(?P<message>.*)$"
)


class LinuxSyslogCollector:
    source_type = "linux_syslog"

    def __init__(
        self,
        paths: tuple[str, ...] = ("/var/log/syslog", "/var/log/messages"),
        poll_interval_seconds: float = 2.0,
        start_at_end: bool = True,
    ) -> None:
        self.paths = tuple(Path(path) for path in paths)
        self.poll_interval_seconds = poll_interval_seconds
        self.start_at_end = start_at_end
        self._running = False
        self._offsets: dict[Path, int] = {}
        self._events = 0
        self._errors = 0
        self._last_error: str | None = None
        self._last_event_at = None

    @property
    def collector_id(self) -> str:
        return "linux_syslog://local"

    async def start(self, sink: asyncio.Queue[RawEvent]) -> None:
        self._running = True
        if self.start_at_end:
            for path in self.paths:
                if path.exists():
                    self._offsets[path] = path.stat().st_size
        try:
            while self._running:
                await self.collect_once(sink)
                await asyncio.sleep(self.poll_interval_seconds)
        finally:
            self._running = False

    async def collect_once(self, sink: asyncio.Queue[RawEvent], read_to_eof: bool = True) -> int:
        emitted = 0
        for path in self.paths:
            emitted += await self._collect_path(path, sink, read_to_eof=read_to_eof)
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

    async def _collect_path(self, path: Path, sink: asyncio.Queue[RawEvent], read_to_eof: bool) -> int:
        if not path.exists():
            return 0
        emitted = 0
        try:
            size = path.stat().st_size
            offset = self._offsets.get(path, 0)
            if size < offset:
                offset = 0
            with path.open("r", encoding="utf-8", errors="replace") as handle:
                handle.seek(offset)
                while True:
                    line = handle.readline()
                    if not line:
                        break
                    offset = handle.tell()
                    raw = self._raw_event(line.rstrip("\r\n"), path, offset)
                    await sink.put(raw)
                    emitted += 1
                    self._events += 1
                    self._last_event_at = raw.collected_at
                    if not read_to_eof:
                        await asyncio.sleep(0)
            self._offsets[path] = offset
        except OSError as exc:
            self._errors += 1
            self._last_error = str(exc)
        return emitted

    def _raw_event(self, line: str, path: Path, offset: int) -> RawEvent:
        parsed = _parse_syslog_line(line, utc_now())
        payload = {
            "timestamp": parsed.get("timestamp"),
            "host": parsed.get("host"),
            "service": parsed.get("program"),
            "level": _level_from_message(parsed.get("message", line)),
            "message": parsed.get("message", line),
            "syslog": {
                "program": parsed.get("program"),
                "pid": parsed.get("pid"),
                "path": str(path),
                "raw": line,
            },
        }
        return RawEvent(
            source_type=self.source_type,
            source_id=f"linux_syslog://{path}",
            raw_payload=json.dumps(payload, sort_keys=True),
            collected_at=utc_now(),
            metadata={
                "path": str(path),
                "raw_offset": offset,
                "program": parsed.get("program"),
                "host": parsed.get("host"),
            },
        )


def _parse_syslog_line(line: str, now: datetime) -> dict[str, str | None]:
    match = SYSLOG_RE.match(line)
    if not match:
        return {"timestamp": None, "host": None, "program": "syslog", "pid": None, "message": line}
    stamp = f"{now.year} {match.group('month')} {match.group('day')} {match.group('time')}"
    parsed = datetime.strptime(stamp, "%Y %b %d %H:%M:%S").replace(tzinfo=UTC)
    if parsed > now.replace(tzinfo=UTC):
        parsed = parsed.replace(year=parsed.year - 1)
    return {
        "timestamp": to_iso(parsed),
        "host": match.group("host"),
        "program": match.group("program"),
        "pid": match.group("pid"),
        "message": match.group("message"),
    }


def _level_from_message(message: str) -> str:
    lower = message.lower()
    if any(token in lower for token in ("critical", "fatal", "panic", "oom")):
        return "critical"
    if any(token in lower for token in ("error", "failed", "failure")):
        return "error"
    if any(token in lower for token in ("warn", "degraded")):
        return "warn"
    if "debug" in lower:
        return "debug"
    return "info"
