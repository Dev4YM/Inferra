from __future__ import annotations

import platform
from typing import Any

from collectors.base import Collector
from collectors.file import FileCollector
from collectors.host_metrics import HostMetricsCollector
from collectors.journald import JournaldCollector
from collectors.kubernetes import KubernetesCollector
from collectors.linux_syslog import LinuxSyslogCollector
from collectors.process_snapshot import ProcessSnapshotCollector
from collectors.windows_eventlog import WindowsEventLogCollector
from collectors.windows_service import WindowsServiceCollector
from config.model import InferraConfig


def build_collectors(config: InferraConfig, state_store: Any | None = None) -> list[Collector]:
    collectors: list[Collector] = []
    current_platform = platform.system().lower()

    if config.collectors.host_metrics.enabled:
        cfg = config.collectors.host_metrics
        collectors.append(
            HostMetricsCollector(
                poll_interval_seconds=cfg.poll_interval_seconds,
                warn_cpu_percent=cfg.warn_cpu_percent,
                warn_memory_percent=cfg.warn_memory_percent,
                warn_disk_percent=cfg.warn_disk_percent,
            )
        )

    if config.collectors.process.enabled:
        cfg = config.collectors.process
        collectors.append(
            ProcessSnapshotCollector(
                poll_interval_seconds=cfg.poll_interval_seconds,
                top_n=cfg.top_n,
                min_cpu_percent=cfg.min_cpu_percent,
                min_memory_mb=cfg.min_memory_mb,
            )
        )

    if config.collectors.file.enabled:
        cfg = config.collectors.file
        for path in cfg.paths:
            collectors.append(
                FileCollector(
                    path,
                    poll_interval_seconds=cfg.poll_interval_seconds,
                    start_at_end=cfg.start_at_end,
                )
            )

    if current_platform == "windows":
        if config.collectors.windows_eventlog.enabled:
            cfg = config.collectors.windows_eventlog
            collectors.append(
                WindowsEventLogCollector(
                    channels=cfg.channels,
                    poll_interval_seconds=cfg.poll_interval_seconds,
                    state_store=state_store,
                )
            )
        if config.collectors.windows_service.enabled:
            cfg = config.collectors.windows_service
            collectors.append(
                WindowsServiceCollector(
                    poll_interval_seconds=cfg.poll_interval_seconds,
                    include_stopped=cfg.include_stopped,
                    names=cfg.names,
                )
            )

    if current_platform == "linux":
        if config.collectors.linux_syslog.enabled:
            cfg = config.collectors.linux_syslog
            collectors.append(
                LinuxSyslogCollector(
                    paths=cfg.paths,
                    poll_interval_seconds=cfg.poll_interval_seconds,
                    start_at_end=cfg.start_at_end,
                )
            )
        if config.collectors.journald.enabled:
            cfg = config.collectors.journald
            collectors.append(
                JournaldCollector(
                    units=cfg.units,
                    since=cfg.since,
                    limit=cfg.limit,
                    poll_interval_seconds=cfg.poll_interval_seconds,
                    state_store=state_store,
                )
            )

    if config.collectors.kubernetes.enabled:
        cfg = config.collectors.kubernetes
        collectors.append(
            KubernetesCollector(
                namespaces=cfg.namespaces,
                all_namespaces=cfg.all_namespaces,
                limit=cfg.limit,
                include_pods=cfg.include_pods,
                include_events=cfg.include_events,
                poll_interval_seconds=cfg.poll_interval_seconds,
            )
        )

    return collectors
