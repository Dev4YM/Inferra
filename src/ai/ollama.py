from __future__ import annotations

import asyncio
import json
import os
from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from time import perf_counter
from typing import Any

import aiohttp

from ai.registry import choose_available_gemma_model
from config.model import AIConfig


class OllamaError(RuntimeError):
    reason_code = "ollama_error"


class OllamaConnectionError(OllamaError):
    reason_code = "connection_error"


class OllamaRemoteDisabledError(OllamaConnectionError):
    reason_code = "remote_disabled"


class OllamaResponseError(OllamaError):
    reason_code = "response_error"


@dataclass(frozen=True)
class OllamaStatus:
    available: bool
    base_url: str
    model: str
    resolved_model: str | None = None
    installed: bool = False
    version: str | None = None
    models: tuple[str, ...] = ()
    show: dict[str, Any] | None = None
    test_response: str | None = None
    latency_ms: float | None = None
    reason: str | None = None
    error: str | None = None


@dataclass(frozen=True)
class OllamaStreamChunk:
    content: str
    done: bool
    raw: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class OllamaPullProgress:
    status: str
    digest: str | None = None
    total: int | None = None
    completed: int | None = None
    raw: dict[str, Any] = field(default_factory=dict)

    @property
    def percent(self) -> float | None:
        if not self.total or self.completed is None:
            return None
        return round((self.completed / self.total) * 100, 1)


class AsyncOllamaProvider:
    def __init__(self, config: AIConfig) -> None:
        self.config = config
        self.base_url = config.base_url.strip().rstrip("/")
        if not self.base_url:
            raise OllamaConnectionError("Ollama base_url is empty")
        if not config.allow_remote and not _is_local_base_url(self.base_url):
            raise OllamaRemoteDisabledError(
                "Refusing to connect to non-local Ollama server while ai.allow_remote is false"
            )

    async def version(self) -> str | None:
        payload = await self._request_json("GET", "/api/version")
        version = payload.get("version")
        return str(version) if version is not None else None

    async def list_models(self) -> list[str]:
        payload = await self._request_json("GET", "/api/tags")
        return sorted(model.get("name", "") for model in payload.get("models", []) if model.get("name"))

    async def show_model(self, model: str | None = None) -> dict[str, Any]:
        return await self._request_json("POST", "/api/show", {"model": model or self.config.model})

    async def pull_model(self, model: str | None = None) -> dict[str, Any]:
        return await self._request_json("POST", "/api/pull", {"model": model or self.config.model, "stream": False})

    def pull_model_stream(self, model: str | None = None) -> AsyncIterator[OllamaPullProgress]:
        payload = {"model": model or self.config.model, "stream": True}
        return self._pull_stream(payload)

    async def chat(
        self,
        messages: list[dict[str, str]],
        model: str | None = None,
        *,
        retries: int = 0,
    ) -> str:
        payload = await self._request_json("POST", "/api/chat", self._chat_payload(messages, model, stream=False), retries=retries)
        message = payload.get("message") or {}
        content = message.get("content")
        if not isinstance(content, str):
            raise OllamaResponseError("Ollama chat response did not include message.content")
        return content

    def chat_stream(
        self,
        messages: list[dict[str, str]],
        model: str | None = None,
    ) -> AsyncIterator[OllamaStreamChunk]:
        return self._content_stream("/api/chat", self._chat_payload(messages, model, stream=True), "message")

    async def generate(
        self,
        prompt: str,
        model: str | None = None,
        *,
        retries: int = 0,
    ) -> str:
        payload = await self._request_json(
            "POST",
            "/api/generate",
            self._generate_payload(prompt, model, stream=False),
            retries=retries,
        )
        response = payload.get("response")
        if not isinstance(response, str):
            raise OllamaResponseError("Ollama generate response did not include response")
        return response

    def generate_stream(self, prompt: str, model: str | None = None) -> AsyncIterator[OllamaStreamChunk]:
        return self._content_stream("/api/generate", self._generate_payload(prompt, model, stream=True), "response")

    async def test(self, model: str | None = None) -> str:
        return await self.chat(
            [
                {"role": "system", "content": "You are Inferra's local AI health checker. Reply in one short sentence."},
                {"role": "user", "content": "Confirm that the model is ready for incident explanation."},
            ],
            model=model,
        )

    async def status(self) -> OllamaStatus:
        start = perf_counter()
        try:
            version = await self.version()
            models = tuple(await self.list_models())
            resolved_model = choose_available_gemma_model(self.config.model, models)
            installed = resolved_model in models
            show = await self.show_model(resolved_model) if installed else None
            test_response = await self.test(resolved_model) if installed else None
        except OllamaError as exc:
            return OllamaStatus(
                available=False,
                base_url=self.config.base_url,
                model=self.config.model,
                resolved_model=None,
                latency_ms=round((perf_counter() - start) * 1000, 1),
                reason=getattr(exc, "reason_code", "ollama_error"),
                error=str(exc),
            )
        return OllamaStatus(
            available=True,
            base_url=self.config.base_url,
            model=self.config.model,
            resolved_model=resolved_model,
            installed=installed,
            version=version,
            models=models,
            show=show,
            test_response=test_response,
            latency_ms=round((perf_counter() - start) * 1000, 1),
            reason=None if installed else "model_not_installed",
        )

    async def _request_json(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
        *,
        retries: int = 0,
    ) -> dict[str, Any]:
        async for attempt in _retry_attempts(retries):
            try:
                async with self._session() as session:
                    async with session.request(method, self._url(path), json=payload) as response:
                        body = await response.text()
                        if response.status >= 400:
                            raise OllamaResponseError(f"Ollama HTTP {response.status}: {body}")
                return _decode_json_object(body)
            except (OllamaConnectionError, OllamaResponseError):
                if attempt >= retries:
                    raise
                await asyncio.sleep(0.25 * (2**attempt))
            except (aiohttp.ClientError, asyncio.TimeoutError) as exc:
                if attempt >= retries:
                    raise OllamaConnectionError(
                        f"Could not connect to Ollama at {self.config.base_url}: {exc}"
                    ) from exc
                await asyncio.sleep(0.25 * (2**attempt))
        raise OllamaConnectionError(f"Could not connect to Ollama at {self.config.base_url}")

    async def _content_stream(
        self,
        path: str,
        payload: dict[str, Any],
        content_key: str,
    ) -> AsyncIterator[OllamaStreamChunk]:
        async for event in self._stream_json(path, payload):
            content = ""
            if content_key == "message":
                message = event.get("message") or {}
                if isinstance(message, dict):
                    content = str(message.get("content") or "")
            else:
                content = str(event.get(content_key) or "")
            yield OllamaStreamChunk(content=content, done=bool(event.get("done", False)), raw=event)

    async def _pull_stream(self, payload: dict[str, Any]) -> AsyncIterator[OllamaPullProgress]:
        async for event in self._stream_json("/api/pull", payload):
            yield OllamaPullProgress(
                status=str(event.get("status") or ""),
                digest=_optional_str(event.get("digest")),
                total=_optional_int(event.get("total")),
                completed=_optional_int(event.get("completed")),
                raw=event,
            )

    async def _stream_json(self, path: str, payload: dict[str, Any]) -> AsyncIterator[dict[str, Any]]:
        try:
            async with self._session() as session:
                async with session.post(self._url(path), json=payload) as response:
                    if response.status >= 400:
                        body = await response.text()
                        raise OllamaResponseError(f"Ollama HTTP {response.status}: {body}")
                    async for raw_line in response.content:
                        line = raw_line.decode("utf-8").strip()
                        if not line:
                            continue
                        yield _decode_json_object(line)
        except (aiohttp.ClientError, asyncio.TimeoutError) as exc:
            raise OllamaConnectionError(f"Could not connect to Ollama at {self.config.base_url}: {exc}") from exc

    def _chat_payload(self, messages: list[dict[str, str]], model: str | None, *, stream: bool) -> dict[str, Any]:
        return {
            "model": model or self.config.model,
            "messages": messages,
            "stream": stream,
            "options": self._options(),
        }

    def _generate_payload(self, prompt: str, model: str | None, *, stream: bool) -> dict[str, Any]:
        return {
            "model": model or self.config.model,
            "prompt": prompt,
            "stream": stream,
            "options": self._options(),
        }

    def _options(self) -> dict[str, Any]:
        return {
            "temperature": self.config.temperature,
            "top_p": self.config.top_p,
            "top_k": self.config.top_k,
            "num_predict": self.config.max_tokens,
        }

    def _headers(self) -> dict[str, str]:
        headers = {"Accept": "application/json"}
        if self.config.token_env:
            token = os.environ.get(self.config.token_env)
            if token:
                headers["Authorization"] = f"Bearer {token}"
        return headers

    def _session(self) -> aiohttp.ClientSession:
        timeout = aiohttp.ClientTimeout(
            total=self.config.timeout_seconds,
            connect=self.config.connect_timeout_seconds,
            sock_read=self.config.read_timeout_seconds,
        )
        return aiohttp.ClientSession(timeout=timeout, headers=self._headers())

    def _url(self, path: str) -> str:
        return f"{self.base_url}/{path.lstrip('/')}"


