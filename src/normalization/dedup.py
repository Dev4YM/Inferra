from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta
from enum import Enum

from events.models import NormalizedEvent


class DedupDecision(str, Enum):
    STORE = "store"
    SUPPRESS = "suppress"


@dataclass
class DedupWindow:
    fingerprint: str
    first_event: NormalizedEvent
    last_event: NormalizedEvent
    count: int
    first_seen: datetime
    last_seen: datetime


@dataclass(frozen=True)
class DedupSummary:
    fingerprint: str
    first_event_id: str
    last_event: NormalizedEvent
    suppressed_count: int
    window_start: datetime
    window_end: datetime


class DedupTracker:
    def __init__(self, window_seconds: int = 60, max_tracked: int = 10000) -> None:
        self.window_seconds = window_seconds
        self.max_tracked = max_tracked
        self._windows: dict[str, DedupWindow] = {}

    def check(self, event: NormalizedEvent) -> DedupDecision:
        existing = self._windows.get(event.fingerprint)
        if existing is not None:
            if event.severity > existing.last_event.severity:
                self._windows[event.fingerprint] = DedupWindow(
                    fingerprint=event.fingerprint,
                    first_event=event,
                    last_event=event,
                    count=1,
                    first_seen=event.timestamp,
                    last_seen=event.timestamp,
                )
                return DedupDecision.STORE
            existing.count += 1
            existing.last_event = event
            existing.last_seen = event.timestamp
            return DedupDecision.SUPPRESS

        if len(self._windows) >= self.max_tracked:
            self._evict_oldest()
        self._windows[event.fingerprint] = DedupWindow(
            fingerprint=event.fingerprint,
            first_event=event,
            last_event=event,
            count=1,
            first_seen=event.timestamp,
            last_seen=event.timestamp,
        )
        return DedupDecision.STORE

    def expire_windows(self, now: datetime) -> list[DedupSummary]:
        cutoff = now - timedelta(seconds=self.window_seconds)
        expired: list[DedupSummary] = []
        for fp, window in list(self._windows.items()):
            if window.last_seen < cutoff:
                del self._windows[fp]
                if window.count > 1:
                    expired.append(
                        DedupSummary(
                            fingerprint=fp,
                            first_event_id=window.first_event.event_id,
                            last_event=window.last_event,
                            suppressed_count=window.count - 1,
                            window_start=window.first_seen,
                            window_end=window.last_seen,
                        )
                    )
        return expired

    def _evict_oldest(self) -> None:
        if not self._windows:
            return
        oldest = min(self._windows.values(), key=lambda w: w.last_seen)
        self._windows.pop(oldest.fingerprint, None)
