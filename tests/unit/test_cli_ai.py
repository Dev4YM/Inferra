import json

import ai
import inferra_legacy.cli as cli
from inferra_legacy.cli import CommandError, main
from config import load_config


def test_setup_writes_ai_config_without_contacting_ollama(tmp_path):
    config_path = tmp_path / "inferra.toml"
    data_dir = tmp_path / "state"

    result = main(
        [
            "--config",
            str(config_path),
            "setup",
            "--yes",
            "--model",
            "gemma4:e2b",
            "--data-dir",
            str(data_dir),
            "--skip-connection-test",
        ]
    )

    assert result == 0
    config = load_config(config_path)
    assert config.ai.enabled is True
    assert config.ai.model == "gemma4:e2b"
    assert config.storage.data_dir == data_dir
    assert (data_dir / "events.db").exists()
    assert config.experience.mode == "operator"
    assert config.experience.ai_role == "investigator"


def test_onboard_can_set_control_plane_mode_and_ai_role(tmp_path):
    config_path = tmp_path / "inferra.toml"

    result = main(
        [
            "--config",
            str(config_path),
            "onboard",
            "--yes",
            "--mode",
            "developer",
            "--ai-role",
            "researcher",
            "--skip-connection-test",
        ]
    )

    assert result == 0
    config = load_config(config_path)
    assert config.experience.mode == "developer"
    assert config.experience.ai_role == "researcher"
    assert config.experience.show_raw_evidence_by_default is True


def test_mode_set_updates_experience_mode(tmp_path, capsys):
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()

    result = main(["--json", "--config", str(config_path), "mode", "set", "developer"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["experience"]["mode"] == "developer"
    config = load_config(config_path)
    assert config.experience.mode == "developer"
    assert config.experience.show_raw_evidence_by_default is True


def test_setup_can_apply_preset_and_disable_ai(tmp_path):
    config_path = tmp_path / "inferra.toml"

    result = main(
        [
            "--config",
            str(config_path),
            "setup",
            "--yes",
            "--preset",
            "linux-node",
            "--disable-ai",
            "--base-url",
            "http://127.0.0.1:11434",
            "--skip-connection-test",
        ]
    )

    assert result == 0
    config = load_config(config_path)
    assert config.ai.enabled is False
    assert config.ai.base_url == "http://127.0.0.1:11434"
    assert config.collectors.auto_start is True
    assert config.collectors.journald.enabled is True


def test_config_set_updates_nested_value(tmp_path):
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0

    result = main(["--config", str(config_path), "config", "set", "ai.model", "gemma4:31b"])

    assert result == 0
    assert load_config(config_path).ai.model == "gemma4:31b"


def test_config_preset_updates_collector_mode(tmp_path):
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0

    result = main(["--config", str(config_path), "config", "preset", "linux-node"])

    assert result == 0
    config = load_config(config_path)
    assert config.collectors.auto_start is True
    assert config.collectors.journald.enabled is True
    assert config.collectors.windows_eventlog.enabled is False


def test_collectors_status_command_lists_configured_collectors(tmp_path, capsys, monkeypatch):
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()

    async def fake_local_api_json(config, method, path, payload=None):
        raise CommandError("No running Inferra supervisor found at http://127.0.0.1:7433/api/collectors.")

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    result = main(["--config", str(config_path), "collectors", "status"])
    output = capsys.readouterr().out

    assert result == 0
    assert "Configured collectors:" in output
    assert "queue_depth=" in output


def test_run_collectors_command_starts_and_stops(monkeypatch, tmp_path):
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0

    async def interrupt_sleep_forever():
        raise KeyboardInterrupt

    monkeypatch.setattr(cli, "_sleep_forever", interrupt_sleep_forever)

    result = main(["--config", str(config_path), "run-collectors"])

    assert result == 0


def test_ai_status_json_uses_service_payload(monkeypatch, tmp_path, capsys):
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()

    async def fake_status(self):
        return {
            "enabled": True,
            "provider": "ollama",
            "base_url": "http://127.0.0.1:11434",
            "model": "gemma4:e4b",
            "resolved_model": "gemma4:e4b-it-q4_K_M",
            "available": True,
            "installed": True,
            "reason": None,
        }

    monkeypatch.setattr(ai.AIService, "status", fake_status)

    capsys.readouterr()
    result = main(["--json", "--config", str(config_path), "ai", "status"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["available"] is True
    assert payload["resolved_model"] == "gemma4:e4b-it-q4_K_M"


def test_ai_setup_json_updates_config_and_runs_probe(monkeypatch, tmp_path, capsys):
    config_path = tmp_path / "inferra.toml"

    async def fake_status(self):
        return {
            "enabled": True,
            "provider": "ollama",
            "base_url": "http://10.0.0.12:11434",
            "model": "gemma4:31b",
            "resolved_model": "gemma4:31b-it-q4_K_M",
            "available": True,
            "installed": True,
            "reason": None,
        }

    monkeypatch.setattr(ai.AIService, "status", fake_status)

    result = main(
        [
            "--json",
            "--config",
            str(config_path),
            "ai",
            "setup",
            "--enable",
            "--model",
            "gemma4:31b",
            "--base-url",
            "http://10.0.0.12:11434",
            "--allow-remote",
            "--token-env",
            "OLLAMA_TOKEN",
        ]
    )
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    config = load_config(config_path)
    assert config.ai.enabled is True
    assert config.ai.model == "gemma4:31b"
    assert config.ai.base_url == "http://10.0.0.12:11434"
    assert config.ai.allow_remote is True
    assert config.ai.token_env == "OLLAMA_TOKEN"
    assert payload["connection_test"]["available"] is True
    assert payload["next_steps"]


def test_ai_models_json_marks_alias_as_installed(monkeypatch, tmp_path, capsys):
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()

    async def fake_installed_models(self):
        return ["gemma4:e4b-it-q4_K_M"]

    def fake_registry(self):
        return [
            {
                "name": "gemma4:e4b",
                "size": "9.6GB",
                "context_window": "128K",
                "quantization": "q4_K_M",
                "resolves_to": "gemma4:e4b-it-q4_K_M",
            }
        ]

    monkeypatch.setattr(ai.AIService, "installed_models", fake_installed_models)
    monkeypatch.setattr(ai.AIService, "registry", fake_registry)

    capsys.readouterr()
    result = main(["--json", "--config", str(config_path), "ai", "models"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["registry"][0]["installed"] is True


def test_ai_test_returns_disabled_payload(tmp_path, capsys):
    config_path = tmp_path / "inferra.toml"

    result = main(["--json", "--config", str(config_path), "ai", "test"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 1
    assert payload["enabled"] is False
    assert payload["reason"] == "AI is disabled in config."


def test_ai_pull_json_uses_non_streaming_pull(monkeypatch, tmp_path, capsys):
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0
    capsys.readouterr()
    called = {}

    async def fake_pull_model(self, model):
        called["model"] = model
        return {"status": "success"}

    monkeypatch.setattr(ai.AIService, "pull_model", fake_pull_model)

    capsys.readouterr()
    result = main(["--json", "--config", str(config_path), "ai", "pull", "gemma4:e2b"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert called["model"] == "gemma4:e2b"
    assert payload["complete"] is True
