from __future__ import annotations

import os
import tomllib
from copy import deepcopy
from dataclasses import fields, is_dataclass
from pathlib import Path
from typing import Any

from pydantic import TypeAdapter, ValidationError

from .models import InferraConfig

try:
    import tomli_w
except ModuleNotFoundError:  # pragma: no cover - exercised only in source trees missing optional dev deps
    tomli_w = None  # type: ignore[assignment]

try:
    from core.errors import ConfigError
except ModuleNotFoundError:  # pragma: no cover - package import path
    from ..core.errors import ConfigError

_CONFIG_ADAPTER: TypeAdapter[InferraConfig] = TypeAdapter(InferraConfig)


def load_config(path: str | Path | None = None) -> InferraConfig:
    config_path = Path(path or "inferra.toml")
    if not config_path.exists():
        return _apply_env_overrides(InferraConfig())
    try:
        data = tomllib.loads(config_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        raise ConfigError(f"Invalid TOML in {config_path}: {exc}") from exc
    config = _validate_data(data)
    return _apply_env_overrides(config)


def validate_config(config: InferraConfig | dict[str, Any]) -> None:
    if isinstance(config, InferraConfig):
        _validate_data(config_to_dict(config))
        return
    _validate_data(config)


def parse_config_payload(data: dict[str, Any]) -> InferraConfig:
    return _validate_data(_normalize_legacy_config(deepcopy(data)))


def write_config(config: InferraConfig, path: str | Path) -> None:
    updated = _validate_data(config_to_dict(config))
    config_path = Path(path)
    config_path.parent.mkdir(parents=True, exist_ok=True)
    config_path.write_text(dump_config(updated), encoding="utf-8")


def dump_config(config: InferraConfig) -> str:
    data = config_to_dict(_validate_data(config_to_dict(config)))
    if tomli_w is not None:
        return tomli_w.dumps(data)
    return _to_toml(data)


def config_to_dict(config: InferraConfig) -> dict[str, Any]:
    return _dataclass_to_dict(config)


def get_config_value(config: InferraConfig, dotted_key: str) -> Any:
    target: Any = config_to_dict(config)
    for part in _split_dotted_key(dotted_key):
        if not isinstance(target, dict) or part not in target:
            raise ConfigError(f"Unknown config key: {dotted_key}")
        target = target[part]
    return target


def set_config_value(path: str | Path, dotted_key: str, value: str) -> InferraConfig:
    config_path = Path(path)
    current = load_config(config_path)
    data = config_to_dict(current)
    target = data
    parts = _split_dotted_key(dotted_key)
    if len(parts) < 2:
        raise ConfigError("Config keys must use a dotted section key, for example ai.model")
    for part in parts[:-1]:
        if part not in target or not isinstance(target[part], dict):
            raise ConfigError(f"Unknown config section: {part}")
        target = target[part]
    leaf = parts[-1]
    if leaf not in target:
        raise ConfigError(f"Unknown config key: {dotted_key}")
    target[leaf] = _coerce_cli_value(value, target[leaf])
    updated = _validate_data(data)
    write_config(updated, config_path)
    return updated


def _validate_data(data: dict[str, Any]) -> InferraConfig:
    try:
        return _CONFIG_ADAPTER.validate_python(_normalize_legacy_config(data))
    except ValidationError as exc:
        errors = "; ".join(_format_validation_error(error) for error in exc.errors())
        raise ConfigError(f"Invalid Inferra config: {errors}") from exc


def _normalize_legacy_config(data: dict[str, Any]) -> dict[str, Any]:
    normalized = deepcopy(data)
    collectors = normalized.get("collectors")
    if isinstance(collectors, dict):
        legacy_file = collectors.get("file")
        if isinstance(legacy_file, list):
            collectors["file"] = {"entries": legacy_file, "paths": _paths_from_file_entries(legacy_file)}
        if "procfs" in collectors and "process" not in collectors:
            collectors["process"] = collectors.pop("procfs")
    return normalized


def _paths_from_file_entries(entries: list[Any]) -> list[str]:
    paths: list[str] = []
    for entry in entries:
        if isinstance(entry, dict) and entry.get("path"):
            paths.append(str(entry["path"]))
    return paths


def _dataclass_to_dict(value: Any) -> Any:
    if value is None:
        return ""
    if is_dataclass(value):
        result: dict[str, Any] = {}
        for item in fields(value):
            result[item.name] = _dataclass_to_dict(getattr(value, item.name))
        return result
    if isinstance(value, Path):
        text = value.as_posix()
        if not value.is_absolute() and not text.startswith((".", "/")):
            return f"./{text}"
        return text
    if isinstance(value, tuple):
        return [_dataclass_to_dict(item) for item in value]
    if isinstance(value, list):
        return [_dataclass_to_dict(item) for item in value]
    if isinstance(value, dict):
        return {str(key): _dataclass_to_dict(item) for key, item in value.items()}
    return value


def _apply_env_overrides(config: InferraConfig) -> InferraConfig:
    data = config_to_dict(config)
    if "INFERRA_PORT" in os.environ:
        data["server"]["port"] = _coerce_cli_value(os.environ["INFERRA_PORT"], data["server"]["port"])
    if "INFERRA_DATA_DIR" in os.environ:
        data["storage"]["data_dir"] = os.environ["INFERRA_DATA_DIR"]
    if "INFERRA_LLM_PROVIDER" in os.environ:
        provider = os.environ["INFERRA_LLM_PROVIDER"]
        data["explanation"]["provider"] = provider
        data["ai"]["provider"] = provider
    return _validate_data(data)


def _split_dotted_key(dotted_key: str) -> list[str]:
    parts = [part.strip() for part in dotted_key.split(".") if part.strip()]
    if not parts:
        raise ConfigError("Config key cannot be empty")
    return parts


def _coerce_cli_value(raw: str, current: Any) -> Any:
    parsed = _parse_toml_scalar(raw)
    if isinstance(current, bool):
        if isinstance(parsed, bool):
            return parsed
        lowered = raw.strip().lower()
        if lowered in {"1", "true", "yes", "on"}:
            return True
        if lowered in {"0", "false", "no", "off"}:
            return False
        raise ConfigError(f"Expected boolean value, got {raw!r}")
    if isinstance(current, int) and not isinstance(current, bool):
        return int(parsed)
    if isinstance(current, float):
        return float(parsed)
    if isinstance(current, list):
        if isinstance(parsed, list):
            return parsed
        return [part.strip() for part in str(parsed).split(",") if part.strip()]
    if isinstance(current, tuple):
        if isinstance(parsed, list):
            return parsed
        return [part.strip() for part in str(parsed).split(",") if part.strip()]
    return parsed


def _parse_toml_scalar(raw: str) -> Any:
    try:
        return tomllib.loads(f"value = {raw}")["value"]
    except tomllib.TOMLDecodeError:
        return raw


def _format_validation_error(error: dict[str, Any]) -> str:
    location = ".".join(str(part) for part in error.get("loc", ())) or "config"
    return f"{location}: {error.get('msg', 'invalid value')}"


def _to_toml(data: dict[str, Any]) -> str:
    lines: list[str] = []
    for section, values in data.items():
        if isinstance(values, dict):
            _write_section(lines, section, values)
    return "\n".join(lines).rstrip() + "\n"


def _write_section(lines: list[str], prefix: str, values: dict[str, Any]) -> None:
    scalars = {key: value for key, value in values.items() if not isinstance(value, dict) and not _is_array_table(value)}
    nested = {key: value for key, value in values.items() if isinstance(value, dict)}
    arrays = {key: value for key, value in values.items() if _is_array_table(value)}
    lines.append(f"[{prefix}]")
    for key, value in scalars.items():
        lines.append(f"{key} = {_format_toml_value(value)}")
    lines.append("")
    for key, value in nested.items():
        _write_section(lines, f"{prefix}.{key}", value)
    for key, rows in arrays.items():
        for row in rows:
            lines.append(f"[[{prefix}.{key}]]")
            for row_key, row_value in row.items():
                lines.append(f"{row_key} = {_format_toml_value(row_value)}")
            lines.append("")


def _format_toml_value(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    if isinstance(value, list):
        return "[" + ", ".join(_format_toml_value(item) for item in value) + "]"
    return '"' + str(value).replace("\\", "\\\\").replace('"', '\\"') + '"'


def _is_array_table(value: Any) -> bool:
    return isinstance(value, list) and bool(value) and all(isinstance(item, dict) for item in value)