class OllamaProvider:
    def __init__(self, config: AIConfig) -> None:
        self.config = config
        self._async_provider = AsyncOllamaProvider(config)

    def list_models(self) -> list[str]:
        return self._run(self._async_provider.list_models())

    def show_model(self, model: str | None = None) -> dict[str, Any]:
        return self._run(self._async_provider.show_model(model))

    def pull_model(self, model: str | None = None) -> dict[str, Any]:
        return self._run(self._async_provider.pull_model(model))

    def chat(self, messages: list[dict[str, str]], model: str | None = None) -> str:
        return self._run(self._async_provider.chat(messages, model))

    def test(self) -> str:
        return self._run(self._async_provider.test())

    def status(self) -> OllamaStatus:
        return self._run(self._async_provider.status())

    def _headers(self) -> dict[str, str]:
        return self._async_provider._headers()

    def _run(self, coroutine: Any) -> Any:
        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return asyncio.run(coroutine)
        coroutine.close()
        raise OllamaError("OllamaProvider cannot run inside an active event loop; use AsyncOllamaProvider")


def _decode_json_object(body: str) -> dict[str, Any]:
    try:
        decoded = json.loads(body or "{}")
    except json.JSONDecodeError as exc:
        raise OllamaResponseError("Ollama returned invalid JSON") from exc
    if not isinstance(decoded, dict):
        raise OllamaResponseError("Ollama returned an unexpected JSON payload")
    return decoded


async def _retry_attempts(retries: int) -> AsyncIterator[int]:
    for attempt in range(retries + 1):
        yield attempt


def _is_local_base_url(base_url: str) -> bool:
    authority = base_url.split("://", 1)[-1].split("/", 1)[0].rsplit("@", 1)[-1]
    if authority.startswith("["):
        host = authority[1:].split("]", 1)[0].lower()
    else:
        host = authority.split(":", 1)[0].lower()
    return host in {"localhost", "127.0.0.1", "::1"}


def _optional_str(value: Any) -> str | None:
    return str(value) if value is not None else None


def _optional_int(value: Any) -> int | None:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None
