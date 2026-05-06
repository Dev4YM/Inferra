from __future__ import annotations

import json
import math
import re
import threading
import time
from collections import deque
from dataclasses import dataclass, field, replace
from pathlib import Path

from config.models import NoiseFilterConfig
from core.enums import Severity
from core.logging import get_logger
from events.models import NormalizedEvent

_log = get_logger(__name__)

FAILURE_TAGS = frozenset({
    "oom",
    "crash",
    "restart",
    "connection_refused",
    "timeout",
    "disk_full",
    "permission_denied",
    "dns_failure",
    "certificate_error",
})

_SEVERITY_LOOKUP: dict[str, Severity] = {
    "DEBUG": Severity.DEBUG,
    "INFO": Severity.INFO,
    "WARN": Severity.WARN,
    "ERROR": Severity.ERROR,
    "CRITICAL": Severity.CRITICAL,
}


def _parse_severity(value: str) -> Severity:
    return _SEVERITY_LOOKUP.get(value.upper(), Severity.INFO)


@dataclass
class _FingerprintBucket:
    timestamps: deque[float] = field(default_factory=deque)

    def record(self, ts: float) -> None:
        self.timestamps.append(ts)

    def trim(self, window_seconds: float) -> None:
        cutoff = time.monotonic() - window_seconds
        while self.timestamps and self.timestamps[0] < cutoff:
            self.timestamps.popleft()

    def rate_per_minute(self, window_seconds: float) -> float:
        self.trim(window_seconds)
        count = len(self.timestamps)
        if count < 2:
            return 0.0
        span = self.timestamps[-1] - self.timestamps[0]
        if span <= 0:
            return float(count) * 60.0
        return (count / span) * 60.0

    def coefficient_of_variation(self, window_seconds: float) -> float:
        self.trim(window_seconds)
        if len(self.timestamps) < 3:
            return 1.0
        intervals: list[float] = []
        ts_list = list(self.timestamps)
        for i in range(1, len(ts_list)):
            intervals.append(ts_list[i] - ts_list[i - 1])
        mean = sum(intervals) / len(intervals)
        if mean <= 0:
            return 0.0
        variance = sum((x - mean) ** 2 for x in intervals) / len(intervals)
        return math.sqrt(variance) / mean


@dataclass(frozen=True)
class RoutineEntry:
    fingerprint: str
    service_id: str
    learned_at: float
    rate_per_minute: float
    cv: float


@dataclass(frozen=True)
class NoiseStats:
    blocklist_hits: int = 0
    allowlist_hits: int = 0
    adaptive_demotions: int = 0
    routine_fingerprints: int = 0
    total_filtered: int = 0


