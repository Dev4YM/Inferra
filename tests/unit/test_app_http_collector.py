import asyncio

from fastapi import FastAPI
from fastapi.testclient import TestClient

from collectors.app_http import AppHttpCollector


def test_app_http_collector_router_enqueues_payload():
    collector = AppHttpCollector(shared_token="secret")
    queue = asyncio.Queue()
    collector.attach_queue(queue)
    app = FastAPI()
    app.include_router(collector.router())

    with TestClient(app) as client:
        response = client.post(
            "/api/ingest",
            json={"service": "api", "level": "error", "message": "timeout calling db"},
            headers={"Authorization": "Bearer secret"},
        )

    assert response.status_code == 200
    assert response.json()["stored"] is True
    raw = queue.get_nowait()
    assert raw.source_type == "app"
    assert raw.metadata["service_id"] == "api"


def test_app_http_collector_rejects_large_payload():
    collector = AppHttpCollector(max_payload_bytes=20)
    queue = asyncio.Queue()
    collector.attach_queue(queue)
    app = FastAPI()
    app.include_router(collector.router())

    with TestClient(app) as client:
        response = client.post("/api/ingest", json={"service": "api", "message": "x" * 200})

    assert response.status_code == 413
