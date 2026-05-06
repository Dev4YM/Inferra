from __future__ import annotations

import json
import re
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from core.enums import Severity
from events.models import freeze_value


@dataclass(frozen=True)
class ParserQuality:
    parser_name: str
    parse_confidence: float
    completeness: float
    flags: frozenset[str] = frozenset()
    severity_explicit: bool = False


@dataclass(frozen=True)
class ParserResult:
    timestamp: datetime | None
    severity: Severity | None
    message: str
    structured: dict[str, Any]
    tags: frozenset[str]
    quality: ParserQuality


_SPACE_RE = re.compile(r"\s+")


def severity_from_value(value: Any) -> Severity | None:
    if value is None:
        return None
    if isinstance(value, int):
        if 0 <= value <= 7:
            if value <= 2:
                return Severity.CRITICAL
            if value == 3:
                return Severity.ERROR
            if value == 4:
                return Severity.WARN
            if value >= 7:
                return Severity.DEBUG
            return Severity.INFO
        if value >= 50:
            return Severity.CRITICAL
        if value >= 40:
            return Severity.ERROR
        if value >= 30:
            return Severity.WARN
        if value >= 20:
            return Severity.INFO
        return Severity.DEBUG
    text = str(value).strip().lower()
    if text in {"debug", "trace"}:
        return Severity.DEBUG
    if text in {"info", "notice", "normal"}:
        return Severity.INFO
    if text in {"warn", "warning"}:
        return Severity.WARN
    if text in {"error", "err"}:
        return Severity.ERROR
    if text in {"fatal", "panic", "critical", "crit", "emerg", "alert"}:
        return Severity.CRITICAL
    try:
        return severity_from_value(int(text))
    except ValueError:
        return None


def infer_severity(message: str) -> Severity:
    lower = message.lower()
    if any(token in lower for token in ("fatal", "panic", "oom", "out of memory", "critical", "crashloopbackoff")):
        return Severity.CRITICAL
    if any(
        token in lower
        for token in ("error", "failed", "failure", "exception", "traceback", "segfault", "timeout", "timed out")
    ):
        return Severity.ERROR
    if "warn" in lower or "degraded" in lower:
        return Severity.WARN
    if "debug" in lower:
        return Severity.DEBUG
    return Severity.INFO


def normalize_message(message: str) -> str:
    text = _SPACE_RE.sub(" ", message.strip())
    return text or "<empty>"


def derive_tags(message: str, structured: dict[str, Any]) -> frozenset[str]:
    haystack = f"{message} {json.dumps(structured, default=str, sort_keys=True)}".lower()
    rules = {
        "oom": ("out of memory", "oom", "cannot allocate", "memory exhausted"),
        "resource_pressure": ("resource pressure", "high memory", "high cpu", "disk_percent"),
        "restart": ("restarting", "restart", "started container", "process started"),
        "connection_refused": ("connection refused", "econnrefused", "connect: connection refused"),
        "timeout": ("timed out", "timeout", "deadline exceeded", "context deadline"),
        "crash": ("segfault", "panic", "fatal", "core dumped", "unhandled exception"),
        "disk_full": ("no space left", "disk full", "enospc"),
        "permission_denied": ("permission denied", "access denied", "eacces", "forbidden"),
        "dns_failure": ("nxdomain", "name resolution", "getaddrinfo", "dns failure"),
        "certificate_error": ("certificate", "ssl", "tls", "x509"),
        "rate_limited": ("rate limit", "too many requests", "429"),
        "config_change": ("configuration", "config reload", "settings changed"),
        "deployment": ("deploy", "release", "rolling update", "image pulled"),
        "kubernetes": ("kubernetes", "pod", "crashloopbackoff", "imagepullbackoff", "evicted"),
        "windows_eventlog": ("windows event", "event log"),
    }
    return frozenset(tag for tag, patterns in rules.items() if any(pattern in haystack for pattern in patterns))


def completeness_score(message: str, structured: dict[str, Any], *, timestamp: datetime | None, severity: Severity | None) -> float:
    present = [bool(message and message != "<empty>"), timestamp is not None, severity is not None, bool(structured)]
    return round(sum(1.0 for item in present if item) / len(present), 4)


def make_result(
    *,
    parser_name: str,
    timestamp: datetime | None,
    severity: Severity | None,
    message: str,
    structured: dict[str, Any],
    parse_confidence: float,
    flags: set[str] | frozenset[str] | None = None,
    severity_explicit: bool = False,
    tags: set[str] | frozenset[str] | None = None,
) -> ParserResult:
    normalized_message = normalize_message(message)
    normalized_structured = dict(freeze_value(structured))
    normalized_tags = frozenset(tags or ()) | derive_tags(normalized_message, normalized_structured)
    return ParserResult(
        timestamp=timestamp,
        severity=severity,
        message=normalized_message,
        structured=normalized_structured,
        tags=normalized_tags,
        quality=ParserQuality(
            parser_name=parser_name,
            parse_confidence=round(parse_confidence, 4),
            completeness=completeness_score(
                normalized_message,
                normalized_structured,
                timestamp=timestamp,
                severity=severity,
            ),
            flags=frozenset(flags or ()),
            severity_explicit=severity_explicit,
        ),
    )
