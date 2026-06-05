from fastapi.testclient import TestClient
import pytest

from config.model import AnomalyDetectionConfig, InferraConfig, StorageConfig
from web import create_app

pytestmark = pytest.mark.legacy_runtime


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

        state_log = client.get(f"/api/incidents/{incident_id}/state-log").json()
        assert state_log["incident_id"] == incident_id
        assert len(state_log["entries"]) >= 2
        states_seen = {entry["new_state"] for entry in state_log["entries"]}
        assert "open" in states_seen
        assert "investigating" in states_seen

        explanation = client.get(f"/api/incidents/{incident_id}/explanation").json()["explanation"]
        assert explanation["generation_model"] == "template_fallback"

        services = client.get("/api/services").json()["services"]
        assert services[0]["service_id"] == "api"


def test_api_anomaly_service_status_returns_buckets(tmp_path):
    cfg = InferraConfig(
        storage=StorageConfig(data_dir=tmp_path),
        anomaly_detection=AnomalyDetectionConfig(cold_start_hours=0),
    )
    app = create_app(cfg)

    with TestClient(app) as client:
        for _ in range(4):
            client.post(
                "/api/ingest",
                json={"service": "api", "level": "error", "message": "failure burst"},
            )
        response = client.get("/api/anomaly/api/status")
        assert response.status_code == 200
        body = response.json()
        assert body["enabled"] is True
        assert body["service_id"] == "api"
        assert body["status"] in ("learning", "active")
        assert "buckets" in body


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
        assert len(models["registry"]) == 58
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


def test_api_collector_single_start_stop(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))

    with TestClient(app) as client:
        health = client.get("/api/collectors").json()
        ids = [row["collector_id"] for row in health.get("collectors", [])]
        if not ids:
            pytest.skip("no collectors in default config for this test")
        cid = ids[0]
        assert client.post("/api/collectors/start").status_code == 200
        r1 = client.post("/api/collectors/one/stop", params={"collector_id": cid})
        assert r1.status_code == 200
        assert r1.json()["stopped"] is True
        r2 = client.post("/api/collectors/one/start", params={"collector_id": cid})
        assert r2.status_code == 200
        assert r2.json()["started"] is True


def test_api_dashboard_logs_event_detail_and_ai_trace(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))

    with TestClient(app) as client:
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "timeout calling postgres"})
        client.post(
            "/api/ingest",
            json={"service": "api", "level": "error", "message": "connection refused from postgres"},
        )

        dashboard = client.get("/api/dashboard")
        assert dashboard.status_code == 200
        data = dashboard.json()
        assert data["severity_counts"]["error"] >= 1
        assert "dedup" in data
        assert "total_suppressed" in data["dedup"]
        assert "noise" in data
        assert "blocklist_hits" in data["noise"]
        assert "routine_fingerprints" in data["noise"]

        logs = client.get("/api/logs?service=api&severity=3&search=timeout").json()["logs"]
        assert len(logs) == 1
        assert logs[0]["service_id"] == "api"

        event_detail = client.get(f"/api/events/{logs[0]['event_id']}")
        assert event_detail.status_code == 200
        assert event_detail.json()["event"]["message"] == "timeout calling postgres"

        incident_id = client.get("/api/incidents").json()["incidents"][0]["incident_id"]
        trace = client.get(f"/api/incidents/{incident_id}/ai-trace")
        assert trace.status_code == 200
        payload = trace.json()
        assert payload["redaction"]["raw_logs_sent"] is False
        assert payload["included_events"]


def test_api_incident_feedback_and_hypothesis_breakdown(tmp_path):
    from dataclasses import fields

    from core.models import ScoreBreakdown

    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))

    with TestClient(app) as client:
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "timeout calling postgres"})
        client.post(
            "/api/ingest",
            json={"service": "api", "level": "error", "message": "connection refused from postgres"},
        )
        incident_id = client.get("/api/incidents").json()["incidents"][0]["incident_id"]
        hyp = client.get(f"/api/incidents/{incident_id}/hypotheses").json()["hypotheses"]
        assert hyp
        top = hyp[0]
        assert set(top["score_breakdown"].keys()) == {f.name for f in fields(ScoreBreakdown)}
        assert top["confidence_label"] in ("high", "medium", "low")
        fb = client.post(
            f"/api/incidents/{incident_id}/feedback",
            json={"was_correct": True, "correct_hypothesis_id": top["hypothesis_id"], "notes": "ok"},
        )
        assert fb.status_code == 200
        assert fb.json()["stored"] is True


