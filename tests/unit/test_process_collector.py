import asyncio
from types import SimpleNamespace

from collectors.process_snapshot import ProcessSnapshotCollector
from normalization.pipeline import NormalizationPipeline


class FakeProcess:
    def __init__(self, info):
        self.info = info


class FakePsutil:
    NoSuchProcess = RuntimeError
    AccessDenied = PermissionError
    ZombieProcess = RuntimeError

    def __init__(self, processes):
        self._processes = processes

    def process_iter(self, attrs=None):
        return iter(self._processes)


def test_process_snapshot_collector_filters_and_orders(monkeypatch):
    processes = [
        FakeProcess(
            {
                "pid": 10,
                "name": "api",
                "username": "user",
                "status": "running",
                "cpu_percent": 82.5,
                "memory_info": SimpleNamespace(rss=128 * 1024 * 1024),
                "create_time": 1.0,
                "cmdline": ["python", "api.py"],
            }
        ),
        FakeProcess(
            {
                "pid": 11,
                "name": "worker",
                "username": "user",
                "status": "sleeping",
                "cpu_percent": 10.0,
                "memory_info": SimpleNamespace(rss=900 * 1024 * 1024),
                "create_time": 2.0,
                "cmdline": ["python", "worker.py"],
            }
        ),
        FakeProcess(
            {
                "pid": 12,
                "name": "idle",
                "username": "user",
                "status": "sleeping",
                "cpu_percent": 1.0,
                "memory_info": SimpleNamespace(rss=32 * 1024 * 1024),
                "create_time": 3.0,
                "cmdline": ["idle"],
            }
        ),
    ]
    monkeypatch.setattr("collectors.process_snapshot.psutil", FakePsutil(processes))
    collector = ProcessSnapshotCollector(top_n=2, min_cpu_percent=75.0, min_memory_mb=512.0)

    snapshots = collector._snapshots()

    assert [item.name for item in snapshots] == ["api", "worker"]


def test_process_snapshot_collect_once_emits_normalizable_events(monkeypatch):
    processes = [
        FakeProcess(
            {
                "pid": 10,
                "name": "api",
                "username": "user",
                "status": "running",
                "cpu_percent": 82.5,
                "memory_info": SimpleNamespace(rss=128 * 1024 * 1024),
                "create_time": 1.0,
                "cmdline": ["python", "api.py"],
            }
        )
    ]
    monkeypatch.setattr("collectors.process_snapshot.psutil", FakePsutil(processes))
    collector = ProcessSnapshotCollector(top_n=1, min_cpu_percent=75.0, min_memory_mb=512.0)
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_once(queue))
    raw = queue.get_nowait()
    event = NormalizationPipeline().normalize(raw)

    assert emitted == 1
    assert raw.source_type == "process_snapshot"
    assert event.service_id == "api"
    assert "high cpu" in event.message
    assert "process" in event.structured_data
