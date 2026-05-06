import asyncio
import json
from types import SimpleNamespace

import pytest

from collectors.journald import JournaldCollector
from collectors.linux_syslog import LinuxSyslogCollector
from core.enums import Severity
from normalization.pipeline import NormalizationPipeline

pytestmark = pytest.mark.linux


class FakeStateStore:
    def __init__(self):
        self.values = {}

    def get_collector_state(self, collector_id, state_key):
        return self.values.get((collector_id, state_key))

    def set_collector_state(self, collector_id, state_key, state_value):
        self.values[(collector_id, state_key)] = state_value


def test_linux_syslog_collector_parses_rfc3164_lines(tmp_path):
    path = tmp_path / "syslog"
    path.write_text(
        "May  3 06:01:02 host-a sshd[123]: Failed password for invalid user root\n"
        "May  3 06:01:03 host-a kernel: disk warning threshold reached\n",
        encoding="utf-8",
    )
    collector = LinuxSyslogCollector(paths=(str(path),), start_at_end=False)
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_once(queue))
    first = NormalizationPipeline().normalize(queue.get_nowait())
    second = NormalizationPipeline().normalize(queue.get_nowait())

    assert emitted == 2
    assert first.service_id == "sshd"
    assert first.severity == Severity.ERROR
    assert first.host_id == "host-a"
    assert second.service_id == "kernel"


def test_journald_collector_reads_json_and_persists_cursor():
    rows = [
        {
            "__CURSOR": "cursor-1",
            "__REALTIME_TIMESTAMP": "1777780000000000",
            "PRIORITY": "3",
            "MESSAGE": "postgres connection failed",
            "_SYSTEMD_UNIT": "postgresql.service",
            "_HOSTNAME": "linux-a",
            "_PID": "99",
        }
    ]
    calls = []

    def runner(command, capture_output, text, check):
        calls.append(command)
        return SimpleNamespace(returncode=0, stdout="\n".join(json.dumps(row) for row in rows), stderr="")

    state = FakeStateStore()
    collector = JournaldCollector(units=("postgresql.service",), state_store=state, command_runner=runner)
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_once(queue))
    event = NormalizationPipeline().normalize(queue.get_nowait())

    assert emitted == 1
    assert event.service_id == "postgresql"
    assert event.severity == Severity.ERROR
    assert state.get_collector_state("linux_journald://postgresql.service", "cursor") == "cursor-1"
    assert "-u" in calls[0]


def test_journald_collector_uses_after_cursor_on_next_read():
    calls = []

    def runner(command, capture_output, text, check):
        calls.append(command)
        return SimpleNamespace(returncode=0, stdout="", stderr="")

    state = FakeStateStore()
    state.set_collector_state("linux_journald://system", "cursor", "cursor-old")
    collector = JournaldCollector(state_store=state, command_runner=runner)

    emitted = asyncio.run(collector.collect_once(asyncio.Queue()))

    assert emitted == 0
    assert "--after-cursor" in calls[0]
    assert "cursor-old" in calls[0]
