from fastapi.testclient import TestClient

from config.model import InferraConfig, StorageConfig
from web import create_app


def test_api_ingest_incident_hypothesis_and_explanation(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))

    with TestClient(app) as client:
        first = client.post(
            "/api/ingest",
            json={"service": "api", "level": "error", "message": "timeout calling postgres"},
        )
        second = client.post(
            "/api/ingest",
            json={"service": "api", "level": "error", "message": "connection refused from postgres"},
        )
        assert first.status_code == 200
        assert second.status_code == 200

        incidents = client.get("/api/incidents").json()["incidents"]
        assert len(incidents) == 1
        incident_id = incidents[0]["incident_id"]

        detail = client.get(f"/api/incidents/{incident_id}").json()
        assert detail["hypotheses"]
        assert detail["clusters"]

        explanation = client.get(f"/api/incidents/{incident_id}/explanation").json()["explanation"]
        assert explanation["generation_model"] == "template_fallback"

        services = client.get("/api/services").json()["services"]
        assert services[0]["service_id"] == "api"


def test_api_topology_edges_are_persisted(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))

    with TestClient(app) as client:
        response = client.post(
            "/api/topology/edges",
            json={"source": "api", "target": "postgres", "relation_type": "depends_on"},
        )
        assert response.status_code == 200

        edges = client.get("/api/topology").json()["edges"]
        assert edges == [{"source": "api", "target": "postgres", "relation_type": "depends_on"}]


def test_api_ai_status_and_chat_disabled_by_default(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))

    with TestClient(app) as client:
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "timeout calling postgres"})
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "connection refused from postgres"})
        incident_id = client.get("/api/incidents").json()["incidents"][0]["incident_id"]

        status = client.get("/api/ai/status").json()
        models = client.get("/api/ai/models").json()
        chat = client.post(f"/api/incidents/{incident_id}/chat", json={"question": "What happened?"}).json()

        assert status["enabled"] is False
        assert len(models["registry"]) == 29
        assert chat["generation_model"] == "disabled"


def test_api_ai_config_update_persists_when_config_path_is_available(tmp_path):
    config_path = tmp_path / "inferra.toml"
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)), config_path=config_path)

    with TestClient(app) as client:
        response = client.post(
            "/api/ai/config",
            json={
                "enabled": True,
                "provider": "ollama",
                "base_url": "http://localhost:11434",
                "model": "gemma4:e2b",
                "token_env": "OLLAMA_TOKEN",
            },
        )

        assert response.status_code == 200
        payload = response.json()
        assert payload["enabled"] is True
        assert payload["model"] == "gemma4:e2b"
        assert "model = \"gemma4:e2b\"" in config_path.read_text(encoding="utf-8")


def test_api_collector_health_endpoint(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))

    with TestClient(app) as client:
        response = client.get("/api/collectors")

        assert response.status_code == 200
        payload = response.json()
        assert "collectors" in payload
        assert "queue_depth" in payload


def test_api_collector_start_stop_controls(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))

    with TestClient(app) as client:
        start = client.post("/api/collectors/start")
        stop = client.post("/api/collectors/stop")

        assert start.status_code == 200
        assert start.json()["started"] is True
        assert stop.status_code == 200
        assert stop.json()["stopped"] is True
