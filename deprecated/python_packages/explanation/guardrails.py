from __future__ import annotations

import re
from datetime import datetime
from typing import Any

from core.time import parse_datetime, to_iso
from events.models import NormalizedEvent

_SERVICE_TOKEN = re.compile(r"\b[a-z][a-z0-9_-]{2,}\b", re.IGNORECASE)
_TIME_HMS = re.compile(r"\b(?:\d{2}:\d{2}:\d{2})\b")
_ISO_TS = re.compile(r"\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?\b")
_CAUSAL_PHRASES = (
    "caused by",
    "led to",
    "resulted in",
    "triggered",
    "root cause",
    "because of",
    "due to",
)
_OVERCONFIDENCE = re.compile(
    r"\b(?:definitely|always|guaranteed)\b",
    re.IGNORECASE,
)

_COMMON_TECHNICAL = frozenset(
    {
        "the",
        "and",
        "for",
        "with",
        "from",
        "this",
        "that",
        "into",
        "over",
        "under",
        "when",
        "were",
        "was",
        "are",
        "has",
        "have",
        "had",
        "not",
        "but",
        "its",
        "our",
        "your",
        "any",
        "all",
        "per",
        "via",
        "using",
        "used",
        "based",
        "during",
        "after",
        "before",
        "between",
        "within",
        "without",
        "about",
        "than",
        "then",
        "them",
        "they",
        "their",
        "there",
        "these",
        "those",
        "which",
        "while",
        "where",
        "what",
        "who",
        "how",
        "why",
        "may",
        "might",
        "could",
        "would",
        "should",
        "will",
        "can",
        "must",
        "likely",
        "possible",
        "events",
        "event",
        "error",
        "errors",
        "warn",
        "warning",
        "critical",
        "info",
        "level",
        "message",
        "messages",
        "timeout",
        "timeouts",
        "connection",
        "connections",
        "refused",
        "failed",
        "failure",
        "failures",
        "service",
        "services",
        "host",
        "hosts",
        "http",
        "https",
        "tcp",
        "dns",
        "tls",
        "ssl",
        "cpu",
        "memory",
        "disk",
        "network",
        "latency",
        "throughput",
        "retry",
        "retries",
        "queue",
        "pool",
        "thread",
        "threads",
        "process",
        "processes",
        "container",
        "containers",
        "node",
        "nodes",
        "cluster",
        "namespace",
        "incident",
        "hypothesis",
        "hypotheses",
        "score",
        "scores",
        "rank",
        "ranks",
        "evidence",
        "timeline",
        "summary",
        "operator",
        "check",
        "checks",
        "verify",
        "review",
        "logs",
        "log",
        "trace",
        "metric",
        "metrics",
        "utc",
        "gmt",
        "iso",
    }
)


def _incident_time_bounds(incident: dict[str, Any]) -> tuple[datetime | None, datetime | None]:
    start = parse_datetime(str(incident.get("time_range_start") or ""))
    end = parse_datetime(str(incident.get("time_range_end") or ""))
    return start, end


def _event_timestamp_strings(events: list[Any]) -> set[str]:
    out: set[str] = set()
    for event in events:
        if not isinstance(event, NormalizedEvent):
            continue
        raw = to_iso(event.timestamp)
        out.add(raw)
        if "T" in raw:
            tail = raw.split("T", 1)[1]
            if "." in tail:
                out.add(tail.split(".", 1)[0])
            else:
                out.add(tail.replace("Z", "").split("+", 1)[0].split("-", 1)[0])
    return out


def verify_service_names(text: str, affected_services: list[str]) -> list[str]:
    allowed = {item.lower() for item in affected_services if item}
    violations: list[str] = []
    for token in _SERVICE_TOKEN.findall(text.lower()):
        if token in _COMMON_TECHNICAL:
            continue
        if token in allowed:
            continue
        if "-" in token or "_" in token:
            violations.append(f"unknown_service_token:{token}")
            continue
        if len(token) >= 12 and token.isalnum():
            violations.append(f"unknown_service_token:{token}")
    return violations


def verify_timestamps(
    text: str,
    incident: dict[str, Any],
    events: list[Any],
) -> list[str]:
    violations: list[str] = []
    start, end = _incident_time_bounds(incident)
    event_ts = _event_timestamp_strings(events)
    for match in _ISO_TS.findall(text):
        parsed = parse_datetime(match)
        if parsed is None:
            violations.append(f"timestamp_unparsed:{match}")
            continue
        if start is not None and parsed < start:
            violations.append(f"timestamp_before_range:{match}")
        if end is not None and parsed > end:
            violations.append(f"timestamp_after_range:{match}")
    if start is None or end is None:
        return violations
    for hms in _TIME_HMS.findall(text):
        if any(hms in full for full in event_ts):
            continue
        violations.append(f"time_not_in_events:{hms}")
    return violations


def verify_causal_claims(text: str, hypotheses: list[dict[str, Any]]) -> list[str]:
    if not hypotheses:
        return []
    lowered = text.lower()
    if not any(phrase in lowered for phrase in _CAUSAL_PHRASES):
        return []
    for hyp in hypotheses:
        desc = str(hyp.get("description") or "").strip().lower()
        if len(desc) >= 6 and desc in lowered:
            return []
        cause = str(hyp.get("cause_type") or "").strip().lower()
        if cause and cause in lowered:
            return []
    return ["causal_claim_not_in_hypotheses"]


def check_overconfidence(text: str) -> list[str]:
    violations: list[str] = []
    for match in _OVERCONFIDENCE.finditer(text):
        violations.append(f"overconfidence:{match.group(0).lower()}")
    return violations


def run_explanation_guardrails(
    incident: dict[str, Any],
    hypotheses: list[dict[str, Any]],
    events: list[Any],
    *,
    summary: str,
    primary_hypothesis_text: str,
    evidence_narrative: str,
    timeline_narrative: str,
    alternative_explanations: list[str],
    suggested_actions: list[str],
    uncertainty_notes: list[str],
) -> list[str]:
    bundle = "\n".join(
        [
            summary,
            primary_hypothesis_text,
            evidence_narrative,
            timeline_narrative,
            "\n".join(alternative_explanations),
            "\n".join(suggested_actions),
            "\n".join(uncertainty_notes),
        ]
    )
    services = list(incident.get("affected_services") or [])
    violations: list[str] = []
    violations.extend(verify_service_names(bundle, services))
    violations.extend(verify_timestamps(bundle, incident, events))
    violations.extend(verify_causal_claims(bundle, hypotheses))
    violations.extend(check_overconfidence(bundle))
    return violations
