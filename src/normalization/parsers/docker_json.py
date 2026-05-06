from __future__ import annotations

from typing import Any

from core.time import parse_datetime
from normalization.parsers.base import ParserResult, make_result, severity_from_value


def parse(data: dict[str, Any]) -> ParserResult:
    message = str(data.get("log") or data.get("message") or "")
    severity = severity_from_value(data.get("level"))
    attrs = _as_dict(data.get("attrs"))
    structured = dict(data)
    if attrs:
        structured["attrs"] = attrs
    tags = {"docker"}
    stream = str(data.get("stream") or "")
    if stream:
        tags.add(f"stream_{stream}")
    return make_result(
        parser_name="docker_json",
        timestamp=parse_datetime(str(data.get("time") or data.get("timestamp") or "")),
        severity=severity,
        message=message,
        structured=structured,
        parse_confidence=0.96,
        severity_explicit=severity is not None,
        tags=tags,
    )


def _as_dict(value: Any) -> dict[str, Any]:
    return dict(value) if isinstance(value, dict) else {}
