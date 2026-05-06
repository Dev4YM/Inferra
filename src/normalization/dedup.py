from __future__ import annotations

import threading
from collections import OrderedDict
from dataclasses import dataclass
from datetime import datetime, timedelta

from config.models import DeduplicationConfig
from core.enums import EventType, Severity
from core.ids import new_id
from core.logging import get_logger
from core.time import utc_now
from events.models import DataQuality, NormalizedEvent, SourceRef

_log = get_logger(__name__)


@dataclass
class DedupWindow:
    fingerprint: str
    first_event: NormalizedEvent
    last_event: NormalizedEvent
    count: int
    first_seen: datetime
    last_seen: datetime
    last_summary_at: datetime | None = None


@dataclass(frozen=True)
class DedupResult:
    decision: str
    event: NormalizedEvent | None = None
    summary_events: tuple[NormalizedEvent, ...] = ()


@dataclass(frozen=True)
class DedupStats:
    tracked_fingerprints: int = 0
    total_suppressed: int = 0
    total_summaries_emitted: int = 0
    evictions: int = 0


class DedupTracker:
    def __init__(self, config: DeduplicationConfig | None = None) -> None:
        self._config = config or DeduplicationConfig()
        self._windows: OrderedDict[str, DedupWindow] = OrderedDict()
        self._lock = threading.Lock()
        self._total_suppressed = 0
        self._total_summaries = 0
        self._evictions = 0

    @property
    def config(self) -> DeduplicationConfig:
        return self._config

    def check(self, event: NormalizedEvent, now: datetime | None = None) -> DedupResult:
        now = now or utc_now()
        with self._lock:
            summaries = self._flush_expired(now)
            summaries.extend(self._flush_periodic_summaries(now))

            existing = self._windows.get(event.fingerprint)
            if existing is not None:
                self._windows.move_to_end(event.fingerprint)

                if self._config.severity_escalation_splits and event.severity > existing.last_event.severity:
                    old_window = self._windows.pop(event.fingerprint)
                    if old_window.count > 1:
                        summaries.append(self._make_summary_event(old_window, now))
                    self._windows[event.fingerprint] = DedupWindow(
                        fingerprint=event.fingerprint,
                        first_event=event,
                        last_event=event,
                        count=1,
                        first_seen=event.timestamp,
                        last_seen=event.timestamp,
                    )
                    return DedupResult(
                        decision="store",
                        event=event,
                        summary_events=tuple(summaries),
                    )

                existing.count += 1
                existing.last_event = event
                existing.last_seen = event.timestamp
                self._total_suppressed += 1
                return DedupResult(
                    decision="suppress",
                    event=None,
                    summary_events=tuple(summaries),
                )

            self._enforce_capacity()
            self._windows[event.fingerprint] = DedupWindow(
                fingerprint=event.fingerprint,
                first_event=event,
                last_event=event,
                count=1,
                first_seen=event.timestamp,
                last_seen=event.timestamp,
            )
            return DedupResult(
                decision="store",
                event=event,
                summary_events=tuple(summaries),
            )

    def flush(self, now: datetime | None = None) -> list[NormalizedEvent]:
        now = now or utc_now()
        with self._lock:
            summaries = self._flush_expired(now)
            summaries.extend(self._flush_periodic_summaries(now))
            return summaries

    def stats(self) -> DedupStats:
        with self._lock:
            return DedupStats(
                tracked_fingerprints=len(self._windows),
                total_suppressed=self._total_suppressed,
                total_summaries_emitted=self._total_summaries,
                evictions=self._evictions,
            )

    def _flush_expired(self, now: datetime) -> list[NormalizedEvent]:
        cutoff = now - timedelta(seconds=self._config.window_seconds)
        summaries: list[NormalizedEvent] = []
        expired_keys: list[str] = []
        for fp, window in self._windows.items():
            if window.last_seen < cutoff:
                expired_keys.append(fp)
                if window.count > 1:
                    summaries.append(self._make_summary_event(window, now))
        for key in expired_keys:
            del self._windows[key]
        return summaries

    def _flush_periodic_summaries(self, now: datetime) -> list[NormalizedEvent]:
        interval = timedelta(seconds=self._config.periodic_summary_interval_seconds)
        summaries: list[NormalizedEvent] = []
        for window in self._windows.values():
            if window.count <= 1:
                continue
            check_time = window.last_summary_at or window.first_seen
            if now - check_time >= interval:
                summaries.append(self._make_summary_event(window, now))
                window.last_summary_at = now
        return summaries

    def _make_summary_event(self, window: DedupWindow, now: datetime) -> NormalizedEvent:
        self._total_summaries += 1
        suppressed = window.count - 1
        source = window.first_event
        return NormalizedEvent(
            event_id=new_id("evt"),
            timestamp=now,
            timestamp_source="dedup_summary",
            service_id=source.service_id,
            host_id=source.host_id,
            severity=Severity.INFO,
            event_type=EventType.LOG,
            message=(
                f"Dedup summary: {window.count} events with fingerprint {window.fingerprint[:16]}... "
                f"({suppressed} suppressed) from {window.first_seen.isoformat()} to {window.last_seen.isoformat()}"
            ),
            structured_data={
                "_dedup_summary": True,
                "fingerprint": window.fingerprint,
                "count": window.count,
                "suppressed": suppressed,
                "first_event_id": source.event_id,
                "sample_event_id": window.last_event.event_id,
                "window_start": window.first_seen.isoformat(),
                "window_end": window.last_seen.isoformat(),
            },
            tags=frozenset({"dedup_summary"}),
            fingerprint=f"dedup-summary-{window.fingerprint}",
            quality=DataQuality(
                overall=1.0,
                timestamp_confidence=1.0,
                parse_confidence=1.0,
                identity_confidence=1.0,
                completeness=1.0,
            ),
            source_ref=SourceRef(
                source_type="dedup",
                source_id="dedup://tracker",
                raw_offset=None,
                collected_at=now,
            ),
            schema_version=1,
        )

    def _enforce_capacity(self) -> None:
        while len(self._windows) >= self._config.max_tracked_fingerprints:
            self._windows.popitem(last=False)
            self._evictions += 1
