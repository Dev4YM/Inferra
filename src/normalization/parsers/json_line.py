from __future__ import annotations

import json
from typing import Any

from core.time import parse_datetime
from normalization.parsers.base import ParserResult, make_result, severity_from_value


_TIMESTAMP_FIELDS = ("timestamp", "time", "ts", "@timestamp", "date")
_SEVERITY_FIELDS = ("level", "severity", "loglevel", "priority")
_MESSAGE_FIELDS = ("message", "msg", "text", "log")


def parse(_text: str, data: dict[str, Any]) -> ParserResult:
    timestamp = None
    for key in _TIMESTAMP_FIELDS:
        value = data.get(key)
        if value is not None:
            timestamp = parse_datetime(str(value))
            if timestamp is not None:
                break

    severity = None
    severity_explicit = False
    for key in _SEVERITY_FIELDS:
        value = data.get(key)
        if value is not None:
            severity = severity_from_value(value)
            severity_explicit = severity is not None
            if severity_explicit:
                break

    message = ""
    for key in _MESSAGE_FIELDS:
        value = data.get(key)
        if value is not None:
            message = str(value)
            break
    if not message:
        message = json.dumps(data, sort_keys=True, default=str)

    tags_value = data.get("tags")
    tags = _extract_tags(tags_value)
    structured = dict(data)
    return make_result(
        parser_name="json_line",
        timestamp=timestamp,
        severity=severity,
        message=message,
        structured=structured,
        parse_confidence=0.93,
        severity_explicit=severity_explicit,
        tags=tags,
    )


def _extract_tags(value: Any) -> set[str]:
    if isinstance(value, list | tuple | set | frozenset):
        return {str(item) for item in value if item}
    if isinstance(value, str):
        return {value} if value else set()
    return set()
