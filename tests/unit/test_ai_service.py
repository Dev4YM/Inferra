from __future__ import annotations

import json
from dataclasses import replace
from datetime import UTC, datetime
from unittest.mock import AsyncMock, MagicMock

import pytest

from ai.ollama import AsyncOllamaProvider, OllamaError, OllamaStatus
from ai.service import AIService
from config.models import InferraConfig
from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline
from storage import initialize_storage


@pytest.mark.asyncio
async def test_status_when_ai_disabled() -> None:
    svc = AIService(InferraConfig())
    payload = await svc.status()
    assert payload["enabled"] is False
    assert payload["available"] is False


@pytest.mark.asyncio
async def test_status_maps_ollama_error(monkeypatch: pytest.MonkeyPatch) -> None:
    cfg = replace(InferraConfig(), ai=replace(InferraConfig().ai, enabled=True))

    async def boom() -> OllamaStatus:
        raise OllamaError("down")

    fake_provider = MagicMock(spec=AsyncOllamaProvider)
    fake_provider.status = boom
    monkeypatch.setattr("ai.service.AsyncOllamaProvider", lambda *_a, **_k: fake_provider)

    svc = AIService(cfg)
    payload = await svc.status()
    assert payload["available"] is False
    assert "error" in payload


@pytest.mark.asyncio
async def test_explain_when_disabled_returns_none() -> None:
    svc = AIService(InferraConfig())
    assert await svc.explain({"incident_id": "x"}, [], []) is None


@pytest.mark.asyncio
async def test_explain_ollama_error_returns_error_payload(monkeypatch: pytest.MonkeyPatch) -> None:
    cfg = replace(InferraConfig(), ai=replace(InferraConfig().ai, enabled=True))

    async def boom(*_a, **_k):
        raise OllamaError("unreachable")

    monkeypatch.setattr("ai.service.AIExplanationEngine.generate", boom)

    svc = AIService(cfg)
    payload, trace = await svc.explain({"incident_id": "inc-1"}, [], [])
    assert trace is None
    assert payload["generation_model"] == "template_fallback"
    assert "provider_unavailable" in (payload.get("guardrail_violations") or [])


@pytest.mark.asyncio
async def test_chat_disabled_message() -> None:
    svc = AIService(InferraConfig())
    answer, trace = await svc.chat("q", {"incident_id": "i"}, [], [])
    assert trace is None
    assert answer["generation_model"] == "disabled"


@pytest.mark.asyncio
async def test_chat_ollama_error_message(monkeypatch: pytest.MonkeyPatch) -> None:
    cfg = replace(InferraConfig(), ai=replace(InferraConfig().ai, enabled=True))

    async def boom(*_a, **_k):
        raise OllamaError("bad")

    monkeypatch.setattr("ai.service.AIExplanationEngine.chat", boom)

    svc = AIService(cfg)
    answer, trace = await svc.chat("q", {"incident_id": "i"}, [], [])
    assert trace is None
    assert "unavailable" in answer["answer"]


@pytest.mark.asyncio
async def test_natural_language_search_requires_enabled_ai(tmp_path) -> None:
    cfg = InferraConfig()
    cfg = replace(cfg, storage=replace(cfg.storage, data_dir=tmp_path))
    event_store, *_rest = initialize_storage(
        tmp_path,
        events_db_name="events.db",
        incidents_db_name="incidents.db",
        retention_hours=cfg.storage.retention_hours,
        prune_interval_seconds=cfg.storage.prune_interval_seconds,
        wal_mode=cfg.storage.wal_mode,
        mmap_size_bytes=0,
        archive_after_days=cfg.incident_lifecycle.archive_after_days,
    )
    svc = AIService(cfg)
    with pytest.raises(OllamaError, match="disabled"):
        await svc.natural_language_search("svc-a errors", event_store)


