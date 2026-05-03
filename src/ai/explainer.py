from __future__ import annotations

import json
from typing import Any

from ai.ollama import OllamaError, OllamaProvider
from ai.prompts import SYSTEM_PROMPT, incident_chat_prompt, incident_explanation_prompt
from core.ids import new_id
from events.models import NormalizedEvent


class AIExplanationEngine:
    def __init__(self, provider: OllamaProvider) -> None:
        self.provider = provider

    def generate(
        self,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> dict[str, Any]:
        prompt = incident_explanation_prompt(
            incident,
            hypotheses,
            events,
            max_events=self.provider.config.max_context_events,
            redact_raw_logs=self.provider.config.redact_raw_logs,
        )
        content = self.provider.chat(
            [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": prompt},
            ]
        )
        parsed = _extract_json_object(content)
        return {
            "explanation_id": new_id("exp"),
            "incident_id": incident["incident_id"],
            "summary": str(parsed.get("summary") or content[:500]),
            "primary_hypothesis_text": str(parsed.get("primary_hypothesis_text") or parsed.get("primary") or ""),
            "evidence_narrative": str(parsed.get("evidence_narrative") or ""),
            "timeline_narrative": str(parsed.get("timeline_narrative") or ""),
            "alternative_explanations": _string_list(parsed.get("alternative_explanations")),
            "suggested_actions": _string_list(parsed.get("suggested_actions")),
            "uncertainty_notes": _string_list(parsed.get("uncertainty_notes")),
            "generation_model": self.provider.config.model,
            "guardrail_violations": [],
        }

    def chat(
        self,
        question: str,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> dict[str, Any]:
        prompt = incident_chat_prompt(
            question,
            incident,
            hypotheses,
            events,
            max_events=self.provider.config.max_context_events,
            redact_raw_logs=self.provider.config.redact_raw_logs,
        )
        answer = self.provider.chat(
            [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": prompt},
            ]
        )
        return {"answer": answer, "generation_model": self.provider.config.model}


def explanation_error(incident_id: str, exc: OllamaError) -> dict[str, Any]:
    return {
        "explanation_id": new_id("exp"),
        "incident_id": incident_id,
        "summary": "AI explanation is unavailable; Inferra kept the deterministic explanation path available.",
        "primary_hypothesis_text": "AI provider unavailable.",
        "evidence_narrative": "",
        "timeline_narrative": "",
        "alternative_explanations": [],
        "suggested_actions": ["Run `inferra ai status` and verify the configured Ollama model is installed."],
        "uncertainty_notes": [str(exc)],
        "generation_model": "ai_unavailable",
        "guardrail_violations": ["provider_unavailable"],
    }


def _extract_json_object(value: str) -> dict[str, Any]:
    try:
        decoded = json.loads(value)
    except json.JSONDecodeError:
        start = value.find("{")
        end = value.rfind("}")
        if start == -1 or end == -1 or end <= start:
            return {"summary": value}
        try:
            decoded = json.loads(value[start : end + 1])
        except json.JSONDecodeError:
            return {"summary": value}
    return decoded if isinstance(decoded, dict) else {"summary": value}


def _string_list(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    return [str(item) for item in value]
