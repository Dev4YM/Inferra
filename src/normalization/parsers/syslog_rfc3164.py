from __future__ import annotations

import re
from datetime import UTC, datetime

from normalization.parsers.base import ParserResult, infer_severity, make_result


_RFC3164_RE = re.compile(
    r"^(?:<(?P<priority>\d{1,3})>)?"
    r"(?P<month>[A-Z][a-z]{2})\s+(?P<day>\d{1,2})\s+"
    r"(?P<time>\d{2}:\d{2}:\d{2})\s+"
    r"(?P<host>\S+)\s+"
    r"(?P<program>[^:\[]+?)(?:\[(?P<pid>\d+)\])?:\s*(?P<message>.*)$"
)


def parse(text: str, collected_at: datetime) -> ParserResult:
    match = _RFC3164_RE.match(text.strip())
    if match is None:
        return make_result(
            parser_name="syslog_rfc3164",
            timestamp=None,
            severity=None,
            message=text,
            structured={"raw": text},
            parse_confidence=0.2,
            flags={"syslog_rfc3164_mismatch"},
        )

    timestamp = _parse_timestamp(match, collected_at)
    message = match.group("message")
    structured = {
        "priority": _as_int(match.group("priority")),
        "host": match.group("host"),
        "program": match.group("program"),
        "pid": _as_int(match.group("pid")),
        "syslog_format": "rfc3164",
    }
    return make_result(
        parser_name="syslog_rfc3164",
        timestamp=timestamp,
        severity=infer_severity(message),
        message=message,
        structured=structured,
        parse_confidence=0.92,
        tags={"syslog"},
    )


def _parse_timestamp(match: re.Match[str], collected_at: datetime) -> datetime:
    candidate = datetime.strptime(
        f"{collected_at.year} {match.group('month')} {match.group('day')} {match.group('time')}",
        "%Y %b %d %H:%M:%S",
    ).replace(tzinfo=UTC)
    if candidate > collected_at.astimezone(UTC):
        return candidate.replace(year=candidate.year - 1)
    return candidate


def _as_int(value: str | None) -> int | None:
    if value is None:
        return None
    return int(value)
