from __future__ import annotations

from dataclasses import replace

from config.model import InferraConfig

PRESET_NAMES = ("web-only", "windows-server", "linux-node", "kubernetes")


def apply_preset(config: InferraConfig, name: str) -> InferraConfig:
    normalized = name.strip().lower()
    if normalized == "web-only":
        return replace(config, collectors=replace(config.collectors, auto_start=False))
    if normalized == "windows-server":
        return replace(
            config,
            collectors=replace(
                config.collectors,
                auto_start=True,
                windows_eventlog=replace(config.collectors.windows_eventlog, enabled=True),
                windows_service=replace(config.collectors.windows_service, enabled=True),
                host_metrics=replace(config.collectors.host_metrics, enabled=True),
                process=replace(config.collectors.process, enabled=True),
                linux_syslog=replace(config.collectors.linux_syslog, enabled=False),
                journald=replace(config.collectors.journald, enabled=False),
                kubernetes=replace(config.collectors.kubernetes, enabled=False),
            ),
        )
    if normalized == "linux-node":
        return replace(
            config,
            collectors=replace(
                config.collectors,
                auto_start=True,
                windows_eventlog=replace(config.collectors.windows_eventlog, enabled=False),
                windows_service=replace(config.collectors.windows_service, enabled=False),
                host_metrics=replace(config.collectors.host_metrics, enabled=True),
                process=replace(config.collectors.process, enabled=True),
                linux_syslog=replace(config.collectors.linux_syslog, enabled=True),
                journald=replace(config.collectors.journald, enabled=True),
                kubernetes=replace(config.collectors.kubernetes, enabled=False),
            ),
        )
    if normalized == "kubernetes":
        return replace(
            config,
            collectors=replace(
                config.collectors,
                auto_start=True,
                windows_eventlog=replace(config.collectors.windows_eventlog, enabled=False),
                windows_service=replace(config.collectors.windows_service, enabled=False),
                linux_syslog=replace(config.collectors.linux_syslog, enabled=False),
                journald=replace(config.collectors.journald, enabled=False),
                kubernetes=replace(config.collectors.kubernetes, enabled=True),
            ),
        )
    raise ValueError(f"Unknown preset {name!r}. Choose one of: {', '.join(PRESET_NAMES)}")
