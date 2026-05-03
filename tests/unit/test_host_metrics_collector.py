import asyncio
from types import SimpleNamespace

from collectors.host_metrics import HostMetricsCollector
from core.enums import EventType, Severity
from normalization.pipeline import NormalizationPipeline


class FakePsutil:
    def cpu_percent(self, interval=None):
        return 91.0

    def virtual_memory(self):
        return SimpleNamespace(percent=88.0)

    def disk_usage(self, path):
        return SimpleNamespace(percent=72.0, free=10 * 1024**3)

    def boot_time(self):
        return 123.0


def test_host_metrics_collector_emits_resource_pressure_event(monkeypatch):
    monkeypatch.setattr("collectors.host_metrics.psutil", FakePsutil())
    collector = HostMetricsCollector(warn_cpu_percent=85.0, warn_memory_percent=85.0, warn_disk_percent=90.0)
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_once(queue))
    event = NormalizationPipeline().normalize(queue.get_nowait())

    assert emitted == 1
    assert event.service_id == "host"
    assert event.severity == Severity.WARN
    assert event.event_type == EventType.METRIC
    assert "resource_pressure" in event.tags
    assert event.structured_data["metrics"]["cpu_percent"] == 91.0