@pytest.mark.asyncio
async def test_natural_language_search_success(monkeypatch: pytest.MonkeyPatch, tmp_path) -> None:
    cfg = replace(InferraConfig(), ai=replace(InferraConfig().ai, enabled=True, stream=False, max_retries=0))
    cfg = replace(cfg, storage=replace(cfg.storage, data_dir=tmp_path))

    event_store, *_rest = initialize_storage(
        tmp_path,
        events_db_name="events.db",
        incidents_db_name="incidents.db",
        retention_hours=cfg.storage.retention_hours,
        prune_interval_seconds=cfg.storage.prune_interval_seconds,
        wal_mode=cfg.storage.wal_mode,
        mmap_size_bytes=0,
        archive_after_days=cfg.incident_lifecycle.archive_after_days,
    )

    pipeline = NormalizationPipeline()
    raw = RawEvent(
        source_type="app",
        source_id="t",
        raw_payload='{"service":"api","level":"error","message":"boom"}',
        collected_at=datetime.now(UTC),
        metadata={},
    )
    event_store.add_event(pipeline.normalize(raw))

    async def fake_complete(self, messages: list[dict[str, str]]) -> str:
        return json.dumps(
            {
                "confidence": 0.9,
                "suggestions": [],
                "filter": {
                    "service_ids": ["api"],
                    "severities": ["ERROR"],
                    "message_contains": None,
                },
            }
        )

    monkeypatch.setattr("ai.explainer.AIExplanationEngine.complete_chat_messages", fake_complete)

    svc = AIService(cfg)
    payload = await svc.natural_language_search("errors on api", event_store)
    assert payload["confidence"] == 0.9
    assert payload["events"]


@pytest.mark.asyncio
async def test_installed_models_delegates(monkeypatch: pytest.MonkeyPatch) -> None:
    cfg = replace(InferraConfig(), ai=replace(InferraConfig().ai, enabled=True))
    fake_provider = MagicMock(spec=AsyncOllamaProvider)
    fake_provider.list_models = AsyncMock(return_value=["m1"])
    monkeypatch.setattr("ai.service.AsyncOllamaProvider", lambda *_a, **_k: fake_provider)
    svc = AIService(cfg)
    assert await svc.installed_models() == ["m1"]


def test_pull_model_stream_delegates(monkeypatch: pytest.MonkeyPatch) -> None:
    cfg = replace(InferraConfig(), ai=replace(InferraConfig().ai, enabled=True))
    fake_provider = MagicMock(spec=AsyncOllamaProvider)
    fake_provider.pull_model_stream.return_value = iter(())
    monkeypatch.setattr("ai.service.AsyncOllamaProvider", lambda *_a, **_k: fake_provider)
    svc = AIService(cfg)
    gen = svc.pull_model_stream("gemma4:e4b")
    assert gen is not None


@pytest.mark.asyncio
async def test_incident_chat_stream_prepares_trace(monkeypatch: pytest.MonkeyPatch) -> None:
    cfg = replace(InferraConfig(), ai=replace(InferraConfig().ai, enabled=True, stream=True))

    async def empty_stream(*_a, **_k):
        if False:
            yield None

    fake_provider = MagicMock(spec=AsyncOllamaProvider)
    fake_provider.config = cfg.ai
    fake_provider.chat_stream.return_value = empty_stream()
    monkeypatch.setattr("ai.service.AsyncOllamaProvider", lambda *_a, **_k: fake_provider)

    pipeline = NormalizationPipeline()
    ev = pipeline.normalize(
        RawEvent(
            source_type="app",
            source_id="t",
            raw_payload='{"service":"api","level":"error","message":"x"}',
            collected_at=datetime.now(UTC),
            metadata={},
        )
    )
    svc = AIService(cfg)
    stream, trace = svc.incident_chat_stream("why?", {"incident_id": "i"}, [{"hypothesis_id": "h"}], [ev])
    assert trace.trace_kind == "chat"
    chunks = [c async for c in stream]
    assert chunks == []
