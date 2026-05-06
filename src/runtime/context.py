from __future__ import annotations

import asyncio
import json
import platform
import shutil
import socket
import subprocess
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any

from core.logging import get_logger
from core.time import utc_now

_log = get_logger(__name__)

try:
    import psutil
except ImportError:  # pragma: no cover
    psutil = None


@dataclass(frozen=True)
class ProcessListEntry:
    pid: int
    name: str
    cpu_percent: float
    memory_mb: float


@dataclass(frozen=True)
class DiskMountSnapshot:
    mount: str
    total_gb: float
    used_percent: float


@dataclass(frozen=True)
class ContainerSummary:
    container_id: str
    name: str
    image: str
    state: str


@dataclass(frozen=True)
class RuntimeContextSnapshot:
    captured_at: datetime
    hostname: str
    system: str
    cpu_percent: float | None
    memory_used_percent: float | None
    disk: tuple[DiskMountSnapshot, ...]
    processes: tuple[ProcessListEntry, ...]
    containers: tuple[ContainerSummary, ...]


def _disk_snapshots() -> tuple[DiskMountSnapshot, ...]:
    root = Path.cwd().anchor or "/"
    results: list[DiskMountSnapshot] = []
    try:
        usage = shutil.disk_usage(root)
        total_gb = float(usage.total) / (1024**3)
        used_percent = float(usage.used) / float(usage.total) * 100.0 if usage.total else 0.0
        results.append(DiskMountSnapshot(mount=str(root), total_gb=round(total_gb, 2), used_percent=round(used_percent, 2)))
    except OSError as exc:
        _log.warning("disk_usage_failed", extra={"path": str(root), "error": str(exc)})
    return tuple(results)


def _process_entries(limit: int) -> tuple[ProcessListEntry, ...]:
    if psutil is None:
        return ()
    entries: list[ProcessListEntry] = []
    try:
        procs = list(psutil.process_iter(["pid", "name", "cpu_percent", "memory_info"]))
        procs.sort(key=lambda proc: int(proc.info.get("pid") or 0))
        for proc in procs[:limit]:
            info = proc.info
            mem_mb = 0.0
            mem_info = info.get("memory_info")
            if mem_info is not None:
                mem_mb = float(getattr(mem_info, "rss", 0) or 0) / (1024**2)
            entries.append(
                ProcessListEntry(
                    pid=int(info.get("pid") or 0),
                    name=str(info.get("name") or ""),
                    cpu_percent=float(info.get("cpu_percent") or 0.0),
                    memory_mb=round(mem_mb, 2),
                )
            )
    except (psutil.Error, TypeError, ValueError) as exc:
        _log.warning("process_snapshot_failed", extra={"error": str(exc)})
    return tuple(entries)


def _container_summaries() -> tuple[ContainerSummary, ...]:
    try:
        proc = subprocess.run(
            ["docker", "ps", "--format", "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}"],
            check=False,
            capture_output=True,
            text=True,
            timeout=3,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        _log.warning("docker_ps_unavailable", extra={"error": str(exc)})
        return ()
    if proc.returncode != 0:
        return ()
    rows: list[ContainerSummary] = []
    for line in proc.stdout.splitlines():
        parts = line.strip().split("\t")
        if len(parts) < 4:
            continue
        rows.append(
            ContainerSummary(
                container_id=parts[0][:12],
                name=parts[1],
                image=parts[2],
                state=parts[3],
            )
        )
    return tuple(rows)


def build_runtime_context_snapshot_sync(*, process_limit: int = 40) -> RuntimeContextSnapshot:
    captured_at = utc_now()
    hostname = socket.gethostname()
    system = platform.system() + " " + platform.release()
    cpu_percent: float | None = None
    memory_percent: float | None = None
    if psutil is not None:
        try:
            cpu_percent = float(psutil.cpu_percent(interval=None))
            memory_percent = float(psutil.virtual_memory().percent)
        except (psutil.Error, TypeError, ValueError) as exc:
            _log.warning("host_metrics_failed", extra={"error": str(exc)})
    return RuntimeContextSnapshot(
        captured_at=captured_at,
        hostname=hostname,
        system=system.strip(),
        cpu_percent=cpu_percent,
        memory_used_percent=memory_percent,
        disk=_disk_snapshots(),
        processes=_process_entries(process_limit),
        containers=_container_summaries(),
    )


async def build_runtime_context_snapshot(*, process_limit: int = 40) -> RuntimeContextSnapshot:
    return await asyncio.to_thread(build_runtime_context_snapshot_sync, process_limit=process_limit)


def runtime_context_to_correlation_dict(snapshot: RuntimeContextSnapshot) -> dict[str, Any]:
    return {
        "captured_at": snapshot.captured_at.isoformat(),
        "hostname": snapshot.hostname,
        "system": snapshot.system,
        "cpu_percent": snapshot.cpu_percent,
        "memory_used_percent": snapshot.memory_used_percent,
        "disk": [disk.__dict__ for disk in snapshot.disk],
        "processes": [proc.__dict__ for proc in snapshot.processes],
        "containers": [ctr.__dict__ for ctr in snapshot.containers],
    }


def runtime_context_json(snapshot: RuntimeContextSnapshot) -> str:
    return json.dumps(runtime_context_to_correlation_dict(snapshot), sort_keys=True)
