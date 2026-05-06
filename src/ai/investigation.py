"""AI investigation contract and bundle builder.

The dossier requires structured, auditable AI investigation output:
- evidence-cited next steps
- explicit risk level
- explicit uncertainty
- read-only safety boundary

This module:
- defines a Pydantic schema (`InvestigationOutput`)
- builds a redacted evidence bundle from runtime state
- runs the bundle through an Ollama provider when AI is enabled
- falls back to a deterministic synthesis when AI is disabled or unavailable

The output never executes commands; `next_steps` are read-only suggestions for the user.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any

from pydantic import BaseModel, ConfigDict, Field, ValidationError

from ai.explainer import AiPromptTrace
from ai.ollama import AsyncOllamaProvider, OllamaError
from ai.prompts import TRACE_SCHEMA_VERSION, extract_json_object
from ai.redaction import sanitize_structure
from ai.service import AIService
from config.model import InferraConfig

INVESTIGATION_SCHEMA_VERSION = 1
INVESTIGATION_SYSTEM_PROMPT = """You are Inferra's read-only investigation assistant.
You receive a redacted runtime evidence bundle. You must:
- explain what is happening using only the supplied facts
- prioritize the next inspection step the operator should take
- cite supporting incident_id, service_id, or event_id values when possible
- never claim you executed or modified anything
- never propose remediation that would mutate the observed system
- include explicit uncertainty when evidence is thin

