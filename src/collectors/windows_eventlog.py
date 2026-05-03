from __future__ import annotations

import asyncio
import json
import platform
from typing import Protocol

from collectors.base import CollectorHealth
from core.time import to_iso
from core.time import utc_now
from events.models import RawEvent


class CollectorStateStore(Protocol):
    def get_collector_state(self, collector_id: str, state_key: str) -> str | None: ...

    def set_collector_state(self, collector_id: str, state_key: str, state_value: str) -> None: ...


class WindowsEventLogCollector:
    source_type = "windows_eventlog"

    def __init__(
        self,
        channels: tuple[str, ...] = ("Application", "System"),
        poll_interval_seconds: float = 5.0,
        state_store: CollectorStateStore | None = None,
    ) -> None:
        self.channels = channels
        self.poll_interval_seconds = poll_interval_seconds
        self.state_store = state_store
        self._running = False
        self._events = 0
        self._errors = 0
        self._last_error: str | None = None
        self._last_event_at = None
        self._record_numbers: dict[str, int] = {}

    @property
    def collector_id(self) -> str:
        return "windows_eventlog://local"

    async def start(self, sink: asyncio.Queue[RawEvent]) -> None:
        self._running = True
        try:
            while self._running:
                await self.collect_once(sink)
                await asyncio.sleep(self.poll_interval_seconds)
        finally:
            self._running = False

    async def collect_once(self, sink: asyncio.Queue[RawEvent]) -> int:
        if platform.system().lower() != "windows":
            return 0
        try:
            import win32evtlog  # type: ignore
            import win32evtlogutil  # type: ignore
        except ImportError:
            self._errors += 1
            self._last_error = "pywin32 is not installed; Windows Event Log collector disabled"
            return 0

        emitted = 0
        for channel in self.channels:
            try:
                handle = win32evtlog.OpenEventLog(None, channel)
                flags = win32evtlog.EVENTLOG_BACKWARDS_READ | win32evtlog.EVENTLOG_SEQUENTIAL_READ
                events = win32evtlog.ReadEventLog(handle, flags, 0) or []
                last_seen = self._last_record_number(channel)
                newest_seen = last_seen
                for event in reversed(events[:50]):
                    record_number = int(event.RecordNumber)
                    if record_number <= last_seen:
                        continue
                    message = win32evtlogutil.SafeFormatMessage(event, channel)
                    provider = str(event.SourceName)
                    generated_at = getattr(event, "TimeGenerated", None)
                    payload = {
                        "timestamp": to_iso(generated_at) if generated_at else None,
                        "level": _level_from_event_type(int(event.EventType)),
                        "service": provider,
                        "host": str(getattr(event, "ComputerName", "") or ""),
                        "message": message,
                        "windows_eventlog": {
                            "channel": channel,
                            "record_number": record_number,
                            "provider": provider,
                            "event_id": int(event.EventID),
                            "event_type": int(event.EventType),
                            "event_category": int(getattr(event, "EventCategory", 0) or 0),
                            "computer_name": str(getattr(event, "ComputerName", "") or ""),
                        },
                    }
                    raw = RawEvent(
                        source_type=self.source_type,
                        source_id=f"windows_eventlog://{channel}",
                        raw_payload=json.dumps(payload, sort_keys=True),
                        collected_at=utc_now(),
                        metadata={
                            "channel": channel,
                            "record_number": record_number,
                            "provider": provider,
                            "event_id": int(event.EventID),
                            "event_type": int(event.EventType),
                            "computer_name": str(getattr(event, "ComputerName", "") or ""),
                        },
                    )
                    await sink.put(raw)
                    self._events += 1
                    emitted += 1
                    self._last_event_at = raw.collected_at
                    newest_seen = max(newest_seen, record_number)
                if newest_seen:
                    self._set_last_record_number(channel, newest_seen)
            except Exception as exc:  # pragma: no cover
                self._errors += 1
                self._last_error = f"{channel}: {exc}"
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

    def _last_record_number(self, channel: str) -> int:
        if channel in self._record_numbers:
            return self._record_numbers[channel]
        if self.state_store is not None:
            stored = self.state_store.get_collector_state(self.collector_id, f"{channel}.record_number")
            if stored:
                try:
                    self._record_numbers[channel] = int(stored)
                    return self._record_numbers[channel]
                except ValueError:
                    pass
        return 0

    def _set_last_record_number(self, channel: str, record_number: int) -> None:
        self._record_numbers[channel] = record_number
        if self.state_store is not None:
            self.state_store.set_collector_state(self.collector_id, f"{channel}.record_number", str(record_number))


def _level_from_event_type(event_type: int) -> str:
    if event_type == 1:
        return "error"
    if event_type in {2, 16}:
        return "warn"
    return "info"
