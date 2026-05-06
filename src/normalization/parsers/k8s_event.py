from __future__ import annotations

from typing import Any

from core.time import parse_datetime
from normalization.parsers.base import ParserResult, make_result, severity_from_value


def parse(data: dict[str, Any]) -> ParserResult:
    nested = _as_dict(data.get("kubernetes"))
    message = str(data.get("message") or _default_message(nested))
    severity = severity_from_value(data.get("level"))
    structured = dict(data)
    structured["kubernetes"] = nested
    tags = {"kubernetes"}
    kind = str(nested.get("kind") or "").lower()
    if kind:
        tags.add(f"k8s_{kind}")
    reason = str(nested.get("reason") or "").lower()
    if reason:
        tags.add(reason.replace(" ", "_"))
    return make_result(
        parser_name="k8s_event",
        timestamp=parse_datetime(str(data.get("timestamp") or "")),
        severity=severity,
        message=message,
        structured=structured,
        parse_confidence=0.98,
        severity_explicit=severity is not None,
        tags=tags,
    )


def _default_message(nested: dict[str, Any]) -> str:
    kind = nested.get("kind") or "object"
    name = nested.get("name") or nested.get("namespace") or "unknown"
    phase = nested.get("phase")
    if phase is not None:
        return f"kubernetes {kind} {name} phase={phase}"
    return f"kubernetes {kind} {name}"


def _as_dict(value: Any) -> dict[str, Any]:
    return dict(value) if isinstance(value, dict) else {}
