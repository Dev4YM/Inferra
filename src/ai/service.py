from __future__ import annotations

from typing import Any

from ai.explainer import AIExplanationEngine, explanation_error
from ai.ollama import OllamaError, OllamaProvider
from ai.registry import gemma4_model, list_gemma4_models
from config.model import InferraConfig
from events.models import NormalizedEvent


class AIService:
    def __init__(self, config: InferraConfig) -> None:
        self.config = config

    def status(self) -> dict[str, Any]:
        ai = self.config.ai
        registry_model = gemma4_model(ai.model)
        payload: dict[str, Any] = {
            "enabled": ai.enabled,
            "provider": ai.provider,
            "base_url": ai.base_url,
            "model": ai.model,
            "token_env": ai.token_env,
            "registry_model": _model_to_dict(registry_model) if registry_model else None,
        }
        if not ai.enabled:
            payload["available"] = False
            payload["reason"] = "AI is disabled in config."
            return payload
        try:
            provider = self._provider()
            status = provider.status()
        except OllamaError as exc:
            payload["available"] = False
            payload["error"] = str(exc)
            return payload
        payload.update(
            {
                "available": status.available,
                "installed": status.installed,
                "error": status.error,
            }
        )
        return payload

    def registry(self) -> list[dict[str, Any]]:
        return [_model_to_dict(model) for model in list_gemma4_models()]

    def installed_models(self) -> list[str]:
        return self._provider().list_models()

    def pull_model(self, model: str) -> dict[str, Any]:
        return self._provider().pull_model(model)

    def test(self) -> str:
        return self._provider().test()

    def explain(
        self,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> dict[str, Any] | None:
        if not self.config.ai.enabled:
            return None
        try:
            return AIExplanationEngine(self._provider()).generate(incident, hypotheses, events)
        except OllamaError as exc:
            return explanation_error(incident["incident_id"], exc)

    def chat(
        self,
        question: str,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> dict[str, Any]:
        if not self.config.ai.enabled:
            return {"answer": "AI is disabled in config.", "generation_model": "disabled"}
        try:
            return AIExplanationEngine(self._provider()).chat(question, incident, hypotheses, events)
        except OllamaError as exc:
            return {"answer": f"AI provider unavailable: {exc}", "generation_model": "ai_unavailable"}

    def _provider(self) -> OllamaProvider:
        if self.config.ai.provider != "ollama":
            raise OllamaError(f"Unsupported AI provider: {self.config.ai.provider}")
        return OllamaProvider(self.config.ai)


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
    }
