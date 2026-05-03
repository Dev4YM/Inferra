from __future__ import annotations

import tomllib
from dataclasses import fields, is_dataclass
from pathlib import Path
from typing import Any, TypeVar

from config.model import InferraConfig
from core.errors import ConfigError

T = TypeVar("T")


def load_config(path: str | Path | None = None) -> InferraConfig:
    if path is None:
        path = Path("inferra.toml")
    path = Path(path)
    if not path.exists():
        return InferraConfig()
    try:
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        raise ConfigError(f"Invalid TOML in {path}: {exc}") from exc
    return _build_dataclass(InferraConfig, data)


def write_config(config: InferraConfig, path: str | Path) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(dump_config(config), encoding="utf-8")


def dump_config(config: InferraConfig) -> str:
    return _to_toml(_dataclass_to_dict(config))


def set_config_value(path: str | Path, dotted_key: str, value: str) -> InferraConfig:
    path = Path(path)
    current = load_config(path)
    data = _dataclass_to_dict(current)
    target = data
    parts = dotted_key.split(".")
    if len(parts) < 2:
        raise ConfigError("Config keys must use a dotted section key, for example ai.model")
    for part in parts[:-1]:
        if part not in target or not isinstance(target[part], dict):
            raise ConfigError(f"Unknown config section: {part}")
        target = target[part]
    leaf = parts[-1]
    if leaf not in target:
        raise ConfigError(f"Unknown config key: {dotted_key}")
    target[leaf] = _coerce_value(value, target[leaf])
    updated = _build_dataclass(InferraConfig, data)
    write_config(updated, path)
    return updated


def _build_dataclass(cls: type[T], data: dict[str, Any]) -> T:
    kwargs: dict[str, Any] = {}
    for f in fields(cls):
        if f.name not in data:
            continue
        value = data[f.name]
        default_value = getattr(cls(), f.name)
        if is_dataclass(default_value):
            kwargs[f.name] = _build_dataclass(type(default_value), value or {})
        elif isinstance(default_value, Path):
            kwargs[f.name] = Path(value)
        elif isinstance(default_value, tuple):
            kwargs[f.name] = tuple(value or ())
        else:
            kwargs[f.name] = value
    return cls(**kwargs)


def _dataclass_to_dict(value: Any) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for f in fields(value):
        item = getattr(value, f.name)
        if is_dataclass(item):
            result[f.name] = _dataclass_to_dict(item)
        elif isinstance(item, Path):
            result[f.name] = str(item)
        elif isinstance(item, tuple):
            result[f.name] = list(item)
        else:
            result[f.name] = item
    return result


def _coerce_value(raw: str, current: Any) -> Any:
    if isinstance(current, bool):
        lowered = raw.strip().lower()
        if lowered in {"1", "true", "yes", "on"}:
            return True
        if lowered in {"0", "false", "no", "off"}:
            return False
        raise ConfigError(f"Expected boolean value, got {raw!r}")
    if isinstance(current, int) and not isinstance(current, bool):
        return int(raw)
    if isinstance(current, float):
        return float(raw)
    if isinstance(current, Path):
        return Path(raw)
    if isinstance(current, list):
        return [part.strip() for part in raw.split(",") if part.strip()]
    if isinstance(current, tuple):
        return tuple(part.strip() for part in raw.split(",") if part.strip())
    return raw


def _to_toml(data: dict[str, Any]) -> str:
    lines: list[str] = []
    for section, values in data.items():
        if not isinstance(values, dict):
            continue
        _write_section(lines, section, values)
    return "\n".join(lines).rstrip() + "\n"


def _write_section(lines: list[str], prefix: str, values: dict[str, Any]) -> None:
    scalars = {key: value for key, value in values.items() if not isinstance(value, dict)}
    nested = {key: value for key, value in values.items() if isinstance(value, dict)}
    lines.append(f"[{prefix}]")
    for key, value in scalars.items():
        lines.append(f"{key} = {_format_toml_value(value)}")
    lines.append("")
    for key, value in nested.items():
        _write_section(lines, f"{prefix}.{key}", value)


def _format_toml_value(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (int, float)):
        return str(value)
    if isinstance(value, (list, tuple)):
        return "[" + ", ".join(_format_toml_value(item) for item in value) + "]"
    return '"' + str(value).replace("\\", "\\\\").replace('"', '\\"') + '"'
