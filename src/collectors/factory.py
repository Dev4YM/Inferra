from __future__ import annotations

import platform
from pathlib import Path
from typing import Any

from collectors.base import Collector
from collectors.docker import DockerCollector
from collectors.file import FileCollector
from collectors.host_metrics import HostMetricsCollector
from collectors.journald import JournaldCollector
from collectors.kubernetes import KubernetesCollector
from collectors.linux_syslog import LinuxSyslogCollector
from collectors.process_snapshot import ProcessSnapshotCollector
from collectors.app_http import AppHttpCollector
from collectors.windows_eventlog import WindowsEventLogCollector
from collectors.windows_service import WindowsServiceCollector
from config.model import InferraConfig


def build_collectors(config: InferraConfig, state_store: Any | None = None) -> list[Collector]:
    collectors: list[Collector] = []
    current_platform = platform.system().lower()
    metrics_dir = Path(config.storage.data_dir) / "metrics"

    if config.collectors.host_metrics.enabled:
        cfg = config.collectors.host_metrics
        collectors.append(
            HostMetricsCollector(
                poll_interval_seconds=cfg.poll_interval_seconds,
                warn_cpu_percent=cfg.warn_cpu_percent,
                warn_memory_percent=cfg.warn_memory_percent,
                warn_disk_percent=cfg.warn_disk_percent,
                metrics_dir=metrics_dir,
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
                watch_processes=tuple(cfg.watch_processes),
                watch_pids=tuple(cfg.watch_pids),
                metrics_dir=metrics_dir,
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
        for entry in cfg.entries:
            if entry.path or entry.glob:
                collectors.append(
                    FileCollector(
                        entry.path,
                        glob_pattern=entry.glob or None,
                        service_id=entry.service_id,
                        service_id_from_filename=entry.service_id_from_filename,
                        multiline_pattern=entry.multiline_pattern or None,
                        poll_interval_seconds=cfg.poll_interval_seconds,
                        start_at_end=cfg.start_at_end,
                    )
                )

    if config.collectors.app.enabled and (
        config.collectors.app.enable_main_api or config.collectors.app.enable_standalone
    ):
        cfg = config.collectors.app
        collectors.append(
            AppHttpCollector(
                listen=cfg.listen,
                max_payload_bytes=cfg.max_payload_bytes,
                shared_token=cfg.shared_token,
                mount_path=cfg.mount_path,
                enable_main_api=cfg.enable_main_api,
                enable_standalone=cfg.enable_standalone,
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

    docker_cfg = config.collectors.docker
    docker_supported = current_platform == "linux" or docker_cfg.socket.startswith(("tcp://", "http://", "https://"))
    if docker_cfg.enabled and docker_supported:
        collectors.append(
            DockerCollector(
                socket=docker_cfg.socket,
                include_names=tuple(docker_cfg.include_names),
                include_labels=tuple(docker_cfg.include_labels),
                exclude_names=tuple(docker_cfg.exclude_names),
                include_all=docker_cfg.include_all,
                state_store=state_store,
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
                    exclude_units=cfg.exclude_units,
                    min_priority=cfg.min_priority,
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
                label_selector=cfg.label_selector or None,
                limit=cfg.limit,
                include_pods=cfg.include_pods,
                include_events=cfg.include_events,
                poll_interval_seconds=cfg.poll_interval_seconds,
            )
        )

    return collectors
