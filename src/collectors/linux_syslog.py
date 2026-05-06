from __future__ import annotations

import asyncio
import json
import re
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

from collectors.base import PollingCollector
from core.time import to_iso, utc_now
from events.models import RawEvent

try:
    from inotify_simple import INotify, flags as inotify_flags
except ImportError:  # pragma: no cover
    INotify = None
    inotify_flags = None

SYSLOG_RE = re.compile(
    r"^(?P<month>[A-Z][a-z]{2})\s+(?P<day>\d{1,2})\s+"
    r"(?P<time>\d{2}:\d{2}:\d{2})\s+(?P<host>\S+)\s+"
    r"(?P<program>[^:\[]+)(?:\[(?P<pid>\d+)\])?:\s*(?P<message>.*)$"
)


@dataclass(frozen=True)
class SyslogFileIdentity:
    device: int
    inode: int


class LinuxSyslogCollector(PollingCollector):
    source_type = "linux_syslog"

    def __init__(
        self,
        paths: tuple[str, ...] = ("/var/log/syslog", "/var/log/messages"),
        poll_interval_seconds: float = 2.0,
        start_at_end: bool = True,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(poll_interval_seconds=poll_interval_seconds, emit_timeout_seconds=emit_timeout_seconds)
        self.paths = tuple(Path(path) for path in paths)
        self.start_at_end = start_at_end
        self._offsets: dict[Path, int] = {}
        self._identities: dict[Path, SyslogFileIdentity] = {}
        self._inotify = self._build_inotify()

    @property
    def collector_id(self) -> str:
        return "linux_syslog://local"

    async def run(self, queue: asyncio.Queue[RawEvent]) -> None:
        await self._mark_running()
        if self.start_at_end:
            for path in self.paths:
                if path.exists():
                    self._offsets[path] = path.stat().st_size
                    self._identities[path] = self._identity(path)
        try:
            while not self._should_stop():
                try:
                    await self.collect_once(queue)
                except asyncio.CancelledError:
                    raise
                except Exception as exc:
                    self._record_error(exc)
                if self._inotify is not None:
                    await asyncio.to_thread(self._wait_for_inotify)
                else:
                    try:
                        await asyncio.wait_for(self._stop_event.wait(), timeout=self.poll_interval_seconds)
                    except TimeoutError:
                        continue
        finally:
            await self._mark_stopped()

    async def collect_once(self, sink, read_to_eof: bool = True) -> int:
        emitted = 0
        for path in self.paths:
            emitted += await self._collect_path(path, sink, read_to_eof=read_to_eof)
        return emitted

    async def _collect_path(self, path: Path, sink, read_to_eof: bool) -> int:
        if not path.exists():
            return 0
        emitted = 0
        try:
            stat = path.stat()
            identity = SyslogFileIdentity(device=stat.st_dev, inode=stat.st_ino)
            previous_identity = self._identities.get(path)
            size = stat.st_size
            offset = self._offsets.get(path, 0)
            if previous_identity is not None and previous_identity != identity:
                offset = 0
            elif size < offset:
                offset = 0
            with path.open("r", encoding="utf-8", errors="replace") as handle:
                handle.seek(offset)
                while True:
                    line = handle.readline()
                    if not line:
                        break
                    offset = handle.tell()
                    raw = self._raw_event(line.rstrip("\r\n"), path, offset)
                    emitted += 1 if await self.emit(sink, raw) else 0
                    if not read_to_eof:
                        await asyncio.sleep(0)
            self._offsets[path] = offset
            self._identities[path] = identity
        except OSError as exc:
            self._record_error(exc)
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

    def _build_inotify(self):
        if INotify is None or inotify_flags is None:
            return None
        watcher = INotify()
        for path in self.paths:
            parent = path.parent if path.parent.exists() else None
            if parent is None:
                continue
            watcher.add_watch(
                str(parent),
                inotify_flags.MODIFY
                | inotify_flags.CREATE
                | inotify_flags.MOVED_TO
                | inotify_flags.DELETE_SELF
                | inotify_flags.MOVE_SELF,
            )
        return watcher

    def _wait_for_inotify(self) -> None:
        if self._inotify is None:
            return
        self._inotify.read(timeout=int(self.poll_interval_seconds * 1000))

    def _identity(self, path: Path) -> SyslogFileIdentity:
        stat = path.stat()
        return SyslogFileIdentity(device=stat.st_dev, inode=stat.st_ino)


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
