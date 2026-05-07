from __future__ import annotations

import json
import re
import socket
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any

from collectors.base import PollingCollector
from core.time import to_iso, utc_now
from events.models import RawEvent
from storage.metric_ringbuffer import MetricRingbuffer

try:
    import psutil
except ImportError:  # pragma: no cover
    psutil = None


@dataclass(frozen=True)
class ProcessSnapshot:
    pid: int
    name: str
    username: str | None
    status: str
    cpu_percent: float
    memory_mb: float
    create_time: float | None
    command: str


class ProcessSnapshotCollector(PollingCollector):
    source_type = "process_snapshot"

    def __init__(
        self,
        poll_interval_seconds: float = 15.0,
        top_n: int = 20,
        min_cpu_percent: float = 75.0,
        min_memory_mb: float = 512.0,
        watch_processes: tuple[str, ...] = (),
        watch_pids: tuple[int, ...] = (),
        metrics_dir: Path | None = None,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(poll_interval_seconds=poll_interval_seconds, emit_timeout_seconds=emit_timeout_seconds)
        self.top_n = top_n
        self.min_cpu_percent = min_cpu_percent
        self.min_memory_mb = min_memory_mb
        self.watch_processes = tuple(item.lower() for item in watch_processes)
        self.watch_pids = frozenset(int(item) for item in watch_pids)
        self.metrics_dir = metrics_dir
        self._host = socket.gethostname()
        self._threshold_state: dict[str, tuple[bool, bool]] = {}
        self._ringbuffers: dict[str, dict[str, MetricRingbuffer]] = {}

    @property
    def collector_id(self) -> str:
        return f"process_snapshot://{self._host}"

    async def collect_once(self, sink) -> int:
        try:
            snapshots = self._snapshots()
            observed_at = utc_now()
            emitted = 0
            active_keys: set[str] = set()
            for snapshot in snapshots:
                process_key = self._process_key(snapshot)
                active_keys.add(process_key)
                self._write_metrics(process_key, snapshot, observed_at)
                crossings = self._threshold_crossings(process_key, snapshot)
                if not crossings:
                    continue
                event = self._event(snapshot, crossings, observed_at)
                emitted += 1 if await self.emit(sink, event) else 0
            self._expire_process_state(active_keys)
            return emitted
        except Exception as exc:  # pragma: no cover
            self._record_error(exc)
            return 0

    def _snapshots(self) -> list[ProcessSnapshot]:
        if psutil is None:
            raise RuntimeError("psutil is required for process snapshots")
        items: list[ProcessSnapshot] = []
        attrs = ["pid", "name", "username", "status", "cpu_percent", "memory_info", "create_time", "cmdline"]
        for proc in psutil.process_iter(attrs=attrs):
            try:
                info = proc.info
                pid = int(info["pid"])
                name = str(info.get("name") or "unknown")
                if self.watch_pids and pid not in self.watch_pids:
                    continue
                if self.watch_processes and name.lower() not in self.watch_processes:
                    continue
                memory_info = info.get("memory_info")
                memory_mb = float(getattr(memory_info, "rss", 0)) / (1024 * 1024)
                cpu_percent = float(info.get("cpu_percent") or 0.0)
                if cpu_percent < self.min_cpu_percent and memory_mb < self.min_memory_mb:
                    continue
                command = " ".join(str(part) for part in (info.get("cmdline") or []) if part)
                items.append(
                    ProcessSnapshot(
                        pid=pid,
                        name=name,
                        username=info.get("username"),
                        status=str(info.get("status") or "unknown"),
                        cpu_percent=cpu_percent,
                        memory_mb=round(memory_mb, 2),
                        create_time=info.get("create_time"),
                        command=command[:512],
                    )
                )
            except (psutil.NoSuchProcess, psutil.AccessDenied, psutil.ZombieProcess):
                continue
        return sorted(items, key=lambda item: (item.cpu_percent, item.memory_mb), reverse=True)[: self.top_n]

    def _process_key(self, snapshot: ProcessSnapshot) -> str:
        return f"{snapshot.name}:{snapshot.pid}:{snapshot.create_time or 0}"

    def _write_metrics(self, process_key: str, snapshot: ProcessSnapshot, observed_at: datetime) -> None:
        ringbuffers = self._ringbuffers.setdefault(
            process_key,
            {
                "cpu_percent": MetricRingbuffer(service_id=snapshot.name, metric_name=f"{snapshot.pid}_cpu_percent"),
                "memory_mb": MetricRingbuffer(service_id=snapshot.name, metric_name=f"{snapshot.pid}_memory_mb"),
            },
        )
        ringbuffers["cpu_percent"].append(observed_at, snapshot.cpu_percent)
        ringbuffers["memory_mb"].append(observed_at, snapshot.memory_mb)
        if self.metrics_dir is not None:
            base_name = f"{_safe_name(snapshot.name)}_{snapshot.pid}"
            ringbuffers["cpu_percent"].save_to_json(self.metrics_dir / f"{base_name}_cpu_percent.json")
            ringbuffers["memory_mb"].save_to_json(self.metrics_dir / f"{base_name}_memory_mb.json")

    def _threshold_crossings(self, process_key: str, snapshot: ProcessSnapshot) -> list[tuple[str, str]]:
        cpu_high = snapshot.cpu_percent >= self.min_cpu_percent
        memory_high = snapshot.memory_mb >= self.min_memory_mb
        previous_cpu, previous_memory = self._threshold_state.get(process_key, (False, False))
        self._threshold_state[process_key] = (cpu_high, memory_high)
        crossings: list[tuple[str, str]] = []
        if cpu_high and not previous_cpu:
            crossings.append(("cpu", "entered"))
        elif previous_cpu and not cpu_high:
            crossings.append(("cpu", "recovered"))
        if memory_high and not previous_memory:
            crossings.append(("memory", "entered"))
        elif previous_memory and not memory_high:
            crossings.append(("memory", "recovered"))
        return crossings

    def _expire_process_state(self, active_keys: set[str]) -> None:
        stale_keys = [key for key in self._threshold_state if key not in active_keys]
        for key in stale_keys:
            del self._threshold_state[key]
            self._ringbuffers.pop(key, None)

    def _event(self, snapshot: ProcessSnapshot, crossings: list[tuple[str, str]], observed_at: datetime) -> RawEvent:
        entered = [name for name, state in crossings if state == "entered"]
        recovered = [name for name, state in crossings if state == "recovered"]
        severity = "warn" if entered else "info"
        if entered and recovered:
            message = (
                f"process {snapshot.name} pid={snapshot.pid} threshold changes "
                f"entered={', '.join(entered)} recovered={', '.join(recovered)}"
            )
        elif entered:
            message = (
                f"process {snapshot.name} pid={snapshot.pid} high {' and '.join(entered)} "
                f"cpu={snapshot.cpu_percent:.1f}% memory={snapshot.memory_mb:.1f}MB status={snapshot.status}"
            )
        else:
            message = (
                f"process {snapshot.name} pid={snapshot.pid} recovered "
                f"{', '.join(recovered)} cpu={snapshot.cpu_percent:.1f}% "
                f"memory={snapshot.memory_mb:.1f}MB status={snapshot.status}"
            )
        payload: dict[str, Any] = {
            "timestamp": to_iso(observed_at),
            "level": severity,
            "service": snapshot.name,
            "host": self._host,
            "message": message,
            "threshold_crossings": [{"metric": name, "state": state} for name, state in crossings],
            "process": {
                "pid": snapshot.pid,
                "name": snapshot.name,
                "username": snapshot.username,
                "status": snapshot.status,
                "cpu_percent": snapshot.cpu_percent,
                "memory_mb": snapshot.memory_mb,
                "create_time": snapshot.create_time,
                "command": snapshot.command,
            },
        }
        return RawEvent(
            source_type=self.source_type,
            source_id=f"{self.collector_id}/{snapshot.pid}",
            raw_payload=json.dumps(payload, sort_keys=True),
            collected_at=observed_at,
            metadata={"pid": snapshot.pid, "process_name": snapshot.name, "host": self._host},
        )


def _safe_name(value: str) -> str:
    return re.sub(r"[^a-zA-Z0-9_.-]+", "_", value).strip("_") or "process"
