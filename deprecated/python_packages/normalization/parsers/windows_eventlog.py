from __future__ import annotations

from typing import Any

from core.time import parse_datetime
from normalization.parsers.base import ParserResult, make_result, severity_from_value


def parse(data: dict[str, Any]) -> ParserResult:
    nested = _as_dict(data.get("windows_eventlog"))
    message = str(data.get("message") or nested.get("message") or "windows event log entry")
    severity = severity_from_value(data.get("level") or nested.get("level") or nested.get("event_type"))
    structured = dict(data)
    structured["windows_eventlog"] = nested
    return make_result(
        parser_name="windows_eventlog",
        timestamp=parse_datetime(str(data.get("timestamp") or "")),
        severity=severity,
        message=message,
        structured=structured,
        parse_confidence=0.99,
        severity_explicit=severity is not None,
        tags={"windows_eventlog"},
    )


def _as_dict(value: Any) -> dict[str, Any]:
    return dict(value) if isinstance(value, dict) else {}
