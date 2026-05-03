from __future__ import annotations

import asyncio
import json
import socket
from dataclasses import dataclass
from typing import Any

from collectors.base import CollectorHealth
from core.time import to_iso, utc_now
from events.models import RawEvent

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


class ProcessSnapshotCollector:
    source_type = "process_snapshot"

    def __init__(
        self,
        poll_interval_seconds: float = 15.0,
        top_n: int = 20,
        min_cpu_percent: float = 75.0,
        min_memory_mb: float = 512.0,
    ) -> None:
        self.poll_interval_seconds = poll_interval_seconds
        self.top_n = top_n
        self.min_cpu_percent = min_cpu_percent
        self.min_memory_mb = min_memory_mb
        self._running = False
        self._events = 0
        self._errors = 0
        self._last_error: str | None = None
        self._last_event_at = None
        self._host = socket.gethostname()

    @property
    def collector_id(self) -> str:
        return f"process_snapshot://{self._host}"

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
            snapshots = self._snapshots()
            emitted = 0
            for snapshot in snapshots:
                event = self._event(snapshot)
                await sink.put(event)
                self._events += 1
                emitted += 1
                self._last_event_at = event.collected_at
            return emitted
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

    def _snapshots(self) -> list[ProcessSnapshot]:
        if psutil is None:
            raise RuntimeError("psutil is required for process snapshots")
        items: list[ProcessSnapshot] = []
        attrs = ["pid", "name", "username", "status", "cpu_percent", "memory_info", "create_time", "cmdline"]
        for proc in psutil.process_iter(attrs=attrs):
            try:
                info = proc.info
                memory_info = info.get("memory_info")
                memory_mb = float(getattr(memory_info, "rss", 0)) / (1024 * 1024)
                cpu_percent = float(info.get("cpu_percent") or 0.0)
                if cpu_percent < self.min_cpu_percent and memory_mb < self.min_memory_mb:
                    continue
                command = " ".join(str(part) for part in (info.get("cmdline") or []) if part)
                items.append(
                    ProcessSnapshot(
                        pid=int(info["pid"]),
                        name=str(info.get("name") or "unknown"),
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

    def _event(self, snapshot: ProcessSnapshot) -> RawEvent:
        severity = "warn" if snapshot.cpu_percent >= self.min_cpu_percent else "info"
        if snapshot.memory_mb >= self.min_memory_mb and snapshot.cpu_percent >= self.min_cpu_percent:
            message = (
                f"process {snapshot.name} pid={snapshot.pid} high cpu={snapshot.cpu_percent:.1f}% "
                f"memory={snapshot.memory_mb:.1f}MB status={snapshot.status}"
            )
        elif snapshot.memory_mb >= self.min_memory_mb:
            message = (
                f"process {snapshot.name} pid={snapshot.pid} high memory={snapshot.memory_mb:.1f}MB "
                f"cpu={snapshot.cpu_percent:.1f}% status={snapshot.status}"
            )
        else:
            message = (
                f"process {snapshot.name} pid={snapshot.pid} high cpu={snapshot.cpu_percent:.1f}% "
                f"memory={snapshot.memory_mb:.1f}MB status={snapshot.status}"
            )
        payload: dict[str, Any] = {
            "timestamp": to_iso(utc_now()),
            "level": severity,
            "service": snapshot.name,
            "host": self._host,
            "message": message,
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
            source_id=self.collector_id,
            raw_payload=json.dumps(payload, sort_keys=True),
            collected_at=utc_now(),
            metadata={"pid": snapshot.pid, "process_name": snapshot.name, "host": self._host},
        )
