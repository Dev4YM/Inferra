from __future__ import annotations

import json
from dataclasses import dataclass

import cli
import windows_service
from cli import main


@dataclass
class _Completed:
    returncode: int
    stdout: str = ""
    stderr: str = ""


def test_service_status_json_reports_runtime_and_sc_state(monkeypatch, tmp_path, capsys):
    runtime = windows_service.ServiceRuntimeOptions(
        config_path=tmp_path / "inferra.toml",
        data_dir=tmp_path / "state",
    )

    monkeypatch.setattr(cli.platform, "system", lambda: "Windows")
    monkeypatch.setattr(windows_service, "read_service_runtime", lambda: runtime)
    monkeypatch.setattr(windows_service, "serve_log_path", lambda: tmp_path / "logs" / "serve.log")

    def fake_run(command):
        if command[:2] == ["sc.exe", "query"]:
            return _Completed(
                0,
                "SERVICE_NAME: Inferra\n        STATE              : 4  RUNNING\n",
            )
        if command[:2] == ["sc.exe", "qc"]:
            return _Completed(
                0,
                "SERVICE_NAME: Inferra\n        START_TYPE         : 2   AUTO_START\n",
            )
        raise AssertionError(command)

    monkeypatch.setattr(cli, "_run_subprocess_capture", fake_run)

    result = main(["--json", "service", "status"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["supported"] is True
    assert payload["installed"] is True
    assert payload["state"] == "running"
    assert payload["startup"] == "auto_start"
    assert payload["config_path"] == str(runtime.config_path)
    assert payload["data_dir"] == str(runtime.data_dir)


def test_service_install_json_invokes_windows_service_helper(monkeypatch, tmp_path, capsys):
    config_path = tmp_path / "inferra.toml"
    data_dir = tmp_path / "state"
    calls: dict[str, list[str]] = {}

    monkeypatch.setattr(cli.platform, "system", lambda: "Windows")

    def fake_run(command):
        calls["command"] = command
        return _Completed(0)

    monkeypatch.setattr(cli, "_run_subprocess_capture", fake_run)

    result = main(
        [
            "--json",
            "--config",
            str(config_path),
            "service",
            "--data-dir",
            str(data_dir),
            "install",
            "--startup",
            "manual",
        ]
    )
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["startup"] == "manual"
    assert calls["command"][:4] == [cli.sys.executable, "-m", "windows_service", "install"]
    assert "--config" in calls["command"]
    assert str(config_path) in calls["command"]
    assert "--data-dir" in calls["command"]
    assert str(data_dir) in calls["command"]
