from __future__ import annotations

import pytest
from aioresponses import aioresponses
from yarl import URL

from ai.ollama import AsyncOllamaProvider, OllamaRemoteDisabledError
from config.model import AIConfig


BASE_URL = "http://127.0.0.1:11434"


@pytest.mark.asyncio
async def test_ollama_provider_streaming_chat_collects_content() -> None:
    provider = AsyncOllamaProvider(AIConfig(enabled=True, model="gemma4:e4b"))
    body = (
        b'{"message":{"content":"ready"},"done":false}\n'
        b'{"message":{"content":" now"},"done":false}\n'
        b'{"message":{"content":""},"done":true}\n'
    )
    with aioresponses() as mocked:
        mocked.post(f"{BASE_URL}/api/chat", body=body)

        chunks = [chunk async for chunk in provider.chat_stream([{"role": "user", "content": "hello"}])]

    assert "".join(chunk.content for chunk in chunks) == "ready now"
    assert chunks[-1].done is True


@pytest.mark.asyncio
async def test_ollama_provider_pull_progress_emits_incremental_events() -> None:
    provider = AsyncOllamaProvider(AIConfig(enabled=True, model="gemma4:e4b"))
    body = (
        b'{"status":"pulling manifest"}\n'
        b'{"status":"downloading","digest":"sha256:abc","total":100,"completed":25}\n'
        b'{"status":"downloading","digest":"sha256:abc","total":100,"completed":100}\n'
        b'{"status":"success"}\n'
    )
    with aioresponses() as mocked:
        mocked.post(f"{BASE_URL}/api/pull", body=body)

        progress = [event async for event in provider.pull_model_stream("gemma4:e4b")]

    assert [event.status for event in progress] == ["pulling manifest", "downloading", "downloading", "success"]
    assert progress[1].percent == 25.0
    assert progress[2].percent == 100.0


@pytest.mark.asyncio
async def test_ollama_provider_status_unavailable_when_connection_refused() -> None:
    provider = AsyncOllamaProvider(AIConfig(enabled=True, base_url="http://127.0.0.1:9"))

    status = await provider.status()

    assert status.available is False
    assert status.reason == "connection_error"
    assert "Could not connect to Ollama" in (status.error or "")


@pytest.mark.asyncio
async def test_bearer_token_injected_only_when_env_is_configured_and_populated(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("OLLAMA_TOKEN", raising=False)
    provider = AsyncOllamaProvider(AIConfig(enabled=True, token_env="OLLAMA_TOKEN"))
    with aioresponses() as mocked:
        mocked.get(f"{BASE_URL}/api/tags", payload={"models": []})

        assert await provider.list_models() == []

        request = mocked.requests[("GET", URL(f"{BASE_URL}/api/tags"))][0]
        assert "Authorization" not in request.kwargs["headers"]

    monkeypatch.setenv("OLLAMA_TOKEN", "secret-token")
    provider = AsyncOllamaProvider(AIConfig(enabled=True, token_env="OLLAMA_TOKEN"))
    with aioresponses() as mocked:
        mocked.get(f"{BASE_URL}/api/tags", payload={"models": []})

        assert await provider.list_models() == []

        request = mocked.requests[("GET", URL(f"{BASE_URL}/api/tags"))][0]
        assert request.kwargs["headers"]["Authorization"] == "Bearer secret-token"


def test_allow_remote_false_blocks_non_local_base_url() -> None:
    with pytest.raises(OllamaRemoteDisabledError):
        AsyncOllamaProvider(AIConfig(enabled=True, base_url="https://ollama.example.com", allow_remote=False))