Return a single JSON object. No markdown fences. No prose outside JSON.
Every next_steps entry must have safety="read_only" and requires_user_action=true."""

INVESTIGATION_USER_TEMPLATE = """Mode: {mode}
Bundle:
{bundle_json}
Schema:
{{
  "headline": "string",
  "risk_level": "low|medium|high|critical",
  "confidence": 0.0,
  "what_happened": ["string"],
  "why_it_matters": ["string"],
  "likely_causes": ["string"],
  "evidence": [{{"type": "incident|service|event|workspace", "id": "string", "summary": "string"}}],
  "missing_evidence": ["string"],
  "next_steps": [
    {{
      "title": "string",
      "reason": "string",
      "safety": "read_only",
      "command": "string",
      "requires_user_action": true
    }}
  ],
  "uncertainty": ["string"],
  "citations": ["string"]
}}"""


class InvestigationStep(BaseModel):
    model_config = ConfigDict(extra="forbid")

    title: str
    reason: str = ""
    safety: str = "read_only"
    command: str = ""
    requires_user_action: bool = True


class InvestigationEvidence(BaseModel):
    model_config = ConfigDict(extra="forbid")

    type: str = Field(default="event")
    id: str = ""
    summary: str = ""


class InvestigationOutput(BaseModel):
    model_config = ConfigDict(extra="forbid")

    headline: str = ""
    risk_level: str = Field(default="low", pattern=r"^(low|medium|high|critical)$")
    confidence: float = Field(default=0.0, ge=0.0, le=1.0)
    what_happened: list[str] = Field(default_factory=list)
    why_it_matters: list[str] = Field(default_factory=list)
    likely_causes: list[str] = Field(default_factory=list)
    evidence: list[InvestigationEvidence] = Field(default_factory=list)
    missing_evidence: list[str] = Field(default_factory=list)
    next_steps: list[InvestigationStep] = Field(default_factory=list)
    uncertainty: list[str] = Field(default_factory=list)
    citations: list[str] = Field(default_factory=list)


@dataclass(slots=True)
class InvestigationResult:
    output: InvestigationOutput
    bundle: dict[str, Any]
    used_ai: bool
    provider_status: dict[str, Any]
    trace: AiPromptTrace | None = None
    fallback_reason: str = ""
    schema_version: int = INVESTIGATION_SCHEMA_VERSION


@dataclass(slots=True)
class EvidenceBundle:
    mode: str
    incident: dict[str, Any] | None = None
    hypotheses: list[dict[str, Any]] = field(default_factory=list)
    events: list[dict[str, Any]] = field(default_factory=list)
    services: list[dict[str, Any]] = field(default_factory=list)
    runtime: dict[str, Any] = field(default_factory=dict)
    workspace: dict[str, Any] = field(default_factory=dict)
    user_question: str = ""
    constraints: dict[str, bool] = field(
        default_factory=lambda: {"read_only": True, "do_not_execute": True, "must_cite_evidence": True}
    )

    def to_dict(self) -> dict[str, Any]:
        return {
            "mode": self.mode,
            "incident": self.incident,
            "hypotheses": self.hypotheses,
            "events": self.events,
            "services": self.services,
            "runtime": self.runtime,
            "workspace": self.workspace,
            "user_question": self.user_question,
            "constraints": dict(self.constraints),
        }


def _summarize_event(event: dict[str, Any]) -> dict[str, Any]:
    message = str(event.get("message") or "")
    if len(message) > 240:
        message = message[:237] + "..."
    return {
        "event_id": event.get("event_id"),
        "timestamp": event.get("timestamp"),
        "service_id": event.get("service_id"),
        "severity": event.get("severity"),
        "summary": message,
        "tags": list(event.get("tags") or []),
        "source_type": (event.get("source_ref") or {}).get("source_type"),
    }


def redact_bundle(bundle: EvidenceBundle, *, redact_raw_logs: bool = True) -> dict[str, Any]:
    """Apply Inferra redaction conventions to the bundle before sending to AI."""
    raw = bundle.to_dict()
    if redact_raw_logs:
        events = [_summarize_event(item) for item in raw.get("events") or []]
        raw["events"] = events
    sanitized, _report = sanitize_structure(raw)
    return sanitized


def _deterministic_fallback(bundle: EvidenceBundle, reason: str) -> InvestigationOutput:
    """Build a safe deterministic investigation when AI is not available."""
    headline_parts: list[str] = []
    risk = "low"
    incident = bundle.incident or {}
    services = bundle.services or []
    events = bundle.events or []
    workspace = bundle.workspace or {}
    if incident:
        sev = int(incident.get("severity") or 0)
        if sev >= 3:
            risk = "high"
        elif sev >= 2:
            risk = "medium"
        headline_parts.append(
            f"Incident {incident.get('incident_id')} on {incident.get('primary_service') or 'unknown'}"
        )
    elif services:
        affected = [item for item in services if item.get("status") in {"degraded", "critical"}]
        if affected:
            risk = "medium" if any(item.get("status") == "degraded" for item in affected) else "high"
            headline_parts.append(f"{len(affected)} services need attention")
        else:
            headline_parts.append(f"{len(services)} services observed")
    else:
        headline_parts.append("No active incident; collectors may be quiet")

    next_steps: list[InvestigationStep] = []
    if incident.get("incident_id"):
        next_steps.append(
            InvestigationStep(
                title=f"Inspect incident {incident['incident_id']}",
                reason="Review hypotheses and supporting evidence locally before any change.",
                command=f"inferra incidents show {incident['incident_id']}",
            )
        )
    if services:
        worst = next(
            (item for item in services if item.get("status") in {"critical", "degraded"}),
            services[0],
        )
        sid = str(worst.get("service_id") or "")
        if sid:
            next_steps.append(
                InvestigationStep(
                    title=f"Look at recent events for {sid}",
                    reason="Recent events often clarify whether the service is failing or noisy.",
                    command=f"inferra services events {sid} --limit 25",
                )
            )
    if not next_steps:
        next_steps.append(
            InvestigationStep(
                title="List recent events",
                reason="No active incident; sample events to understand current activity.",
                command="inferra events list --limit 25",
            )
        )

    return InvestigationOutput(
        headline=" · ".join(headline_parts) or "Inferra has nothing critical to report.",
        risk_level=risk,
        confidence=0.4,
        what_happened=[part for part in headline_parts if part],
        why_it_matters=(
            ["Severity warrants prompt inspection."] if risk in {"high", "critical"} else ["No urgent failure observed."]
        ),
        likely_causes=[],
        evidence=[
            *(
                [InvestigationEvidence(type="incident", id=str(incident.get("incident_id") or ""), summary="active incident")]
                if incident
                else []
            ),
            *[
                InvestigationEvidence(type="service", id=str(item.get("service_id") or ""), summary=str(item.get("status") or ""))
                for item in services[:5]
            ],
            *[
                InvestigationEvidence(type="event", id=str(item.get("event_id") or ""), summary=str(item.get("summary") or ""))
                for item in (_summarize_event(e) for e in events[:5])
            ],
            *(
                [InvestigationEvidence(type="workspace", id="projects", summary=f"{len(workspace.get('projects') or [])} projects detected")]
                if workspace.get("projects")
                else []
            ),
        ],
        missing_evidence=["AI provider unavailable; reasoning is deterministic."],
        next_steps=next_steps,
        uncertainty=[reason] if reason else ["Deterministic fallback used."],
        citations=[],
    )


async def run_investigation(
    config: InferraConfig,
    bundle: EvidenceBundle,
    *,
    ai_service: AIService | None = None,
) -> InvestigationResult:
    """Run an investigation; falls back deterministically when AI is disabled or fails."""
    service = ai_service or AIService(config)
    provider_status = await service.status()
    redacted = redact_bundle(bundle, redact_raw_logs=bool(config.ai.redact_raw_logs))
    if not config.ai.enabled or not provider_status.get("available"):
        reason = (
            "AI is disabled in config."
            if not config.ai.enabled
            else f"AI provider unavailable: {provider_status.get('reason') or provider_status.get('error') or 'unknown'}"
        )
        return InvestigationResult(
            output=_deterministic_fallback(bundle, reason),
            bundle=redacted,
            used_ai=False,
            provider_status=provider_status,
            fallback_reason=reason,
        )
    try:
        provider = AsyncOllamaProvider(config.ai)
    except OllamaError as exc:
        reason = f"Ollama setup failed: {exc}"
        return InvestigationResult(
            output=_deterministic_fallback(bundle, reason),
            bundle=redacted,
            used_ai=False,
            provider_status=provider_status,
            fallback_reason=reason,
        )
    bundle_json = json.dumps(redacted, sort_keys=True, default=str)
    user_prompt = INVESTIGATION_USER_TEMPLATE.format(mode=bundle.mode, bundle_json=bundle_json)
    messages = [
        {"role": "system", "content": INVESTIGATION_SYSTEM_PROMPT},
        {"role": "user", "content": user_prompt},
    ]
    trace = AiPromptTrace(
        trace_kind="investigate",
        sanitized_system_prompt=INVESTIGATION_SYSTEM_PROMPT,
        sanitized_user_prompt=user_prompt,
        allowed_fields=("mode", "incident", "hypotheses", "events", "services", "runtime", "workspace", "user_question", "constraints"),
        blocked_fields=("raw_event_messages", "env_values", "ip_addresses", "secrets"),
        raw_logs_sent=False,
        schema_version=TRACE_SCHEMA_VERSION,
    )
    try:
        raw = await provider.chat(messages)
    except OllamaError as exc:
        reason = f"AI call failed: {exc}"
        return InvestigationResult(
            output=_deterministic_fallback(bundle, reason),
            bundle=redacted,
            used_ai=False,
            provider_status=provider_status,
            trace=trace,
            fallback_reason=reason,
        )
    data = extract_json_object(raw)
    try:
        validated = InvestigationOutput.model_validate(data)
    except ValidationError as exc:
        reason = f"AI returned an invalid investigation payload: {exc.errors()[0]['msg'] if exc.errors() else exc}"
        return InvestigationResult(
            output=_deterministic_fallback(bundle, reason),
            bundle=redacted,
            used_ai=False,
            provider_status=provider_status,
            trace=trace,
            fallback_reason=reason,
        )
    for step in validated.next_steps:
        step.safety = "read_only"
        step.requires_user_action = True
    return InvestigationResult(
        output=validated,
        bundle=redacted,
        used_ai=True,
        provider_status=provider_status,
        trace=trace,
    )


def investigation_result_to_dict(result: InvestigationResult) -> dict[str, Any]:
    return {
        "schema_version": result.schema_version,
        "output": result.output.model_dump(),
        "used_ai": result.used_ai,
        "fallback_reason": result.fallback_reason,
        "provider": {
            "enabled": bool(result.provider_status.get("enabled")),
            "available": bool(result.provider_status.get("available")),
            "model": result.provider_status.get("model"),
            "base_url": result.provider_status.get("base_url"),
            "allow_remote": bool(result.provider_status.get("allow_remote")),
            "reason": result.provider_status.get("reason"),
        },
        "trace": (
            None
            if result.trace is None
            else {
                "trace_kind": result.trace.trace_kind,
                "sanitized_system_prompt": result.trace.sanitized_system_prompt,
                "sanitized_user_prompt": result.trace.sanitized_user_prompt,
                "allowed_fields": list(result.trace.allowed_fields),
                "blocked_fields": list(result.trace.blocked_fields),
                "raw_logs_sent": bool(result.trace.raw_logs_sent),
                "schema_version": int(result.trace.schema_version),
            }
        ),
        "bundle": result.bundle,
    }
