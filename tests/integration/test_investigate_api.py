"""Tests for the structured AI investigation routes (read-only, deterministic fallback)."""

from __future__ import annotations

from fastapi.testclient import TestClient

from config.model import InferraConfig, StorageConfig
from web import create_app


def _ingest_two_errors(client: TestClient) -> str:
    client.post("/api/ingest", json={"service": "api", "level": "error", "message": "timeout calling postgres"})
    client.post("/api/ingest", json={"service": "api", "level": "error", "message": "connection refused from postgres"})
    incidents = client.get("/api/incidents").json()["incidents"]
    assert incidents, "expected at least one incident"
    return incidents[0]["incident_id"]


def test_investigate_now_returns_structured_output_without_ai(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        _ingest_two_errors(client)
        response = client.get("/api/investigate/now")
        assert response.status_code == 200
        payload = response.json()
        assert payload["used_ai"] is False, "AI is disabled by default; must use deterministic fallback"
        assert payload["fallback_reason"], "fallback_reason must explain why AI was not used"
        output = payload["output"]
        assert output["risk_level"] in {"low", "medium", "high", "critical"}
        assert isinstance(output["next_steps"], list)
        for step in output["next_steps"]:
            assert step["safety"] == "read_only"
            assert step["requires_user_action"] is True
        assert payload["focus"] == "overview"


def test_investigate_incident_includes_evidence_and_safe_steps(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        incident_id = _ingest_two_errors(client)
        response = client.get(f"/api/investigate/incident/{incident_id}")
        assert response.status_code == 200
        payload = response.json()
        output = payload["output"]
        assert payload["focus"].endswith(incident_id)
        assert isinstance(output["evidence"], list)
        assert isinstance(output["next_steps"], list)
        for step in output["next_steps"]:
            assert step["safety"] == "read_only"


def test_ai_ask_with_overview_scope_returns_investigation(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        _ingest_two_errors(client)
        response = client.post(
            "/api/ai/ask",
            json={"question": "what should I look at first?", "scope": "overview", "mode": "operator"},
        )
        assert response.status_code == 200
        payload = response.json()
        assert payload["question"] == "what should I look at first?"
        assert payload["focus"] == "overview"
        assert "headline" in payload["output"]


def test_ai_ask_with_latest_scope_resolves_to_latest_incident(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        incident_id = _ingest_two_errors(client)
        response = client.post(
            "/api/ai/ask",
            json={"question": "what changed most recently?", "scope": "latest", "mode": "expert"},
        )
        assert response.status_code == 200
        payload = response.json()
        assert payload["focus"] == f"incident:{incident_id}"
        assert payload["question"] == "what changed most recently?"


def test_ai_doctor_reports_disabled_provider_and_warnings(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        response = client.get("/api/ai/doctor")
        assert response.status_code == 200
        payload = response.json()
        assert payload["enabled"] is False
        assert payload["provider"] == "ollama"
        assert payload["ok"] is True, "AI disabled should report ok=True (graceful)"
        assert "guidance" in payload


def test_ai_report_returns_structured_payload(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        incident_id = _ingest_two_errors(client)
        response = client.get(f"/api/ai/report/{incident_id}", params={"mode": "operator"})
        assert response.status_code == 200
        payload = response.json()
        assert payload.get("report") is True
        assert payload["focus"].endswith(incident_id)


def test_investigate_incident_404_for_unknown_id(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        response = client.get("/api/investigate/incident/inc-does-not-exist")
        assert response.status_code == 404
