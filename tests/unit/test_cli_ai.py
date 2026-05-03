from cli import main
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


def test_collectors_status_command_lists_configured_collectors(tmp_path):
    config_path = tmp_path / "inferra.toml"
    assert main(["--config", str(config_path), "setup", "--yes", "--skip-connection-test"]) == 0

    result = main(["--config", str(config_path), "collectors", "status"])

    assert result == 0
