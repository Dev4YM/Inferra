from __future__ import annotations

import json
from typing import Any

from pydantic import BaseModel, ConfigDict, Field

from ai.redaction import SECRET_REPLACEMENT, SanitizationReport, redact_value, sanitize_structure
from core.enums import EventType, Severity
from core.time import to_iso
from events.models import EventFilter, NormalizedEvent
from events.serialization import json_dumps

TRACE_SCHEMA_VERSION = 1

EXPLAIN_INCIDENT_ALLOWED_FIELDS: tuple[str, ...] = (
    "incident_id",
    "time_range_start",
    "time_range_end",
    "primary_service",
    "affected_services",
    "severity",
    "state",
    "ranked_hypotheses",
    "hypothesis_scores",
    "suggested_checks",
    "event_summaries",
)

CHAT_INCIDENT_ALLOWED_FIELDS: tuple[str, ...] = EXPLAIN_INCIDENT_ALLOWED_FIELDS + (
    "conversation_turns",
    "user_question",
)

NATURAL_LANGUAGE_SEARCH_ALLOWED_FIELDS: tuple[str, ...] = (
    "user_query",
    "service_catalog_hint",
)

EXPLAIN_INCIDENT_SYSTEM = """You are Inferra's guided AI explanation layer.
Use only the supplied incident summary, ranked hypotheses, suggested checks, and redacted event summaries.
Do not claim that you changed a system.
Do not invent evidence.
Always cite internal event_id or hypothesis_id values when they support a statement.
Return practical debugging guidance, not autonomous remediation.
Respond with a single JSON object only. No markdown fences or prose outside JSON."""

CHAT_INCIDENT_SYSTEM = """You are Inferra's guided AI assistant for incident investigation.
Answer using only the supplied incident context and conversation history.
Do not claim remediation actions were executed.
Do not invent evidence; cite event_id or hypothesis_id when relevant.
Respond with a single JSON object only with key \"answer\" (string). No markdown fences."""

NATURAL_LANGUAGE_SEARCH_SYSTEM = """You extract structured event filter criteria from the user's natural-language query.
Infer services, hosts, severity keywords, tags, and message substrings when explicit.
Respond with a single JSON object only matching the schema. No markdown fences.
If unsure, lower confidence and populate suggestions with clarifying questions."""

SECRET_VALUE_SENTINEL = SECRET_REPLACEMENT


class ExplainIncidentOutput(BaseModel):
    model_config = ConfigDict(extra="forbid")

    summary: str
    primary_hypothesis_text: str
    evidence_narrative: str = ""
    timeline_narrative: str = ""
    alternative_explanations: list[str] = Field(default_factory=list)
    suggested_actions: list[str] = Field(default_factory=list)
    uncertainty_notes: list[str] = Field(default_factory=list)


class ChatIncidentOutput(BaseModel):
    model_config = ConfigDict(extra="forbid")

    answer: str


class NaturalLanguageFilterShape(BaseModel):
    model_config = ConfigDict(extra="forbid")

    service_ids: list[str] | None = None
    host_ids: list[str] | None = None
    severities: list[str] | None = None
    event_types: list[str] | None = None
    tags: list[str] | None = None
    message_contains: str | None = None


class NaturalLanguageSearchOutput(BaseModel):
    model_config = ConfigDict(extra="forbid")

    filter: NaturalLanguageFilterShape
    confidence: float = Field(ge=0.0, le=1.0)
    suggestions: list[str] = Field(default_factory=list)


EXPLAIN_INCIDENT_JSON_SCHEMA: dict[str, Any] = ExplainIncidentOutput.model_json_schema()
CHAT_INCIDENT_JSON_SCHEMA: dict[str, Any] = ChatIncidentOutput.model_json_schema()
NATURAL_LANGUAGE_SEARCH_JSON_SCHEMA: dict[str, Any] = NaturalLanguageSearchOutput.model_json_schema()


def incident_summary_payload(incident: dict[str, Any]) -> dict[str, Any]:
    return {
        "incident_id": incident.get("incident_id"),
        "time_range_start": incident.get("time_range_start"),
        "time_range_end": incident.get("time_range_end"),
        "primary_service": incident.get("primary_service"),
        "affected_services": list(incident.get("affected_services") or []),
        "severity": incident.get("severity"),
        "state": incident.get("state"),
    }


