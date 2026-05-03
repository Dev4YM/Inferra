from __future__ import annotations

import asyncio
from pathlib import Path

from collectors.base import CollectorHealth
from core.time import utc_now
from events.models import RawEvent


class FileCollector:
    source_type = "file"

    def __init__(
        self,
        path: str | Path,
        service_id: str | None = None,
        poll_interval_seconds: float = 1.0,
        start_at_end: bool = False,
    ) -> None:
        self.path = Path(path)
        self.service_id = service_id
        self.poll_interval_seconds = poll_interval_seconds
        self.start_at_end = start_at_end
        self._running = False
        self._offset = 0
        self._events = 0
        self._errors = 0
        self._last_error: str | None = None
        self._last_event_at = None

    @property
    def collector_id(self) -> str:
        return f"file://{self.path}"

    async def start(self, sink: asyncio.Queue[RawEvent]) -> None:
        self._running = True
        if self.start_at_end and self.path.exists():
            self._offset = self.path.stat().st_size
        try:
            while self._running:
                await self._poll_once(sink)
                await asyncio.sleep(self.poll_interval_seconds)
        finally:
            self._running = False

    async def collect_existing(self, sink: asyncio.Queue[RawEvent]) -> int:
        before = self._events
        await self._poll_once(sink, read_to_eof=True)
        return self._events - before

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

    async def _poll_once(self, sink: asyncio.Queue[RawEvent], read_to_eof: bool = False) -> None:
        if not self.path.exists():
            return
        try:
            size = self.path.stat().st_size
            if size < self._offset:
                self._offset = 0
            with self.path.open("r", encoding="utf-8", errors="replace") as handle:
                handle.seek(self._offset)
                while True:
                    line = handle.readline()
                    if not line:
                        break
                    self._offset = handle.tell()
                    event = RawEvent(
                        source_type=self.source_type,
                        source_id=self.collector_id,
                        raw_payload=line.rstrip("\r\n"),
                        collected_at=utc_now(),
                        metadata={"path": str(self.path), "raw_offset": self._offset, "service_id": self.service_id},
                    )
                    await sink.put(event)
                    self._events += 1
                    self._last_event_at = event.collected_at
                    if not read_to_eof:
                        await asyncio.sleep(0)
        except OSError as exc:
            self._errors += 1
            self._last_error = str(exc)
