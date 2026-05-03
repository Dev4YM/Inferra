from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Any
from urllib.parse import urljoin

from config.model import AIConfig


class OllamaError(RuntimeError):
    pass


class OllamaConnectionError(OllamaError):
    pass


class OllamaResponseError(OllamaError):
    pass


@dataclass(frozen=True)
class OllamaStatus:
    available: bool
    base_url: str
    model: str
    installed: bool = False
    error: str | None = None


class OllamaProvider:
    def __init__(self, config: AIConfig) -> None:
        self.config = config
        self.base_url = config.base_url.rstrip("/") + "/"

    def list_models(self) -> list[str]:
        payload = self._request_json("GET", "/api/tags")
        return sorted(model.get("name", "") for model in payload.get("models", []) if model.get("name"))

    def show_model(self, model: str | None = None) -> dict[str, Any]:
        return self._request_json("POST", "/api/show", {"model": model or self.config.model})

    def pull_model(self, model: str | None = None) -> dict[str, Any]:
        return self._request_json("POST", "/api/pull", {"model": model or self.config.model, "stream": False}, timeout=None)

    def chat(self, messages: list[dict[str, str]], model: str | None = None) -> str:
        payload = self._request_json(
            "POST",
            "/api/chat",
            {
                "model": model or self.config.model,
                "messages": messages,
                "stream": False,
                "options": {
                    "temperature": self.config.temperature,
                    "top_p": self.config.top_p,
                    "top_k": self.config.top_k,
                },
            },
        )
        message = payload.get("message") or {}
        content = message.get("content")
        if not isinstance(content, str):
            raise OllamaResponseError("Ollama chat response did not include message.content")
        return content

    def test(self) -> str:
        return self.chat(
            [
                {"role": "system", "content": "You are Inferra's local AI health checker. Reply in one short sentence."},
                {"role": "user", "content": "Confirm that the model is ready for incident explanation."},
            ]
        )

    def status(self) -> OllamaStatus:
        try:
            models = self.list_models()
        except OllamaError as exc:
            return OllamaStatus(False, self.config.base_url, self.config.model, error=str(exc))
        return OllamaStatus(True, self.config.base_url, self.config.model, self.config.model in models)

    def _request_json(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
        timeout: float | None = 30.0,
    ) -> dict[str, Any]:
        timeout_value = self.config.timeout_seconds if timeout is not None else None
        data = None if payload is None else json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            urljoin(self.base_url, path.lstrip("/")),
            data=data,
            method=method,
            headers=self._headers(),
        )
        try:
            with urllib.request.urlopen(request, timeout=timeout_value) as response:
                body = response.read().decode("utf-8")
        except urllib.error.HTTPError as exc:
            detail = exc.read().decode("utf-8", errors="replace")
            raise OllamaResponseError(f"Ollama HTTP {exc.code}: {detail}") from exc
        except urllib.error.URLError as exc:
            raise OllamaConnectionError(f"Could not connect to Ollama at {self.config.base_url}: {exc.reason}") from exc
        except TimeoutError as exc:
            raise OllamaConnectionError(f"Timed out connecting to Ollama at {self.config.base_url}") from exc
        try:
            decoded = json.loads(body or "{}")
        except json.JSONDecodeError as exc:
            raise OllamaResponseError("Ollama returned invalid JSON") from exc
        if not isinstance(decoded, dict):
            raise OllamaResponseError("Ollama returned an unexpected JSON payload")
        return decoded

    def _headers(self) -> dict[str, str]:
        headers = {"Content-Type": "application/json", "Accept": "application/json"}
        if self.config.token_env:
            token = os.environ.get(self.config.token_env)
            if token:
                headers["Authorization"] = f"Bearer {token}"
        return headers
