from __future__ import annotations

import json
import platform
import socket
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from collectors.base import PollingCollector
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


class WindowsServiceCollector(PollingCollector):
    source_type = "windows_service"

    def __init__(
        self,
        poll_interval_seconds: float = 30.0,
        include_stopped: bool = False,
        names: tuple[str, ...] = (),
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(poll_interval_seconds=poll_interval_seconds, emit_timeout_seconds=emit_timeout_seconds)
        self.include_stopped = include_stopped
        self.names = tuple(name.lower() for name in names)
        self._host = socket.gethostname()
        self._last_snapshots: dict[str, WindowsServiceSnapshot] = {}

    @property
    def collector_id(self) -> str:
        return f"windows_service://{self._host}"

    async def collect_once(self, sink) -> int:
        if platform.system().lower() != "windows":
            return 0
        try:
            snapshots = self._snapshots()
            current = {snapshot.name.lower(): snapshot for snapshot in snapshots}
            changed: list[tuple[WindowsServiceSnapshot, WindowsServiceSnapshot | None]] = []
            for key, snapshot in current.items():
                previous = self._last_snapshots.get(key)
                if previous != snapshot:
                    changed.append((snapshot, previous))
            self._last_snapshots = current
            emitted = 0
            observed_at = utc_now()
            for snapshot, previous in changed:
                event = self._event(snapshot, previous, observed_at)
                emitted += 1 if await self.emit(sink, event) else 0
            return emitted
        except Exception as exc:  # pragma: no cover
            self._record_error(exc)
            return 0

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

    def _event(
        self,
        snapshot: WindowsServiceSnapshot,
        previous: WindowsServiceSnapshot | None,
        observed_at: datetime,
    ) -> RawEvent:
        severity = "info"
        if snapshot.status not in {"running", "paused"}:
            severity = "warn"
        if snapshot.start_type == "automatic" and snapshot.status != "running":
            severity = "error"
        previous_status = previous.status if previous is not None else "unknown"
        message = (
            f"windows service {snapshot.name} state change {previous_status}->{snapshot.status} "
            f"status={snapshot.status} start_type={snapshot.start_type or 'unknown'}"
        )
        payload: dict[str, Any] = {
            "timestamp": to_iso(observed_at),
            "level": severity,
            "service": snapshot.name,
            "host": self._host,
            "message": message,
            "windows_service": {
                "name": snapshot.name,
                "display_name": snapshot.display_name,
                "previous_status": previous_status,
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
            collected_at=observed_at,
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