class NoiseFilter:
    def __init__(self, config: NoiseFilterConfig | None = None, data_dir: Path | None = None) -> None:
        self._config = config or NoiseFilterConfig()
        self._data_dir = data_dir
        self._lock = threading.Lock()
        self._buckets: dict[str, _FingerprintBucket] = {}
        self._routines: dict[str, RoutineEntry] = {}
        self._blocklist_hits = 0
        self._allowlist_hits = 0
        self._adaptive_demotions = 0
        self._total_filtered = 0
        self._always_keep = _parse_severity(self._config.always_keep_severity)
        if self._config.registry_enabled and data_dir is not None:
            self._load_registry()

    @property
    def config(self) -> NoiseFilterConfig:
        return self._config

    def annotate(self, event: NormalizedEvent) -> NormalizedEvent:
        score = self._relevance_score(event)
        is_routine = self._is_routine(event)
        structured = dict(event.structured_data)
        structured["_noise_score"] = score
        if is_routine:
            structured["_noise_routine"] = True
        return replace(event, structured_data=structured)

    def should_store(self, event: NormalizedEvent) -> bool:
        if self._config.allowlist_enabled and self._matches_allowlist(event):
            with self._lock:
                self._allowlist_hits += 1
            return True

        if event.severity >= self._always_keep:
            return True

        if self._config.blocklist_enabled and self._matches_blocklist(event):
            with self._lock:
                self._blocklist_hits += 1
                self._total_filtered += 1
            return False

        if self._config.adaptive_enabled and self._adaptive_should_filter(event):
            with self._lock:
                self._adaptive_demotions += 1
                self._total_filtered += 1
            return False

        return True

    def record_event(self, event: NormalizedEvent) -> None:
        if not self._config.adaptive_enabled:
            return
        with self._lock:
            bucket = self._buckets.get(event.fingerprint)
            if bucket is None:
                bucket = _FingerprintBucket()
                self._buckets[event.fingerprint] = bucket
            bucket.record(time.monotonic())

    def stats(self) -> NoiseStats:
        with self._lock:
            return NoiseStats(
                blocklist_hits=self._blocklist_hits,
                allowlist_hits=self._allowlist_hits,
                adaptive_demotions=self._adaptive_demotions,
                routine_fingerprints=len(self._routines),
                total_filtered=self._total_filtered,
            )

    def persist_registry(self) -> None:
        if not self._config.registry_enabled or self._data_dir is None:
            return
        path = self._registry_path()
        with self._lock:
            entries = [
                {
                    "fingerprint": entry.fingerprint,
                    "service_id": entry.service_id,
                    "learned_at": entry.learned_at,
                    "rate_per_minute": entry.rate_per_minute,
                    "cv": entry.cv,
                }
                for entry in self._routines.values()
            ]
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(entries, indent=2, sort_keys=True), encoding="utf-8")

    def _load_registry(self) -> None:
        path = self._registry_path()
        if not path.exists():
            return
        try:
            raw = json.loads(path.read_text(encoding="utf-8"))
            expiry = self._config.registry_expiry_days * 86400
            now = time.time()
            for item in raw:
                if now - item["learned_at"] > expiry:
                    continue
                entry = RoutineEntry(
                    fingerprint=item["fingerprint"],
                    service_id=item["service_id"],
                    learned_at=item["learned_at"],
                    rate_per_minute=item["rate_per_minute"],
                    cv=item["cv"],
                )
                self._routines[entry.fingerprint] = entry
            if self._routines:
                _log.info("Loaded noise registry", extra={"entries": len(self._routines)})
        except (json.JSONDecodeError, KeyError, TypeError) as exc:
            _log.warning("Failed to load noise registry", extra={"error": str(exc)})

    def _registry_path(self) -> Path:
        assert self._data_dir is not None
        return self._data_dir / "noise_registry.json"

    def _matches_blocklist(self, event: NormalizedEvent) -> bool:
        for rule in self._config.blocklist:
            if not rule.pattern:
                continue
            sev_max = _parse_severity(rule.severity_max)
            if event.severity > sev_max:
                continue
            if rule.service_id and rule.service_id != event.service_id:
                continue
            try:
                if re.search(rule.pattern, event.message, re.IGNORECASE):
                    return True
            except re.error:
                if rule.pattern.lower() in event.message.lower():
                    return True
        return False

    def _matches_allowlist(self, event: NormalizedEvent) -> bool:
        for rule in self._config.allowlist:
            if not rule.pattern:
                continue
            if rule.tags and event.tags & frozenset(rule.tags):
                return True
            try:
                if re.search(rule.pattern, event.message, re.IGNORECASE):
                    return True
            except re.error:
                if rule.pattern.lower() in event.message.lower():
                    return True
        return False

    def _adaptive_should_filter(self, event: NormalizedEvent) -> bool:
        if event.severity >= self._always_keep:
            return False

        with self._lock:
            if event.fingerprint in self._routines:
                return self._should_sample_routine(event)

            bucket = self._buckets.get(event.fingerprint)
            if bucket is None:
                return False

            window_seconds = self._config.frequency_window_minutes * 60.0
            rate = bucket.rate_per_minute(window_seconds)
            if rate <= self._config.high_rate_threshold_per_minute:
                return False

            cv = bucket.coefficient_of_variation(window_seconds)
            if cv >= self._config.stability_threshold_cv:
                return False

            self._routines[event.fingerprint] = RoutineEntry(
                fingerprint=event.fingerprint,
                service_id=event.service_id,
                learned_at=time.time(),
                rate_per_minute=rate,
                cv=cv,
            )
            _log.info(
                "Learned routine fingerprint",
                extra={
                    "fingerprint": event.fingerprint[:16],
                    "rate": round(rate, 1),
                    "cv": round(cv, 4),
                    "service_id": event.service_id,
                },
            )
            return self._should_sample_routine(event)

    def _should_sample_routine(self, event: NormalizedEvent) -> bool:
        bucket = self._buckets.get(event.fingerprint)
        if bucket is None:
            return False
        window_seconds = self._config.frequency_window_minutes * 60.0
        rate = bucket.rate_per_minute(window_seconds)
        target = self._config.routine_sample_target_per_minute
        if target <= 0 or rate <= 0:
            return True
        keep_probability = min(1.0, target / rate)
        recent_count = len(bucket.timestamps)
        if recent_count == 0:
            return False
        return (recent_count % max(1, int(1.0 / keep_probability))) != 0

    def _is_routine(self, event: NormalizedEvent) -> bool:
        with self._lock:
            return event.fingerprint in self._routines

    def _relevance_score(self, event: NormalizedEvent) -> float:
        severity_score = {
            Severity.DEBUG: 0.1,
            Severity.INFO: 0.2,
            Severity.WARN: 0.6,
            Severity.ERROR: 1.0,
            Severity.CRITICAL: 1.0,
        }.get(event.severity, 0.2)
        tag_score = 1.0 if event.tags & FAILURE_TAGS else 0.2
        return round(0.7 * severity_score + 0.3 * tag_score, 4)
