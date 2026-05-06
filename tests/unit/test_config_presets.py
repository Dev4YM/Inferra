from __future__ import annotations

from config.model import InferraConfig
from config.presets import apply_preset


def test_windows_server_preset_enables_windows_collectors_and_autostart():
    config = apply_preset(InferraConfig(), "windows-server")

    assert config.collectors.auto_start is True
    assert config.collectors.windows_eventlog.enabled is True
    assert config.collectors.windows_service.enabled is True
    assert config.collectors.linux_syslog.enabled is False
    assert config.collectors.kubernetes.enabled is False


def test_kubernetes_preset_enables_kubernetes_collector():
    config = apply_preset(InferraConfig(), "kubernetes")

    assert config.collectors.auto_start is True
    assert config.collectors.kubernetes.enabled is True
    assert config.collectors.windows_eventlog.enabled is False


def test_web_only_preset_keeps_collectors_manual():
    config = apply_preset(apply_preset(InferraConfig(), "linux-node"), "web-only")

    assert config.collectors.auto_start is False
    assert config.collectors.host_metrics.enabled is False
    assert config.collectors.app.enabled is False


def test_linux_node_preset_enables_linux_collectors() -> None:
    config = apply_preset(InferraConfig(), "linux-node")

    assert config.collectors.auto_start is True
    assert config.collectors.linux_syslog.enabled is True
    assert config.collectors.journald.enabled is True
    assert config.collectors.windows_eventlog.enabled is False
    assert config.collectors.kubernetes.enabled is False


def test_docker_host_preset_enables_docker_collector() -> None:
    config = apply_preset(InferraConfig(), "docker-host")

    assert config.collectors.auto_start is True
    assert config.collectors.docker.enabled is True
    assert config.collectors.host_metrics.enabled is True
    assert config.collectors.linux_syslog.enabled is True
    assert config.collectors.kubernetes.enabled is False
