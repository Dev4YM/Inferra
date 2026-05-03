from __future__ import annotations

import asyncio
import json
import platform
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
class WindowsServiceSnapshot:
    name: str
    display_name: str
    status: str
    start_type: str | None
    pid: int | None
    username: str | None
    binary_path: str | None


class WindowsServiceCollector:
    source_type = "windows_service"

    def __init__(
        self,
        poll_interval_seconds: float = 30.0,
        include_stopped: bool = False,
        names: tuple[str, ...] = (),
    ) -> None:
        self.poll_interval_seconds = poll_interval_seconds
        self.include_stopped = include_stopped
        self.names = tuple(name.lower() for name in names)
        self._running = False
        self._events = 0
        self._errors = 0
        self._last_error: str | None = None
        self._last_event_at = None
        self._host = socket.gethostname()

    @property
    def collector_id(self) -> str:
        return f"windows_service://{self._host}"

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
            emitted = 0
            for snapshot in self._snapshots():
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

    def _snapshots(self) -> list[WindowsServiceSnapshot]:
        if psutil is None or not hasattr(psutil, "win_service_iter"):
            raise RuntimeError("psutil Windows service support is unavailable")
        snapshots: list[WindowsServiceSnapshot] = []
        for service in psutil.win_service_iter():
            try:
                info = service.as_dict()
            except Exception:
                continue
            name = str(info.get("name") or "")
            if self.names and name.lower() not in self.names:
                continue
            status = str(info.get("status") or "unknown")
            if status == "stopped" and not self.include_stopped:
                continue
            snapshots.append(
                WindowsServiceSnapshot(
                    name=name,
                    display_name=str(info.get("display_name") or name),
                    status=status,
                    start_type=_optional_str(info.get("start_type")),
                    pid=_optional_int(info.get("pid")),
                    username=_optional_str(info.get("username")),
                    binary_path=_optional_str(info.get("binpath")),
                )
            )
        return sorted(snapshots, key=lambda item: item.name.lower())

    def _event(self, snapshot: WindowsServiceSnapshot) -> RawEvent:
        severity = "info"
        if snapshot.status not in {"running", "paused"}:
            severity = "warn"
        if snapshot.start_type == "automatic" and snapshot.status != "running":
            severity = "error"
        message = (
            f"windows service {snapshot.name} status={snapshot.status} "
            f"start_type={snapshot.start_type or 'unknown'}"
        )
        payload: dict[str, Any] = {
            "timestamp": to_iso(utc_now()),
            "level": severity,
            "service": snapshot.name,
            "host": self._host,
            "message": message,
            "windows_service": {
                "name": snapshot.name,
                "display_name": snapshot.display_name,
                "status": snapshot.status,
                "start_type": snapshot.start_type,
                "pid": snapshot.pid,
                "username": snapshot.username,
                "binary_path": snapshot.binary_path,
            },
        }
        return RawEvent(
            source_type=self.source_type,
            source_id=f"{self.collector_id}/{snapshot.name}",
            raw_payload=json.dumps(payload, sort_keys=True),
            collected_at=utc_now(),
            metadata={"service_name": snapshot.name, "status": snapshot.status, "host": self._host},
        )


def _optional_str(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value)
    return text if text else None


def _optional_int(value: Any) -> int | None:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None
