from __future__ import annotations

import re
from typing import Any

SECRET_KEYWORDS = ("password", "passwd", "secret", "token", "api_key", "apikey", "authorization", "cookie")
SECRET_REPLACEMENT = "[REDACTED]"

_BEARER_RE = re.compile(r"\bBearer\s+[A-Za-z0-9._~+/=-]+", re.IGNORECASE)
_ASSIGNMENT_RE = re.compile(
    r"(?i)\b(password|passwd|secret|token|api[_-]?key|authorization|cookie)\s*[:=]\s*([^\s,;]+)"
)


def redact_text(value: str) -> str:
    value = _BEARER_RE.sub("Bearer " + SECRET_REPLACEMENT, value)
    return _ASSIGNMENT_RE.sub(lambda match: f"{match.group(1)}={SECRET_REPLACEMENT}", value)


def redact_value(value: Any) -> Any:
    if isinstance(value, dict):
        redacted: dict[str, Any] = {}
        for key, item in value.items():
            if _is_secret_key(str(key)):
                redacted[key] = SECRET_REPLACEMENT
            else:
                redacted[key] = redact_value(item)
        return redacted
    if isinstance(value, list):
        return [redact_value(item) for item in value]
    if isinstance(value, tuple):
        return tuple(redact_value(item) for item in value)
    if isinstance(value, str):
        return redact_text(value)
    return value


def _is_secret_key(key: str) -> bool:
    normalized = key.lower().replace("-", "_")
    return any(keyword in normalized for keyword in SECRET_KEYWORDS)
