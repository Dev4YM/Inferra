from __future__ import annotations

import re
from typing import Any

from core.enums import Severity
from core.time import parse_datetime
from normalization.parsers.base import ParserResult, infer_severity, make_result, severity_from_value


_RFC5424_RE = re.compile(
    r"^<(?P<priority>\d{1,3})>(?P<version>\d)\s+"
    r"(?P<timestamp>\S+)\s+"
    r"(?P<host>\S+)\s+"
    r"(?P<app>\S+)\s+"
    r"(?P<procid>\S+)\s+"
    r"(?P<msgid>\S+)\s+"
    r"(?P<structured_data>(?:-|\[[^\]]*\](?:\[[^\]]*\])*))"
    r"(?:\s+(?P<message>.*))?$"
)
_SD_ELEMENT_RE = re.compile(r"\[(?P<sdid>[^\s\]]+)(?P<body>[^\]]*)\]")
_SD_PARAM_RE = re.compile(r'(?P<key>[A-Za-z0-9._-]+)="(?P<value>(?:[^"\\]|\\.)*)"')


def parse(text: str) -> ParserResult:
    match = _RFC5424_RE.match(text.strip())
    if match is None:
        return make_result(
            parser_name="syslog_rfc5424",
            timestamp=None,
            severity=None,
            message=text,
            structured={"raw": text},
            parse_confidence=0.2,
            flags={"syslog_rfc5424_mismatch"},
        )

    priority = int(match.group("priority"))
    inferred = infer_severity(match.group("message") or "")
    severity = severity_from_value(priority % 8) or inferred
    if severity == Severity.CRITICAL and inferred >= Severity.ERROR and inferred < Severity.CRITICAL:
        severity = inferred
    structured = {
        "priority": priority,
        "version": int(match.group("version")),
        "host": match.group("host"),
        "app": _none_if_nil(match.group("app")),
        "procid": _none_if_nil(match.group("procid")),
        "msgid": _none_if_nil(match.group("msgid")),
        "structured_data": _parse_structured_data(match.group("structured_data")),
        "syslog_format": "rfc5424",
    }
    return make_result(
        parser_name="syslog_rfc5424",
        timestamp=parse_datetime(match.group("timestamp")),
        severity=severity,
        message=match.group("message") or "",
        structured=structured,
        parse_confidence=0.97,
        tags={"syslog"},
    )


def _parse_structured_data(raw: str) -> dict[str, dict[str, str]]:
    if raw == "-":
        return {}
    result: dict[str, dict[str, str]] = {}
    for match in _SD_ELEMENT_RE.finditer(raw):
        body = match.group("body")
        params = {
            item.group("key"): item.group("value").replace('\\"', '"')
            for item in _SD_PARAM_RE.finditer(body)
        }
        result[match.group("sdid")] = params
    return result


def _none_if_nil(value: str) -> Any:
    if value == "-":
        return None
    return value
