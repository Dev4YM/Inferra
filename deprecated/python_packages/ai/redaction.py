from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any

SECRET_KEYWORDS = ("password", "passwd", "secret", "token", "api_key", "apikey", "authorization", "cookie")
SECRET_REPLACEMENT = "[REDACTED]"

_BEARER_RE = re.compile(r"\bBearer\s+[A-Za-z0-9._~+/=-]+", re.IGNORECASE)
_ASSIGNMENT_RE = re.compile(
    r"(?i)\b(password|passwd|secret|token|api[_-]?key|authorization|cookie)\s*[:=]\s*([^\s,;]+)"
)
_SESSION_PAIR_RE = re.compile(r"(?i)\bsession\s*=\s*(\S+)")
_EXPORT_SET_RE = re.compile(
    r"(?i)\b(?:export|set)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(\S+)"
)
_IPV4_RE = re.compile(
    r"\b(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\b"
)
_IPV6_RE = re.compile(
    r"\b(?:[0-9a-fA-F]{1,4}:){2,}[0-9a-fA-F:]{2,}\b|::1\b|::ffff:[0-9a-fA-F:.]+\b"
)
_WIN_PATH_RE = re.compile(r"(?:[A-Za-z]:\\(?:[^\\\s]|\\[^\\\s])+)")
_UNIX_PATH_RE = re.compile(
    r"(?<![A-Za-z0-9])(?:/(?:home|Users|usr|var|etc|opt|tmp|root|mnt|proc|sys|dev|srv|lib|bin|sbin|run|media)(?:/[^|\s'\"]+)+)"
)
_JWTISH_RE = re.compile(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b")
_AWS_KEY_RE = re.compile(r"\bAKIA[0-9A-Z]{16}\b")
_HEX_AFTER_KEY_RE = re.compile(
    r"(?i)\b((?:signature|digest|secret|token|apikey)\s*[:=]\s*)([0-9a-f]{32,64})\b"
)


@dataclass(frozen=True)
class SanitizationRemoval:
    category: str
    detail: str
    count: int


@dataclass
class SanitizationReport:
    removals: list[SanitizationRemoval] = field(default_factory=list)

    def add(self, category: str, detail: str, count: int = 1) -> None:
        if count <= 0:
            return
        self.removals.append(SanitizationRemoval(category=category, detail=detail, count=count))

    def merge(self, other: SanitizationReport) -> SanitizationReport:
        return SanitizationReport(removals=[*self.removals, *other.removals])


def _apply_pattern(
    value: str,
    pattern: re.Pattern[str],
    category: str,
    replacement: str,
    report: SanitizationReport,
) -> str:
    matches = list(pattern.finditer(value))
    if not matches:
        return value
    report.add(category, pattern.pattern[:120], len(matches))
    return pattern.sub(replacement, value)


def sanitize_plaintext(value: str) -> tuple[str, SanitizationReport]:
    report = SanitizationReport()
    out = value
    out = _apply_pattern(out, _BEARER_RE, "bearer_token", "Bearer " + SECRET_REPLACEMENT, report)
    out, assign_n = _ASSIGNMENT_RE.subn(lambda match: f"{match.group(1)}={SECRET_REPLACEMENT}", out)
    if assign_n:
        report.add("secret_assignment", "keyword=value", assign_n)
    out, sess_n = _SESSION_PAIR_RE.subn("session=" + SECRET_REPLACEMENT, out)
    if sess_n:
        report.add("session_cookie", "session=value", sess_n)
    out, export_n = _EXPORT_SET_RE.subn(lambda match: f"{match.group(1)}={SECRET_REPLACEMENT}", out)
    if export_n:
        report.add("env_assignment", "export_or_set", export_n)
    out = _apply_pattern(out, _JWTISH_RE, "jwt_like", SECRET_REPLACEMENT, report)
    out = _apply_pattern(out, _AWS_KEY_RE, "aws_access_key", SECRET_REPLACEMENT, report)
    out, hex_n = _HEX_AFTER_KEY_RE.subn(lambda match: f"{match.group(1)}{SECRET_REPLACEMENT}", out)
    if hex_n:
        report.add("hex_secret", "keyed_hex", hex_n)
    out = _apply_pattern(out, _IPV4_RE, "ipv4", "[IP]", report)
    out = _apply_pattern(out, _IPV6_RE, "ipv6", "[IP]", report)
    out = _apply_pattern(out, _WIN_PATH_RE, "windows_path", "[PATH]", report)
    out = _apply_pattern(out, _UNIX_PATH_RE, "unix_path", "[PATH]", report)
    return out, report


def redact_text(value: str) -> str:
    text, _report = sanitize_plaintext(value)
    return text


def sanitize_structure(value: Any) -> tuple[Any, SanitizationReport]:
    report = SanitizationReport()
    return _sanitize_structure_inner(value, report)


def _sanitize_structure_inner(value: Any, report: SanitizationReport) -> tuple[Any, SanitizationReport]:
    if isinstance(value, dict):
        out: dict[str, Any] = {}
        for key, item in value.items():
            cleaned, nested = _sanitize_structure_inner(item, SanitizationReport())
            out[key] = cleaned
            report.merge(nested)
        return out, report
    if isinstance(value, list):
        out_list: list[Any] = []
        for item in value:
            cleaned, nested = _sanitize_structure_inner(item, SanitizationReport())
            out_list.append(cleaned)
            report.merge(nested)
        return out_list, report
    if isinstance(value, tuple):
        out_tuple, nested = _sanitize_structure_inner(list(value), report)
        return tuple(out_tuple), nested
    if isinstance(value, str):
        text, nested = sanitize_plaintext(value)
        report.merge(nested)
        return text, report
    return value, report


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
