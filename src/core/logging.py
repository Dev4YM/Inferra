from __future__ import annotations

import json
import logging
import sys
from datetime import datetime, timezone
from typing import Any


class JsonFormatter(logging.Formatter):
    def format(self, record: logging.LogRecord) -> str:
        reserved = set(vars(logging.LogRecord("", 0, "", 0, "", (), None)))
        extra = {
            key: value
            for key, value in record.__dict__.items()
            if key not in reserved and key not in {"message", "asctime"}
        }
        payload = {
            "timestamp": datetime.fromtimestamp(record.created, timezone.utc).isoformat(),
            "level": record.levelname,
            "module": record.name,
            "message": record.getMessage(),
            "extra": _json_safe(extra),
        }
        if record.exc_info:
            payload["extra"]["exception"] = self.formatException(record.exc_info)
        return json.dumps(payload, separators=(",", ":"), sort_keys=True)


def configure_logging(config: Any | None = None) -> None:
    root = logging.getLogger()
    root.handlers.clear()
    handler = logging.StreamHandler(sys.stdout)
    handler.setFormatter(JsonFormatter())
    root.addHandler(handler)
    root.setLevel(_level(getattr(getattr(config, "logging", None), "level", "INFO")))

    module_levels = getattr(getattr(config, "logging", None), "module_levels", {}) or {}
    for module, level in module_levels.items():
        logging.getLogger(module).setLevel(_level(level))


def get_logger(name: str) -> logging.Logger:
    if not logging.getLogger().handlers:
        configure_logging()
    return logging.getLogger(name)


def _level(value: str | int) -> int:
    if isinstance(value, int):
        return value
    return getattr(logging, value.upper(), logging.INFO)


def _json_safe(value: Any) -> Any:
    try:
        json.dumps(value)
        return value
    except TypeError:
        if isinstance(value, dict):
            return {key: _json_safe(item) for key, item in value.items()}
        if isinstance(value, (list, tuple, set)):
            return [_json_safe(item) for item in value]
        return str(value)
