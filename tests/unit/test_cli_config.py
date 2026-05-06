from __future__ import annotations

import json

from cli import main


def test_config_show_json_returns_typed_config(tmp_path, capsys) -> None:
    config_path = tmp_path / "inferra.toml"

    result = main(["--json", "--config", str(config_path), "config", "show"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["config"]["server"]["host"] == "127.0.0.1"
    assert payload["config"]["ai"]["model"] == "gemma4:e4b"


def test_config_get_json_returns_selected_value(tmp_path, capsys) -> None:
    config_path = tmp_path / "inferra.toml"

    result = main(["--json", "--config", str(config_path), "config", "get", "ai.model"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["key"] == "ai.model"
    assert payload["value"] == "gemma4:e4b"


def test_config_set_json_reports_updated_value(tmp_path, capsys) -> None:
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()

    result = main(["--json", "--config", str(config_path), "config", "set", "ai.enabled", "false"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["key"] == "ai.enabled"
    assert payload["value"] is False


def test_config_preset_json_returns_collector_changes(tmp_path, capsys) -> None:
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()

    result = main(["--json", "--config", str(config_path), "config", "preset", "windows-server"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["preset"] == "windows-server"
    assert payload["collectors"]["auto_start"] is True
    assert payload["collectors"]["windows_eventlog"]["enabled"] is True
