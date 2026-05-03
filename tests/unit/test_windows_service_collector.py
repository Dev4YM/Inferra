import asyncio

from collectors.windows_service import WindowsServiceCollector
from core.enums import EventType, Severity
from normalization.pipeline import NormalizationPipeline


class FakeService:
    def __init__(self, info):
        self._info = info

    def as_dict(self):
        return self._info


class FakePsutil:
    def __init__(self, services):
        self._services = services

    def win_service_iter(self):
        return iter(self._services)


def test_windows_service_collector_filters_and_normalizes(monkeypatch):
    services = [
        FakeService(
            {
                "name": "Spooler",
                "display_name": "Print Spooler",
                "status": "running",
                "start_type": "automatic",
                "pid": 123,
                "username": "LocalSystem",
                "binpath": "spoolsv.exe",
            }
        ),
        FakeService(
            {
                "name": "StoppedManual",
                "display_name": "Stopped Manual",
                "status": "stopped",
                "start_type": "manual",
                "pid": None,
                "username": None,
                "binpath": None,
            }
        ),
    ]
    monkeypatch.setattr("collectors.windows_service.platform.system", lambda: "Windows")
    monkeypatch.setattr("collectors.windows_service.psutil", FakePsutil(services))
    collector = WindowsServiceCollector(include_stopped=False)
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_once(queue))
    event = NormalizationPipeline().normalize(queue.get_nowait())

    assert emitted == 1
    assert event.service_id == "spooler"
    assert event.event_type == EventType.STATE_CHANGE
    assert event.severity == Severity.INFO
    assert event.structured_data["windows_service"]["status"] == "running"


def test_windows_service_collector_marks_automatic_stopped_as_error(monkeypatch):
    services = [
        FakeService(
            {
                "name": "MSSQLSERVER",
                "display_name": "SQL Server",
                "status": "stopped",
                "start_type": "automatic",
                "pid": None,
                "username": None,
                "binpath": None,
            }
        )
    ]
    monkeypatch.setattr("collectors.windows_service.platform.system", lambda: "Windows")
    monkeypatch.setattr("collectors.windows_service.psutil", FakePsutil(services))
    collector = WindowsServiceCollector(include_stopped=True)
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_once(queue))
    event = NormalizationPipeline().normalize(queue.get_nowait())

    assert emitted == 1
    assert event.service_id == "mssqlserver"
    assert event.severity == Severity.ERROR
    assert "status=stopped" in event.message
