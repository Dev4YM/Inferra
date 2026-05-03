from __future__ import annotations

import asyncio
import json
import shutil
import socket

from collectors.base import CollectorHealth
from core.time import to_iso, utc_now
from events.models import RawEvent

try:
    import psutil
except ImportError:  # pragma: no cover
    psutil = None


class HostMetricsCollector:
    source_type = "host_metrics"

    def __init__(
        self,
        poll_interval_seconds: float = 10.0,
        warn_cpu_percent: float = 85.0,
        warn_memory_percent: float = 85.0,
        warn_disk_percent: float = 90.0,
    ) -> None:
        self.poll_interval_seconds = poll_interval_seconds
        self.warn_cpu_percent = warn_cpu_percent
        self.warn_memory_percent = warn_memory_percent
        self.warn_disk_percent = warn_disk_percent
        self._running = False
        self._events = 0
        self._errors = 0
        self._last_error: str | None = None
        self._last_event_at = None
        self._host = socket.gethostname()

    @property
    def collector_id(self) -> str:
        return "host_metrics://local"

    async def start(self, sink: asyncio.Queue[RawEvent]) -> None:
        self._running = True
        try:
            while self._running:
                await self.collect_once(sink)
                await asyncio.sleep(self.poll_interval_seconds)
        finally:
            self._running = False

    async def collect_once(self, sink: asyncio.Queue[RawEvent]) -> int:
        try:
            metrics = self._metrics()
            event = self._event(metrics)
            await sink.put(event)
            self._events += 1
            self._last_event_at = event.collected_at
            return 1
        except Exception as exc:  # pragma: no cover
            self._errors += 1
            self._last_error = str(exc)
            return 0

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

    def _metrics(self) -> dict[str, float]:
        if psutil is not None:
            disk = psutil.disk_usage("/")
            boot_time = float(psutil.boot_time())
            return {
                "cpu_percent": float(psutil.cpu_percent(interval=None)),
                "memory_percent": float(psutil.virtual_memory().percent),
                "disk_percent": float(disk.percent),
                "disk_free_gb": round(float(disk.free) / (1024**3), 2),
                "boot_time": boot_time,
            }
        total, used, _free = shutil.disk_usage("/")
        return {"disk_percent": used / total * 100.0}

    def _event(self, metrics: dict[str, float]) -> RawEvent:
        severity = "info"
        pressure = []
        if metrics.get("cpu_percent", 0) >= self.warn_cpu_percent:
            pressure.append("cpu")
        if metrics.get("memory_percent", 0) >= self.warn_memory_percent:
            pressure.append("memory")
        if metrics.get("disk_percent", 0) >= self.warn_disk_percent:
            pressure.append("disk")
        if pressure:
            severity = "warn"
        message = "host metrics snapshot"
        if pressure:
            message = f"host resource pressure detected: {', '.join(pressure)}"
        payload = {
            "timestamp": to_iso(utc_now()),
            "level": severity,
            "service": "host",
            "host": self._host,
            "message": message,
            "metrics": metrics,
            "thresholds": {
                "warn_cpu_percent": self.warn_cpu_percent,
                "warn_memory_percent": self.warn_memory_percent,
                "warn_disk_percent": self.warn_disk_percent,
            },
        }
        return RawEvent(
            source_type=self.source_type,
            source_id=self.collector_id,
            raw_payload=json.dumps(payload, sort_keys=True),
            collected_at=utc_now(),
            metadata={"host": self._host, "metrics": sorted(metrics)},
        )