def ranked_hypothesis_payloads(hypotheses: list[dict[str, Any]]) -> list[dict[str, Any]]:
    sorted_h = sorted(hypotheses, key=lambda item: (int(item.get("rank") or 999), -float(item.get("total_score") or 0.0)))
    rows: list[dict[str, Any]] = []
    for hyp in sorted_h:
        breakdown = hyp.get("score_breakdown") or {}
        rows.append(
            {
                "hypothesis_id": hyp.get("hypothesis_id"),
                "rank": hyp.get("rank"),
                "cause_type": hyp.get("cause_type"),
                "description": hyp.get("description"),
                "total_score": hyp.get("total_score"),
                "score_breakdown": breakdown,
                "supporting_events": list(hyp.get("supporting_events") or [])[:64],
                "contradicting_events": list(hyp.get("contradicting_events") or [])[:64],
                "suggested_checks": list(hyp.get("suggested_checks") or []),
                "confidence_label": hyp.get("confidence_label"),
            }
        )
    return rows


def suggested_checks_union(hypotheses: list[dict[str, Any]], *, limit: int = 48) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for hyp in sorted(hypotheses, key=lambda item: (int(item.get("rank") or 999))):
        for check in list(hyp.get("suggested_checks") or []):
            text = str(check).strip()
            if not text or text in seen:
                continue
            seen.add(text)
            ordered.append(text)
            if len(ordered) >= limit:
                return ordered
    return ordered


def event_summaries_for_prompt(events: list[NormalizedEvent], *, limit: int) -> list[dict[str, Any]]:
    summaries: list[dict[str, Any]] = []
    for event in sorted(events, key=lambda item: item.timestamp)[:limit]:
        summaries.append(
            {
                "event_id": event.event_id,
                "timestamp": to_iso(event.timestamp),
                "service_id": event.service_id,
                "host_id": event.host_id,
                "severity": event.severity.name,
                "message": event.message[:640],
                "tags": sorted(event.tags),
            }
        )
    return summaries


def collect_blocked_secret_paths(value: Any, prefix: str = "") -> list[str]:
    blocked: list[str] = []
    if isinstance(value, dict):
        for key, item in value.items():
            path = f"{prefix}.{key}" if prefix else str(key)
            if item == SECRET_VALUE_SENTINEL:
                blocked.append(f"secret_key:{path}")
            else:
                blocked.extend(collect_blocked_secret_paths(item, path))
    elif isinstance(value, list):
        for index, item in enumerate(value):
            blocked.extend(collect_blocked_secret_paths(item, f"{prefix}[{index}]"))
    return blocked


def sanitization_blocked_labels(report: SanitizationReport) -> list[str]:
    labels: list[str] = []
    for removal in report.removals:
        labels.append(f"{removal.category}:{removal.detail}")
    return sorted(set(labels))


def prepare_explain_incident_payload(
    incident: dict[str, Any],
    hypotheses: list[dict[str, Any]],
    events: list[NormalizedEvent],
    *,
    max_events: int,
    redact_raw_logs: bool,
) -> tuple[dict[str, Any], SanitizationReport]:
    payload: dict[str, Any] = {
        "incident": incident_summary_payload(incident),
        "ranked_hypotheses": ranked_hypothesis_payloads(hypotheses),
        "suggested_checks": suggested_checks_union(hypotheses),
        "event_summaries": event_summaries_for_prompt(events, limit=max_events),
        "required_json_schema": EXPLAIN_INCIDENT_JSON_SCHEMA,
    }
    if redact_raw_logs:
        payload = redact_value(payload)
    sanitized, report = sanitize_structure(payload)
    return sanitized, report


def explain_incident_user_prompt(sanitized_payload: dict[str, Any]) -> str:
    instructions = (
        "Produce JSON keys: summary, primary_hypothesis_text, evidence_narrative, timeline_narrative, "
        "alternative_explanations, suggested_actions, uncertainty_notes.\n"
        "Ground every claim in hypothesis_id or event_id references where applicable.\n"
    )
    return instructions + json_dumps(sanitized_payload)


