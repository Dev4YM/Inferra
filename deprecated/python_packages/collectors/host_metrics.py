from __future__ import annotations

import json
import re
import shutil
import socket
from pathlib import Path

from collectors.base import PollingCollector
from core.time import to_iso, utc_now
from events.models import RawEvent
from storage.metric_ringbuffer import MetricRingbuffer

try:
    import psutil
except ImportError:  # pragma: no cover
    psutil = None


class HostMetricsCollector(PollingCollector):
    source_type = "host_metrics"

    def __init__(
        self,
        poll_interval_seconds: float = 10.0,
        warn_cpu_percent: float = 85.0,
        warn_memory_percent: float = 85.0,
        warn_disk_percent: float = 90.0,
        metrics_dir: Path | None = None,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(poll_interval_seconds=poll_interval_seconds, emit_timeout_seconds=emit_timeout_seconds)
        self.warn_cpu_percent = warn_cpu_percent
        self.warn_memory_percent = warn_memory_percent
        self.warn_disk_percent = warn_disk_percent
        self.metrics_dir = metrics_dir
        self._host = socket.gethostname()
        self._threshold_state = {"cpu": False, "memory": False, "disk": False}
        self._ringbuffers = {
            "cpu_percent": MetricRingbuffer(service_id="host", metric_name="cpu_percent"),
            "memory_percent": MetricRingbuffer(service_id="host", metric_name="memory_percent"),
            "disk_percent": MetricRingbuffer(service_id="host", metric_name="disk_percent"),
        }

    @property
    def collector_id(self) -> str:
        return "host_metrics://local"

    async def collect_once(self, sink) -> int:
        try:
            observed_at = utc_now()
            metrics = self._metrics()
            self._write_metrics(observed_at, metrics)
            crossings = self._threshold_crossings(metrics)
            if not crossings:
                return 0
            event = self._event(metrics, crossings, observed_at)
            emitted = await self.emit(sink, event)
            return 1 if emitted else 0
        except Exception as exc:  # pragma: no cover
            self._record_error(exc)
            return 0

    def _metrics(self) -> dict[str, float]:
        root_path = Path.cwd().anchor or "/"
        if psutil is not None:
            disk = psutil.disk_usage(root_path)
            boot_time = float(psutil.boot_time())
            return {
                "cpu_percent": float(psutil.cpu_percent(interval=None)),
                "memory_percent": float(psutil.virtual_memory().percent),
                "disk_percent": float(disk.percent),
                "disk_free_gb": round(float(disk.free) / (1024**3), 2),
                "boot_time": boot_time,
            }
        total, used, _free = shutil.disk_usage(root_path)
        return {"disk_percent": used / total * 100.0}

    def _write_metrics(self, observed_at, metrics: dict[str, float]) -> None:
        for metric_name in ("cpu_percent", "memory_percent", "disk_percent"):
            value = metrics.get(metric_name)
            if value is None:
                continue
            ringbuffer = self._ringbuffers[metric_name]
            ringbuffer.append(observed_at, float(value))
            if self.metrics_dir is not None:
                ringbuffer.save_to_json(self.metrics_dir / f"{_safe_name(self.collector_id)}_{metric_name}.json")

    def _threshold_crossings(self, metrics: dict[str, float]) -> list[tuple[str, str]]:
        states = {
            "cpu": metrics.get("cpu_percent", 0.0) >= self.warn_cpu_percent,
            "memory": metrics.get("memory_percent", 0.0) >= self.warn_memory_percent,
            "disk": metrics.get("disk_percent", 0.0) >= self.warn_disk_percent,
        }
        changes: list[tuple[str, str]] = []
        for name, active in states.items():
            previous = self._threshold_state[name]
            if active and not previous:
                changes.append((name, "entered"))
            elif not active and previous:
                changes.append((name, "recovered"))
            self._threshold_state[name] = active
        return changes

    def _event(self, metrics: dict[str, float], crossings: list[tuple[str, str]], observed_at) -> RawEvent:
        entered = [name for name, state in crossings if state == "entered"]
        recovered = [name for name, state in crossings if state == "recovered"]
        severity = "warn" if entered else "info"
        if entered and recovered:
            message = f"host threshold changes: entered={', '.join(entered)} recovered={', '.join(recovered)}"
        elif entered:
            message = f"host resource pressure detected: {', '.join(entered)}"
        else:
            message = f"host resource pressure recovered: {', '.join(recovered)}"
        payload = {
            "timestamp": to_iso(observed_at),
            "level": severity,
            "service": "host",
            "host": self._host,
            "message": message,
            "metrics": metrics,
            "threshold_crossings": [{"metric": name, "state": state} for name, state in crossings],
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
            collected_at=observed_at,
            metadata={
                "host": self._host,
                "metrics": sorted(metrics),
                "threshold_crossings": tuple(f"{name}:{state}" for name, state in crossings),
            },
        )


def _safe_name(value: str) -> str:
    return re.sub(r"[^a-zA-Z0-9_.-]+", "_", value).strip("_") or "collector"
