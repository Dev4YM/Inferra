from __future__ import annotations

from collections.abc import AsyncIterator
from dataclasses import asdict
from datetime import timedelta
from typing import Any

from pydantic import ValidationError

from ai.explainer import AIExplanationEngine, AiPromptTrace
from ai.ollama import AsyncOllamaProvider, OllamaError, OllamaPullProgress, OllamaStreamChunk
from ai.prompts import (
    NATURAL_LANGUAGE_SEARCH_SYSTEM,
    NaturalLanguageSearchOutput,
    event_filter_from_nl_output,
    extract_json_object,
    natural_language_search_user_prompt,
    serialized_event_filter,
)
from ai.registry import gemma4_model, list_gemma_models
from config.model import InferraConfig
from core.time import utc_now
from events.models import NormalizedEvent
from events.serialization import event_to_dict
from explanation.finalize import finalize_explanation_payload
from explanation.template import TemplateExplanationEngine
from storage.event_store import EventStore


class AIService:
    def __init__(self, config: InferraConfig) -> None:
        self.config = config

    async def status(self) -> dict[str, Any]:
        ai = self.config.ai
        registry_model = gemma4_model(ai.model)
        payload: dict[str, Any] = {
            "enabled": ai.enabled,
            "provider": ai.provider,
            "base_url": ai.base_url,
            "model": ai.model,
            "token_env": ai.token_env,
            "allow_remote": ai.allow_remote,
            "registry_model": _model_to_dict(registry_model) if registry_model else None,
        }
        if not ai.enabled:
            payload["available"] = False
            payload["reason"] = "AI is disabled in config."
            return payload
        try:
            provider = self._provider()
            status = await provider.status()
        except OllamaError as exc:
            payload["available"] = False
            payload["reason"] = getattr(exc, "reason_code", "ollama_error")
            payload["error"] = str(exc)
            return payload
        payload.update(
            {
                "available": status.available,
                "resolved_model": status.resolved_model,
                "installed": status.installed,
                "version": status.version,
                "latency_ms": status.latency_ms,
                "models": list(status.models),
                "show": status.show,
                "test_response": status.test_response,
                "reason": status.reason,
                "error": status.error,
            }
        )
        return payload

    def registry(self) -> list[dict[str, Any]]:
        return [_model_to_dict(model) for model in list_gemma_models()]

    async def installed_models(self) -> list[str]:
        return await self._provider().list_models()

    async def pull_model(self, model: str) -> dict[str, Any]:
        return await self._provider().pull_model(model)

    def pull_model_stream(self, model: str) -> AsyncIterator[OllamaPullProgress]:
        return self._provider().pull_model_stream(model)

    async def test(self) -> str:
        return await self._provider().test()

    async def explain(
        self,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> tuple[dict[str, Any], AiPromptTrace | None] | None:
        if not self.config.ai.enabled:
            return None
        try:
            payload, trace = await AIExplanationEngine(self._provider()).generate(incident, hypotheses, events)
            return payload, trace
        except OllamaError:
            tmpl = TemplateExplanationEngine()
            template_result = tmpl.generate(incident, hypotheses, events)
            payload = asdict(template_result)
            merged = sorted({*(payload.get("guardrail_violations") or []), "provider_unavailable"})
            payload["guardrail_violations"] = merged
            payload["generation_model"] = "template_fallback"
            finalized = finalize_explanation_payload(incident, hypotheses, events, payload, template=True)
            return finalized, None

    async def chat(
        self,
        question: str,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
        *,
        history: list[dict[str, str]] | None = None,
    ) -> tuple[dict[str, Any], AiPromptTrace | None]:
        if not self.config.ai.enabled:
            return {"answer": "AI is disabled in config.", "generation_model": "disabled"}, None
        try:
            return await AIExplanationEngine(self._provider()).chat(
                question,
                incident,
                hypotheses,
                events,
                history=history,
            )
        except OllamaError as exc:
            return {"answer": f"AI provider unavailable: {exc}", "generation_model": "ai_unavailable"}, None

    async def natural_language_search(
        self,
        query: str,
        event_store: EventStore,
        *,
        window_hours: int = 24,
        limit: int = 200,
    ) -> dict[str, Any]:
        if not self.config.ai.enabled:
            raise OllamaError("AI is disabled in configuration.")
        stripped = query.strip()
        if not stripped:
            raise OllamaError("Query is empty.")
        catalog = [str(item.get("service_id") or "") for item in event_store.list_services()]
        user_prompt = natural_language_search_user_prompt(stripped, service_catalog=catalog)
        messages = [
            {"role": "system", "content": NATURAL_LANGUAGE_SEARCH_SYSTEM},
            {"role": "user", "content": user_prompt},
        ]
        engine = AIExplanationEngine(self._provider())
        validated: NaturalLanguageSearchOutput | None = None
        attempts = self.config.ai.max_retries + 1
        for _ in range(attempts):
            raw = await engine.complete_chat_messages(messages)
            data = extract_json_object(raw)
            try:
                validated = NaturalLanguageSearchOutput.model_validate(data)
                break
            except ValidationError:
                continue
        if validated is None:
            raise OllamaError("Could not extract a valid JSON filter payload for natural language search.")
        filt = event_filter_from_nl_output(validated)
        window_end = utc_now()
        window_start = window_end - timedelta(hours=max(1, min(int(window_hours), 168)))
        bounded = max(1, min(int(limit), 500))
        events = list(event_store.query_time_range(window_start, window_end, filters=filt, limit=bounded))
        return {
            "query": stripped,
            "confidence": validated.confidence,
            "suggestions": list(validated.suggestions),
            "filter": serialized_event_filter(filt),
            "events": [event_to_dict(event) for event in events],
            "window_hours": max(1, min(int(window_hours), 168)),
        }

    def chat_stream(
        self,
        messages: list[dict[str, str]],
        model: str | None = None,
    ) -> AsyncIterator[OllamaStreamChunk]:
        return self._provider().chat_stream(messages, model)

    def incident_explain_stream(
        self,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> tuple[AsyncIterator[OllamaStreamChunk], AiPromptTrace] | tuple[None, None]:
        if not self.config.ai.enabled:
            return None, None
        engine = AIExplanationEngine(self._provider())
        messages, trace = engine.prepare_explain_incident(incident, hypotheses, events)
        return self._provider().chat_stream(messages), trace

    def incident_chat_stream(
        self,
        question: str,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
        *,
        history: list[dict[str, str]] | None = None,
    ) -> tuple[AsyncIterator[OllamaStreamChunk], AiPromptTrace]:
        engine = AIExplanationEngine(self._provider())
        messages, trace = engine.prepare_incident_chat(
            question,
            incident,
            hypotheses,
            events,
            history=history,
        )
        return self._provider().chat_stream(messages), trace

    def _provider(self) -> AsyncOllamaProvider:
        if self.config.ai.provider != "ollama":
            raise OllamaError(f"Unsupported AI provider: {self.config.ai.provider}")
        return AsyncOllamaProvider(self.config.ai)


def _model_to_dict(model: Any) -> dict[str, Any]:
    return {
        "name": model.name,
        "size": model.size,
        "context_window": model.context_window,
        "input_modes": list(model.input_modes),
        "family": model.family,
        "variant": model.variant,
        "local_weight": model.local_weight,
        "recommended_for": model.recommended_for,
        "quantization": model.quantization,
        "digest": model.digest,
        "release_date": model.release_date,
        "resolves_to": model.resolves_to,
        "forward_alias": model.forward_alias,
    }
