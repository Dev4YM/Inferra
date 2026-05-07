from __future__ import annotations

import json

import pytest

import inferra_legacy.cli as cli
from inferra_legacy.cli import CommandError, main
from collectors.host_metrics import HostMetricsCollector


def test_collectors_status_json_falls_back_to_configured_collectors(monkeypatch: pytest.MonkeyPatch, tmp_path, capsys) -> None:
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()

    async def fake_local_api_json(config, method, path, payload=None):
        raise CommandError("No running Inferra supervisor found at http://127.0.0.1:7433/api/collectors.")

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    result = main(["--json", "--config", str(config_path), "collectors", "status"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["mode"] == "configured"
    assert payload["running"] is False
    assert payload["collectors"]


def test_collectors_start_requires_running_daemon(monkeypatch: pytest.MonkeyPatch, tmp_path, capsys) -> None:
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0

    async def fake_require_local_api(config, method, path, payload=None):
        raise CommandError("No running Inferra supervisor found at http://127.0.0.1:7433/api/collectors/start.")

    monkeypatch.setattr(cli, "_require_local_api", fake_require_local_api)

    result = main(["--config", str(config_path), "collectors", "start"])
    error = capsys.readouterr().err

    assert result == 1
    assert "No running Inferra supervisor found" in error


def test_collectors_stop_json_uses_local_api_payload(monkeypatch: pytest.MonkeyPatch, tmp_path, capsys) -> None:
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()

    async def fake_require_local_api(config, method, path, payload=None):
        return {"stopped": True, "collectors": [], "queue_depth": 0}

    monkeypatch.setattr(cli, "_require_local_api", fake_require_local_api)

    result = main(["--json", "--config", str(config_path), "collectors", "stop"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["stopped"] is True
    assert payload["server_url"] == "http://127.0.0.1:7433"


def test_collect_host_runs_one_shot_pipeline(monkeypatch: pytest.MonkeyPatch, tmp_path, capsys) -> None:
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()

    def fake_metrics(self: HostMetricsCollector) -> dict[str, float]:
        return {
            "cpu_percent": 100.0,
            "memory_percent": 95.0,
            "disk_percent": 97.0,
            "disk_free_gb": 1.0,
            "boot_time": 0.0,
        }

    monkeypatch.setattr(HostMetricsCollector, "_metrics", fake_metrics)

    result = main(["--json", "--config", str(config_path), "collect-host"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["source_type"] == "host_metrics"
    assert payload["raw_events_emitted"] >= 1
    assert payload["events_stored"] >= 1
