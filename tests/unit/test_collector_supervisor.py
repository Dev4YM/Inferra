import asyncio

from collectors.base import CollectorHealth
from collectors.factory import build_collectors
from collectors.supervisor import CollectorSupervisor
from config.model import CollectorsConfig, InferraConfig, KubernetesCollectorConfig


class FlakyCollector:
    source_type = "flaky"

    def __init__(self):
        self.starts = 0
        self.running = False
        self.events = 0
        self.errors = 0

    @property
    def collector_id(self):
        return "flaky://test"

    async def start(self, sink):
        self.starts += 1
        self.running = True
        if self.starts == 1:
            self.errors += 1
            raise RuntimeError("boom")
        await asyncio.sleep(10)

    async def stop(self):
        self.running = False

    def health_check(self):
        return CollectorHealth(
            collector_id=self.collector_id,
            source_type=self.source_type,
            is_running=self.running,
            events_emitted=self.events,
            error_count=self.errors,
        )


def test_build_collectors_includes_cross_platform_and_enabled_kubernetes(monkeypatch):
    monkeypatch.setattr("collectors.factory.platform.system", lambda: "Linux")
    config = InferraConfig(
        collectors=CollectorsConfig(kubernetes=KubernetesCollectorConfig(enabled=True, limit=1))
    )

    collectors = build_collectors(config)
    source_types = {collector.source_type for collector in collectors}

    assert "host_metrics" in source_types
    assert "process_snapshot" in source_types
    assert "linux_syslog" in source_types
    assert "linux_journald" in source_types
    assert "kubernetes" in source_types
    assert "windows_eventlog" not in source_types


def test_collector_supervisor_retries_failed_collector():
    async def run():
        collector = FlakyCollector()
        supervisor = CollectorSupervisor([collector], asyncio.Queue(), retry_initial_seconds=0.01, retry_max_seconds=0.01)
        await supervisor.start()
        await asyncio.sleep(0.05)
        health = supervisor.health()[0]
        await supervisor.stop()
        return collector.starts, health

    starts, health = asyncio.run(run())

    assert starts >= 2
    assert health["attempts"] == 1
    assert health["last_error"] == "boom"
