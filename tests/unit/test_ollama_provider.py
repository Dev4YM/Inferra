from config.model import AIConfig
from ai.ollama import OllamaProvider


class FakeOllamaProvider(OllamaProvider):
    def __init__(self, config):
        super().__init__(config)
        self.calls = []

    def _request_json(self, method, path, payload=None, timeout=30.0):
        self.calls.append((method, path, payload, timeout, self._headers()))
        if path == "/api/tags":
            return {"models": [{"name": "gemma4:e4b"}, {"name": "llama3.2"}]}
        if path == "/api/chat":
            return {"message": {"content": "ready"}}
        if path == "/api/pull":
            return {"status": "success"}
        if path == "/api/show":
            return {"model": payload["model"]}
        return {}


def test_ollama_provider_lists_models_and_status(monkeypatch):
    monkeypatch.setenv("OLLAMA_TOKEN", "secret-token")
    provider = FakeOllamaProvider(AIConfig(enabled=True, token_env="OLLAMA_TOKEN"))

    assert provider.list_models() == ["gemma4:e4b", "llama3.2"]
    status = provider.status()

    assert status.available is True
    assert status.installed is True
    assert provider.calls[-1][-1]["Authorization"] == "Bearer secret-token"


def test_ollama_provider_chat_and_pull_payloads():
    provider = FakeOllamaProvider(AIConfig(enabled=True, model="gemma4:e2b"))

    assert provider.chat([{"role": "user", "content": "hello"}]) == "ready"
    assert provider.pull_model()["status"] == "success"

    chat_call = provider.calls[0]
    pull_call = provider.calls[1]
    assert chat_call[2]["model"] == "gemma4:e2b"
    assert chat_call[2]["stream"] is False
    assert pull_call[2] == {"model": "gemma4:e2b", "stream": False}
