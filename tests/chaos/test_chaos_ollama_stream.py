from __future__ import annotations

from dataclasses import replace
from unittest.mock import MagicMock

import pytest

from ai.ollama import OllamaResponseError, OllamaStreamChunk
from ai.service import AIService
from config.models import InferraConfig


@pytest.mark.chaos
@pytest.mark.asyncio
async def test_explain_stream_failure_falls_back_to_template(monkeypatch: pytest.MonkeyPatch) -> None:
    cfg = replace(InferraConfig(), ai=replace(InferraConfig().ai, enabled=True, stream=True))

    async def failing_stream(*_a, **_k):
        yield OllamaStreamChunk(content='{"summary":', done=False)
        raise OllamaResponseError("Ollama HTTP 500: boom")

    fake_provider = MagicMock()
    fake_provider.config = cfg.ai
    fake_provider.chat_stream = failing_stream
    monkeypatch.setattr("ai.service.AsyncOllamaProvider", lambda *_a, **_k: fake_provider)

    svc = AIService(cfg)
    payload, trace = await svc.explain({"incident_id": "inc-1", "affected_services": []}, [], [])
    assert trace is None
    assert payload["generation_model"] == "template_fallback"
    assert "provider_unavailable" in (payload.get("guardrail_violations") or [])
