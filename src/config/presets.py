from __future__ import annotations

from dataclasses import replace

from .model import InferraConfig

PRESET_NAMES = ("web-only", "windows-server", "linux-node", "kubernetes", "docker-host")


def apply_preset(config: InferraConfig, name: str) -> InferraConfig:
    normalized = name.strip().lower()
    if normalized == "web-only":
        collectors = replace(
            config.collectors,
            auto_start=False,
            docker=replace(config.collectors.docker, enabled=False),
            journald=replace(config.collectors.journald, enabled=False),
            file=replace(config.collectors.file, enabled=False),
            process=replace(config.collectors.process, enabled=False),
            app=replace(config.collectors.app, enabled=False),
            windows_eventlog=replace(config.collectors.windows_eventlog, enabled=False),
            host_metrics=replace(config.collectors.host_metrics, enabled=False),
            windows_service=replace(config.collectors.windows_service, enabled=False),
            linux_syslog=replace(config.collectors.linux_syslog, enabled=False),
            kubernetes=replace(config.collectors.kubernetes, enabled=False),
        )
        return replace(config, collectors=collectors)
    if normalized == "windows-server":
        collectors = replace(
            config.collectors,
            auto_start=True,
            docker=replace(config.collectors.docker, enabled=False),
            journald=replace(config.collectors.journald, enabled=False),
            process=replace(config.collectors.process, enabled=True),
            app=replace(config.collectors.app, enabled=True),
            windows_eventlog=replace(config.collectors.windows_eventlog, enabled=True),
            host_metrics=replace(config.collectors.host_metrics, enabled=True),
            windows_service=replace(config.collectors.windows_service, enabled=True),
            linux_syslog=replace(config.collectors.linux_syslog, enabled=False),
            kubernetes=replace(config.collectors.kubernetes, enabled=False),
        )
        return replace(config, collectors=collectors)
    if normalized == "linux-node":
        collectors = replace(
            config.collectors,
            auto_start=True,
            docker=replace(config.collectors.docker, enabled=False),
            journald=replace(config.collectors.journald, enabled=True),
            process=replace(config.collectors.process, enabled=True),
            app=replace(config.collectors.app, enabled=True),
            windows_eventlog=replace(config.collectors.windows_eventlog, enabled=False),
            host_metrics=replace(config.collectors.host_metrics, enabled=True),
            windows_service=replace(config.collectors.windows_service, enabled=False),
            linux_syslog=replace(config.collectors.linux_syslog, enabled=True),
            kubernetes=replace(config.collectors.kubernetes, enabled=False),
        )
        return replace(config, collectors=collectors)
    if normalized == "kubernetes":
        collectors = replace(
            config.collectors,
            auto_start=True,
            docker=replace(config.collectors.docker, enabled=False),
            journald=replace(config.collectors.journald, enabled=False),
            process=replace(config.collectors.process, enabled=True),
            app=replace(config.collectors.app, enabled=True),
            windows_eventlog=replace(config.collectors.windows_eventlog, enabled=False),
            host_metrics=replace(config.collectors.host_metrics, enabled=True),
            windows_service=replace(config.collectors.windows_service, enabled=False),
            linux_syslog=replace(config.collectors.linux_syslog, enabled=False),
            kubernetes=replace(config.collectors.kubernetes, enabled=True),
        )
        return replace(config, collectors=collectors)
    if normalized == "docker-host":
        collectors = replace(
            config.collectors,
            auto_start=True,
            docker=replace(config.collectors.docker, enabled=True),
            journald=replace(config.collectors.journald, enabled=True),
            process=replace(config.collectors.process, enabled=True),
            app=replace(config.collectors.app, enabled=True),
            windows_eventlog=replace(config.collectors.windows_eventlog, enabled=False),
            host_metrics=replace(config.collectors.host_metrics, enabled=True),
            windows_service=replace(config.collectors.windows_service, enabled=False),
            linux_syslog=replace(config.collectors.linux_syslog, enabled=True),
            kubernetes=replace(config.collectors.kubernetes, enabled=False),
        )
        return replace(config, collectors=collectors)
    raise ValueError(f"Unknown preset {name!r}. Choose one of: {', '.join(PRESET_NAMES)}")
