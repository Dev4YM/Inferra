from config import load_config
from config.loader import set_config_value, write_config


def test_load_config_defaults_when_file_missing(tmp_path):
    config = load_config(tmp_path / "missing.toml")

    assert config.server.host == "127.0.0.1"
    assert config.server.port == 7433
    assert config.storage.retention_hours == 72
    assert config.ai.provider == "ollama"
    assert config.ai.model == "gemma4:e4b"
    assert config.collectors.process.top_n == 20
    assert config.collectors.process.min_cpu_percent == 75.0
    assert config.collectors.host_metrics.warn_cpu_percent == 85.0
    assert config.collectors.auto_start is False
    assert config.collectors.retry_initial_seconds == 1.0
    assert config.collectors.windows_service.include_stopped is False
    assert "/var/log/syslog" in config.collectors.linux_syslog.paths
    assert config.collectors.journald.limit == 200


def test_write_and_update_ai_config(tmp_path):
    config_path = tmp_path / "inferra.toml"
    config = load_config(config_path)
    write_config(config, config_path)

    updated = set_config_value(config_path, "ai.enabled", "true")
    updated = set_config_value(config_path, "ai.base_url", "https://ollama.example.test")

    assert updated.ai.enabled is True
    assert load_config(config_path).ai.base_url == "https://ollama.example.test"
