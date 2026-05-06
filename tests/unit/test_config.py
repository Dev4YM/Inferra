from __future__ import annotations

import json
import tomllib
from dataclasses import fields, is_dataclass
from pathlib import Path
from typing import Any, get_args, get_origin, get_type_hints

import pytest

from cli import main
from config import StorageConfig, config_to_dict, get_config_value, load_config
from config.loader import set_config_value, validate_config, write_config
from config.model import InferraConfig
from config.presets import PRESET_NAMES, apply_preset
from core.errors import ConfigError


def test_load_config_defaults_when_file_missing(tmp_path: Path) -> None:
    config = load_config(tmp_path / "missing.toml")

    assert config.server.host == "127.0.0.1"
    assert config.server.port == 7433
    assert config.storage.retention_hours == 72
    assert config.ai.provider == "ollama"
    assert config.ai.model == "gemma4:e4b"
    assert config.ai.max_retries == 2
    assert config.ai.max_tokens == 2048
    assert config.ai.stream is True
    assert config.ai.cache_ttl_seconds == 3600
    assert config.collectors.process.top_n == 20
    assert config.collectors.process.min_cpu_percent == 75.0
    assert config.collectors.host_metrics.warn_cpu_percent == 85.0
    assert config.collectors.auto_start is False
    assert config.collectors.retry_initial_seconds == 1.0
    assert config.collectors.windows_service.include_stopped is False
    assert "/var/log/syslog" in config.collectors.linux_syslog.paths
    assert config.collectors.journald.limit == 200


def test_write_and_update_ai_config(tmp_path: Path) -> None:
    config_path = tmp_path / "inferra.toml"
    config = load_config(config_path)
    write_config(config, config_path)

    updated = set_config_value(config_path, "ai.enabled", "true")
    updated = set_config_value(config_path, "ai.base_url", "https://ollama.example.test")

    assert updated.ai.enabled is True
    assert load_config(config_path).ai.base_url == "https://ollama.example.test"
    assert get_config_value(updated, "ai.enabled") is True


def test_invalid_values_are_rejected(tmp_path: Path) -> None:
    config_path = tmp_path / "inferra.toml"
    write_config(InferraConfig(), config_path)

    with pytest.raises(ConfigError):
        set_config_value(config_path, "ai.temperature", "9")
    with pytest.raises(ConfigError):
        set_config_value(config_path, "ai.top_p", "1.5")
    with pytest.raises(ConfigError):
        set_config_value(config_path, "ai.top_k", "-1")
    with pytest.raises(ConfigError):
        set_config_value(config_path, "server.port", "70000")


def test_round_trip_through_load_write_preserves_typed_config(tmp_path: Path) -> None:
    config_path = tmp_path / "inferra.toml"
    original = load_config(Path("inferra.toml"))

    write_config(original, config_path)
    reloaded = load_config(config_path)

    assert config_to_dict(reloaded) == config_to_dict(original)
    validate_config(tomllib.loads(config_path.read_text(encoding="utf-8")))


def test_preset_mutations_are_scoped_to_collectors() -> None:
    base = InferraConfig()
    before = config_to_dict(base)

    for preset in PRESET_NAMES:
        after = config_to_dict(apply_preset(base, preset))
        before_non_collectors = dict(before)
        after_non_collectors = dict(after)
        before_non_collectors.pop("collectors")
        after_non_collectors.pop("collectors")

        assert after_non_collectors == before_non_collectors


def test_check_config_json_prints_valid_report(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    config_path = tmp_path / "inferra.toml"
    write_config(InferraConfig(), config_path)

    exit_code = main(["--config", str(config_path), "check-config", "--json"])
    payload = json.loads(capsys.readouterr().out)

    assert exit_code == 0
    assert payload["ok"] is True
    assert payload["errors"] == []


def test_default_inferra_toml_contains_every_model_field() -> None:
    data = tomllib.loads(Path("inferra.toml").read_text(encoding="utf-8"))

    _assert_model_fields_present(InferraConfig, data)


def test_storage_data_dir_is_path_at_boundary(tmp_path: Path) -> None:
    config = StorageConfig(data_dir=str(tmp_path / "state"))

    assert isinstance(config.data_dir, Path)
    assert config.data_dir == tmp_path / "state"


def _assert_model_fields_present(model_type: type[Any], data: Any) -> None:
    assert isinstance(data, dict)
    hints = get_type_hints(model_type)
    for item in fields(model_type):
        assert item.name in data, f"Missing config key: {model_type.__name__}.{item.name}"
        field_type = hints[item.name]
        value = getattr(model_type(), item.name)
        if is_dataclass(value):
            _assert_model_fields_present(type(value), data[item.name])
            continue
        item_type = _list_item_type(field_type)
        if item_type is not None and data[item.name]:
            _assert_model_fields_present(item_type, data[item.name][0])


def _list_item_type(type_hint: Any) -> type[Any] | None:
    origin = get_origin(type_hint)
    if origin is not list:
        return None
    args = get_args(type_hint)
    if args and isinstance(args[0], type) and is_dataclass(args[0]):
        return args[0]
    return None
