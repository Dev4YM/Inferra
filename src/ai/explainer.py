from __future__ import annotations

from dataclasses import asdict, dataclass
from typing import Any

from pydantic import ValidationError

from ai.ollama import AsyncOllamaProvider
from ai.prompts import (
    CHAT_INCIDENT_ALLOWED_FIELDS,
    CHAT_INCIDENT_SYSTEM,
    EXPLAIN_INCIDENT_ALLOWED_FIELDS,
    EXPLAIN_INCIDENT_SYSTEM,
    ExplainIncidentOutput,
    ChatIncidentOutput,
    TRACE_SCHEMA_VERSION,
    chat_incident_user_prompt,
    explain_incident_user_prompt,
    extract_json_object,
    merge_blocked_lists,
    prepare_chat_incident_payload,
    prepare_explain_incident_payload,
)
from core.ids import new_id
from events.models import NormalizedEvent
from explanation.finalize import finalize_explanation_payload


@dataclass(frozen=True)
class AiPromptTrace:
    trace_kind: str
    sanitized_system_prompt: str
    sanitized_user_prompt: str
    allowed_fields: tuple[str, ...]
    blocked_fields: tuple[str, ...]
    raw_logs_sent: bool
    schema_version: int


class AIExplanationEngine:
    def __init__(self, provider: AsyncOllamaProvider) -> None:
        self.provider = provider

    def prepare_incident_chat(
        self,
        question: str,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
        *,
        history: list[dict[str, str]] | None = None,
    ) -> tuple[list[dict[str, str]], AiPromptTrace]:
        sanitized_payload, report = prepare_chat_incident_payload(
            question,
            history or [],
            incident,
            hypotheses,
            events,
            max_events=self.provider.config.max_context_events,
            redact_raw_logs=self.provider.config.redact_raw_logs,
        )
        user_prompt = chat_incident_user_prompt(sanitized_payload)
        blocked = tuple(merge_blocked_lists(report, sanitized_payload))
        trace = AiPromptTrace(
            trace_kind="chat",
            sanitized_system_prompt=CHAT_INCIDENT_SYSTEM,
            sanitized_user_prompt=user_prompt,
            allowed_fields=CHAT_INCIDENT_ALLOWED_FIELDS,
            blocked_fields=blocked,
            raw_logs_sent=False,
            schema_version=TRACE_SCHEMA_VERSION,
        )
        messages = [
            {"role": "system", "content": CHAT_INCIDENT_SYSTEM},
            {"role": "user", "content": user_prompt},
        ]
        return messages, trace

    async def complete_chat_messages(self, messages: list[dict[str, str]]) -> str:
        return await self._complete_messages(messages)

    async def _complete_messages(self, messages: list[dict[str, str]]) -> str:
        model = self.provider.config.model
        if self.provider.config.stream:
            chunks: list[str] = []
            async for chunk in self.provider.chat_stream(messages, model):
                chunks.append(chunk.content)
            return "".join(chunks)
        return await self.provider.chat(messages, model, retries=self.provider.config.max_retries)

    def prepare_explain_incident(
        self,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> tuple[list[dict[str, str]], AiPromptTrace]:
        sanitized_payload, report = prepare_explain_incident_payload(
            incident,
            hypotheses,
            events,
            max_events=self.provider.config.max_context_events,
            redact_raw_logs=self.provider.config.redact_raw_logs,
        )
        user_prompt = explain_incident_user_prompt(sanitized_payload)
        blocked = tuple(merge_blocked_lists(report, sanitized_payload))
        trace = AiPromptTrace(
            trace_kind="explain",
            sanitized_system_prompt=EXPLAIN_INCIDENT_SYSTEM,
            sanitized_user_prompt=user_prompt,
            allowed_fields=EXPLAIN_INCIDENT_ALLOWED_FIELDS,
            blocked_fields=blocked,
            raw_logs_sent=False,
            schema_version=TRACE_SCHEMA_VERSION,
        )
        messages = [
            {"role": "system", "content": EXPLAIN_INCIDENT_SYSTEM},
            {"role": "user", "content": user_prompt},
        ]
        return messages, trace

    async def generate(
        self,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> tuple[dict[str, Any], AiPromptTrace]:
        messages, trace = self.prepare_explain_incident(incident, hypotheses, events)
        validated = await self._explain_with_retries(messages)
        if validated is None:
            from explanation.template import TemplateExplanationEngine

            tmpl = TemplateExplanationEngine()
            template_result = tmpl.generate(incident, hypotheses, events)
            payload = asdict(template_result)
            merged_violations = sorted(
                {*list(payload.get("guardrail_violations") or []), "schema_validation_fallback"}
            )
            payload["guardrail_violations"] = merged_violations
            payload["generation_model"] = "template_fallback"
            finalized = finalize_explanation_payload(incident, hypotheses, events, payload, template=True)
            return finalized, trace

        payload = {
            "explanation_id": new_id("exp"),
            "incident_id": incident["incident_id"],
            "summary": validated.summary,
            "primary_hypothesis_text": validated.primary_hypothesis_text,
            "evidence_narrative": validated.evidence_narrative,
            "timeline_narrative": validated.timeline_narrative,
            "alternative_explanations": list(validated.alternative_explanations),
            "suggested_actions": list(validated.suggested_actions),
            "uncertainty_notes": list(validated.uncertainty_notes),
            "generation_model": self.provider.config.model,
            "guardrail_violations": [],
        }
        finalized = finalize_explanation_payload(incident, hypotheses, events, payload, template=False)
        return finalized, trace

    async def _explain_with_retries(self, messages: list[dict[str, str]]) -> ExplainIncidentOutput | None:
        attempts = self.provider.config.max_retries + 1
        for _ in range(attempts):
            content = await self._complete_messages(messages)
            data = extract_json_object(content)
            try:
                return ExplainIncidentOutput.model_validate(data)
            except ValidationError:
                continue
        return None

    async def chat(
        self,
        question: str,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
        *,
        history: list[dict[str, str]] | None = None,
    ) -> tuple[dict[str, Any], AiPromptTrace]:
        messages, trace = self.prepare_incident_chat(
            question,
            incident,
            hypotheses,
            events,
            history=history,
        )
        validated = await self._chat_with_retries(messages)
        if validated is None:
            fallback = _chat_template_fallback(hypotheses)
            return (
                {
                    "answer": fallback,
                    "generation_model": "template_fallback",
                    "guardrail_violations": ["schema_validation_fallback"],
                },
                trace,
            )
        return (
            {
                "answer": validated.answer,
                "generation_model": self.provider.config.model,
                "guardrail_violations": [],
            },
            trace,
        )

    async def _chat_with_retries(self, messages: list[dict[str, str]]) -> ChatIncidentOutput | None:
        attempts = self.provider.config.max_retries + 1
        for _ in range(attempts):
            content = await self._complete_messages(messages)
            data = extract_json_object(content)
            try:
                return ChatIncidentOutput.model_validate(data)
            except ValidationError:
                continue
        return None


def _chat_template_fallback(hypotheses: list[dict[str, Any]]) -> str:
    top = hypotheses[0] if hypotheses else None
    if top:
        return (
            "Deterministic fallback (AI JSON schema validation failed). "
            f"Leading hypothesis: {top.get('description')}. "
            "Review supporting_event identifiers on the Timeline tab."
        )
    return (
        "Deterministic fallback (AI JSON schema validation failed). "
        "No hypotheses are available yet; review incident-linked events."
    )


def chat_answer_from_model_output(raw: str, hypotheses: list[dict[str, Any]]) -> str:
    try:
        return ChatIncidentOutput.model_validate(extract_json_object(raw)).answer
    except ValidationError:
        return _chat_template_fallback(hypotheses)