def prepare_chat_incident_payload(
    question: str,
    history: list[dict[str, str]],
    incident: dict[str, Any],
    hypotheses: list[dict[str, Any]],
    events: list[NormalizedEvent],
    *,
    max_events: int,
    redact_raw_logs: bool,
) -> tuple[dict[str, Any], SanitizationReport]:
    turns = [{"role": item["role"], "content": item["content"]} for item in history if item.get("content")]
    payload: dict[str, Any] = {
        "user_question": question,
        "conversation_turns": turns[-40:],
        "incident": incident_summary_payload(incident),
        "ranked_hypotheses": ranked_hypothesis_payloads(hypotheses),
        "suggested_checks": suggested_checks_union(hypotheses),
        "event_summaries": event_summaries_for_prompt(events, limit=max_events),
        "required_json_schema": CHAT_INCIDENT_JSON_SCHEMA,
    }
    if redact_raw_logs:
        payload = redact_value(payload)
    sanitized, report = sanitize_structure(payload)
    return sanitized, report


def chat_incident_user_prompt(sanitized_payload: dict[str, Any]) -> str:
    instructions = 'Respond as JSON: {"answer": "<plain text>"}.\n'
    return instructions + json_dumps(sanitized_payload)


def natural_language_search_user_prompt(
    query: str,
    *,
    service_catalog: list[str],
) -> str:
    payload = {
        "user_query": query.strip(),
        "service_catalog_hint": sorted({str(item).strip() for item in service_catalog if str(item).strip()}),
        "required_json_schema": NATURAL_LANGUAGE_SEARCH_JSON_SCHEMA,
    }
    sanitized, _report = sanitize_structure(payload)
    instructions = (
        "Return JSON with keys filter (object), confidence (0..1 float), suggestions (string array). "
        "filter may include service_ids, host_ids, severities (DEBUG|INFO|WARN|ERROR|CRITICAL), "
        "event_types (LOG|METRIC|STATE_CHANGE|HEALTH_CHECK), tags, message_contains.\n"
    )
    return instructions + json_dumps(sanitized)


def merge_blocked_lists(report: SanitizationReport, payload: dict[str, Any]) -> list[str]:
    labels = sanitization_blocked_labels(report)
    labels.extend(collect_blocked_secret_paths(payload))
    return sorted(set(labels))


def extract_json_object(raw: str) -> dict[str, Any]:
    text = raw.strip()
    try:
        decoded = json.loads(text)
    except json.JSONDecodeError:
        start = text.find("{")
        end = text.rfind("}")
        if start == -1 or end == -1 or end <= start:
            return {}
        try:
            decoded = json.loads(text[start : end + 1])
        except json.JSONDecodeError:
            return {}
    return decoded if isinstance(decoded, dict) else {}


def event_filter_from_nl_output(output: NaturalLanguageSearchOutput) -> EventFilter:
    sev: set[Severity] | None = None
    if output.filter.severities:
        resolved: set[Severity] = set()
        for name in output.filter.severities:
            key = str(name).strip().upper()
            if key in Severity.__members__:
                resolved.add(Severity[key])
        sev = resolved or None
    etypes: set[EventType] | None = None
    if output.filter.event_types:
        resolved_types: set[EventType] = set()
        for name in output.filter.event_types:
            key = str(name).strip().upper()
            if key in EventType.__members__:
                resolved_types.add(EventType[key])
        etypes = resolved_types or None
    svc = {str(item).strip() for item in (output.filter.service_ids or []) if str(item).strip()}
    hosts = {str(item).strip() for item in (output.filter.host_ids or []) if str(item).strip()}
    tags = {str(item).strip() for item in (output.filter.tags or []) if str(item).strip()}
    msg = output.filter.message_contains
    message_contains = str(msg).strip() if msg else None
    return EventFilter(
        service_ids=svc or None,
        host_ids=hosts or None,
        severities=sev,
        event_types=etypes,
        tags=tags or None,
        message_contains=message_contains,
    )


def serialized_event_filter(filters: EventFilter) -> dict[str, Any]:
    return {
        "service_ids": sorted(filters.service_ids) if filters.service_ids else None,
        "host_ids": sorted(filters.host_ids) if filters.host_ids else None,
        "severities": sorted(item.name for item in filters.severities) if filters.severities else None,
        "event_types": sorted(item.name for item in filters.event_types) if filters.event_types else None,
        "tags": sorted(filters.tags) if filters.tags else None,
        "message_contains": filters.message_contains,
    }


SYSTEM_PROMPT = CHAT_INCIDENT_SYSTEM

