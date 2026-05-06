import asyncio
import json
import sys
from types import SimpleNamespace

import pytest

from collectors.windows_eventlog import WindowsEventLogCollector

pytestmark = pytest.mark.windows


class FakeStateStore:
    def __init__(self):
        self.values = {}

    def get_collector_state(self, collector_id, state_key):
        return self.values.get((collector_id, state_key))

    def set_collector_state(self, collector_id, state_key, state_value):
        self.values[(collector_id, state_key)] = state_value


class FakeWin32EventLog:
    EVENTLOG_BACKWARDS_READ = 1
    EVENTLOG_SEQUENTIAL_READ = 2

    def __init__(self, records):
        self.records = records

    def OpenEventLog(self, server, channel):
        return channel

    def ReadEventLog(self, handle, flags, offset):
        return self.records


class FakeWin32EventLogUtil:
    def SafeFormatMessage(self, event, channel):
        return f"{channel} event {event.RecordNumber}"


def test_windows_eventlog_collector_uses_persistent_bookmark(monkeypatch):
    records = [
        SimpleNamespace(RecordNumber=3, SourceName="App", EventID=300, EventType=1, EventCategory=0, ComputerName="host-a"),
        SimpleNamespace(RecordNumber=2, SourceName="App", EventID=200, EventType=2, EventCategory=0, ComputerName="host-a"),
        SimpleNamespace(RecordNumber=1, SourceName="App", EventID=100, EventType=4, EventCategory=0, ComputerName="host-a"),
    ]
    state = FakeStateStore()
    state.set_collector_state("windows_eventlog://local", "Application.record_number", "1")
    monkeypatch.setattr("collectors.windows_eventlog.platform.system", lambda: "Windows")
    monkeypatch.setitem(sys.modules, "win32evtlog", FakeWin32EventLog(records))
    monkeypatch.setitem(sys.modules, "win32evtlogutil", FakeWin32EventLogUtil())
    collector = WindowsEventLogCollector(channels=("Application",), state_store=state)
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_once(queue))
    first = queue.get_nowait()
    second = queue.get_nowait()

    assert emitted == 2
    assert first.metadata["record_number"] == 2
    assert second.metadata["record_number"] == 3
    assert json.loads(first.raw_payload)["windows_eventlog"]["channel"] == "Application"
    assert json.loads(first.raw_payload)["level"] == "warn"
    assert json.loads(second.raw_payload)["level"] == "error"
    assert state.get_collector_state("windows_eventlog://local", "Application.record_number") == "3"
