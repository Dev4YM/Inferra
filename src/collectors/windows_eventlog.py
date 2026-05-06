from __future__ import annotations

import importlib
import json
import platform
from datetime import datetime

from collectors.base import PollingCollector
from core.time import to_iso, utc_now
from events.models import RawEvent


class WindowsEventLogCollector(PollingCollector):
    source_type = "windows_eventlog"

    def __init__(
        self,
        channels: tuple[str, ...] = ("Application", "System"),
        poll_interval_seconds: float = 5.0,
        state_store=None,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(
            poll_interval_seconds=poll_interval_seconds,
            state_store=state_store,
            emit_timeout_seconds=emit_timeout_seconds,
        )
        self.channels = channels
        self._record_numbers: dict[str, int] = {}

    @property
    def collector_id(self) -> str:
        return "windows_eventlog://local"

    async def collect_once(self, sink) -> int:
        if platform.system().lower() != "windows":
            return 0
        try:
            win32evtlog, win32evtlogutil = _import_win32_modules()
        except ImportError:
            self._record_error("pywin32 is not installed; Windows Event Log collector disabled")
            return 0

        emitted = 0
        observed_at = utc_now()
        for channel in self.channels:
            try:
                handle = win32evtlog.OpenEventLog(None, channel)
                flags = win32evtlog.EVENTLOG_BACKWARDS_READ | win32evtlog.EVENTLOG_SEQUENTIAL_READ
                records = list(win32evtlog.ReadEventLog(handle, flags, 0) or [])
                if not records:
                    continue
                last_seen = self._last_record_number(channel)
                newest_available = max(int(record.RecordNumber) for record in records)
                if newest_available < last_seen:
                    last_seen = 0
                    self._set_last_record_number(channel, 0)
                newest_seen = last_seen
                for event in reversed(records):
                    record_number = int(event.RecordNumber)
                    if record_number <= last_seen:
                        continue
                    raw = self._raw_event(channel, event, win32evtlogutil, observed_at)
                    emitted += 1 if await self.emit(sink, raw) else 0
                    newest_seen = max(newest_seen, record_number)
                if newest_seen:
                    self._set_last_record_number(channel, newest_seen)
            except Exception as exc:  # pragma: no cover
                self._record_error(f"{channel}: {exc}")
        return emitted

    def _raw_event(self, channel: str, event, win32evtlogutil, observed_at: datetime) -> RawEvent:
        message = win32evtlogutil.SafeFormatMessage(event, channel)
        provider = str(getattr(event, "SourceName", "") or "")
        generated_at = getattr(event, "TimeGenerated", None)
        event_type = int(getattr(event, "EventType", 0) or 0)
        event_id = int(getattr(event, "EventID", 0) or 0) & 0xFFFF
        record_number = int(getattr(event, "RecordNumber", 0) or 0)
        computer_name = str(getattr(event, "ComputerName", "") or "")
        payload = {
            "timestamp": to_iso(generated_at) if generated_at else None,
            "level": _level_from_event_type(event_type),
            "service": provider or channel,
            "host": computer_name,
            "message": message,
            "windows_eventlog": {
                "channel": channel,
                "record_number": record_number,
                "provider": provider,
                "level": _level_from_event_type(event_type),
                "opcode": _optional_int(getattr(event, "EventCategory", None)),
                "keywords": _keywords_from_event(event),
                "event_id": event_id,
                "event_type": event_type,
                "event_category": _optional_int(getattr(event, "EventCategory", None)),
                "computer_name": computer_name,
            },
        }
        return RawEvent(
            source_type=self.source_type,
            source_id=f"windows_eventlog://{channel}",
            raw_payload=json.dumps(payload, sort_keys=True),
            collected_at=observed_at,
            metadata={
                "channel": channel,
                "record_number": record_number,
                "provider": provider,
                "level": _level_from_event_type(event_type),
                "opcode": _optional_int(getattr(event, "EventCategory", None)),
                "keywords": tuple(_keywords_from_event(event)),
                "event_id": event_id,
                "event_type": event_type,
                "computer_name": computer_name,
            },
        )

    def _last_record_number(self, channel: str) -> int:
        if channel in self._record_numbers:
            return self._record_numbers[channel]
        stored = self.checkpoint_load(f"{channel}.record_number")
        if stored:
            try:
                self._record_numbers[channel] = int(stored)
                return self._record_numbers[channel]
            except ValueError:
                return 0
        return 0

    def _set_last_record_number(self, channel: str, record_number: int) -> None:
        self._record_numbers[channel] = record_number
        self.checkpoint_save(f"{channel}.record_number", str(record_number))


def _import_win32_modules():
    return importlib.import_module("win32evtlog"), importlib.import_module("win32evtlogutil")


def _level_from_event_type(event_type: int) -> str:
    if event_type == 1:
        return "error"
    if event_type in {2, 16}:
        return "warn"
    return "info"


def _keywords_from_event(event) -> list[str]:
    values = getattr(event, "StringInserts", None)
    if not values:
        return []
    return [str(item) for item in values if item]


def _optional_int(value: object) -> int | None:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None