def test_api_version_config_metrics_and_ai_trace_alias(tmp_path):
    from dataclasses import replace

    base_dir = tmp_path / "one"
    base_dir.mkdir()
    base = InferraConfig(storage=StorageConfig(data_dir=base_dir))
    app = create_app(base)
    with TestClient(app) as client:
        ver = client.get("/api/version")
        assert ver.status_code == 200
        assert ver.json()["name"] == "inferra"

        cfg = client.get("/api/config").json()["config"]
        cfg["logging"]["level"] = "DEBUG"
        put = client.put("/api/config", json={"config": cfg})
        assert put.status_code == 200
        assert client.get("/api/config").json()["config"]["logging"]["level"] == "DEBUG"

        assert client.get("/api/metrics").status_code == 404

        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "alias trace a"})
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "alias trace b"})
        incident_id = client.get("/api/incidents").json()["incidents"][0]["incident_id"]
        a = client.get(f"/api/ai/trace/{incident_id}").json()
        b = client.get(f"/api/incidents/{incident_id}/ai-trace").json()
        assert a == b

    mdir = tmp_path / "metrics"
    mdir.mkdir()
    metrics_cfg = InferraConfig(
        storage=StorageConfig(data_dir=mdir),
        server=replace(InferraConfig().server, expose_prometheus_metrics=True),
    )
    with TestClient(create_app(metrics_cfg)) as client:
        assert "inferra_events_total" in client.get("/api/metrics").text


def test_websocket_live_receives_event_count(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        with client.websocket_connect("/ws") as ws:
            client.post(
                "/api/ingest",
                json={"service": "api", "level": "info", "message": "hello websocket"},
            )
            types: list[str] = []
            for _ in range(30):
                msg = ws.receive_json()
                types.append(str(msg.get("type")))
                if msg.get("type") == "event_count":
                    assert "total" in msg
                    break
            else:
                raise AssertionError(f"expected event_count, saw: {types}")


def test_api_rest_surface_core_gets(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "one"})
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "two"})
        events = client.get("/api/events?limit=10").json()["events"]
        assert events
        bad = client.get("/api/events/does-not-exist")
        assert bad.status_code == 404

        incident_id = client.get("/api/incidents").json()["incidents"][0]["incident_id"]
        resolved = client.post(f"/api/incidents/{incident_id}/resolve", json={"resolved_by": "t"})
        assert resolved.status_code == 200
        assert resolved.json()["resolved"] is True


def test_api_search_natural_requires_ai_or_returns_error(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        r = client.get("/api/search/natural?q=api")
        assert r.status_code == 503


def test_api_services_detail_and_missing(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        client.post("/api/ingest", json={"service": "billing", "level": "info", "message": "hello"})
        detail = client.get("/api/services/billing")
        assert detail.status_code == 200
        assert detail.json()["service"]["service_id"] == "billing"
        ev = client.get("/api/services/billing/events?limit=5").json()["events"]
        assert ev

        missing = client.get("/api/services/does-not-exist")
        assert missing.status_code == 404


def test_websocket_ai_pull_streams_progress(monkeypatch, tmp_path):
    async def fake_pull_stream(self, model):
        class _P:
            status = "done"
            digest = ""
            total = 0
            completed = 0
            percent = 100.0

        yield _P()

    monkeypatch.setattr("web.api.AIService.pull_model_stream", fake_pull_stream)

    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))

    with TestClient(app) as client:
        with client.websocket_connect("/api/ai/pull") as ws:
            ws.send_json({"model": "gemma4:e4b"})
            msgs = []
            for _ in range(10):
                msg = ws.receive_json()
                msgs.append(msg.get("type"))
                if msg.get("type") == "done":
                    break
            assert "progress" in msgs or "done" in msgs


def test_incident_events_alias_route(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "one"})
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "two"})
        incident_id = client.get("/api/incidents").json()["incidents"][0]["incident_id"]
        body = client.get(f"/api/incidents/{incident_id}/events").json()
        assert len(body["events"]) >= 2


def test_chat_messages_endpoint_roundtrip(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "x"})
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "y"})
        incident_id = client.get("/api/incidents").json()["incidents"][0]["incident_id"]
        client.post(f"/api/incidents/{incident_id}/chat", json={"question": "ping"})
        rows = client.get(f"/api/incidents/{incident_id}/chat/messages").json()["messages"]
        assert len(rows) >= 2
        roles = {row["role"] for row in rows}
        assert "user" in roles
        assert "assistant" in roles
