from __future__ import annotations

import json
from typing import Any

from ai.redaction import redact_value
from events.models import NormalizedEvent
from events.serialization import event_to_dict, json_dumps


SYSTEM_PROMPT = """You are Inferra's guided AI explanation layer.
Use only the supplied incident, hypotheses, and event evidence.
Do not claim that you changed a system.
Do not invent evidence.
Always cite internal event_id or hypothesis_id values when they support a statement.
Return practical debugging guidance, not autonomous remediation."""


def incident_explanation_prompt(
    incident: dict[str, Any],
    hypotheses: list[dict[str, Any]],
    events: list[NormalizedEvent],
    max_events: int,
    redact_raw_logs: bool,
) -> str:
    event_payload = [event_to_dict(event) for event in sorted(events, key=lambda item: item.timestamp)[:max_events]]
    payload: dict[str, Any] = {
        "incident": incident,
        "hypotheses": hypotheses,
        "events": event_payload,
        "required_output": {
            "summary": "one short paragraph",
            "primary_hypothesis_text": "specific cause hypothesis with cited evidence ids",
            "evidence_narrative": "why the evidence supports it",
            "timeline_narrative": "ordered timeline using event ids",
            "alternative_explanations": ["short alternatives"],
            "suggested_actions": ["safe checks the user can run manually"],
            "uncertainty_notes": ["known uncertainty or missing evidence"],
        },
    }
    if redact_raw_logs:
        payload = redact_value(payload)
    return "Create a JSON incident explanation from this grounded Inferra payload:\n" + json_dumps(payload)


def incident_chat_prompt(
    question: str,
    incident: dict[str, Any],
    hypotheses: list[dict[str, Any]],
    events: list[NormalizedEvent],
    max_events: int,
    redact_raw_logs: bool,
) -> str:
    payload: dict[str, Any] = {
        "question": question,
        "incident": incident,
        "hypotheses": hypotheses,
        "events": [event_to_dict(event) for event in sorted(events, key=lambda item: item.timestamp)[:max_events]],
    }
    if redact_raw_logs:
        payload = redact_value(payload)
    return (
        "Answer the user's incident question using only this Inferra payload. "
        "Cite event_id or hypothesis_id values when relevant.\n"
        + json.dumps(payload, sort_keys=True)
    )
