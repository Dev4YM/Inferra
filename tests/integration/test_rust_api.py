"""Integration tests against the Rust Axum runtime (not deprecated Python FastAPI)."""

from __future__ import annotations

import pytest

pytestmark = pytest.mark.integration


def test_rust_health_reports_storage(rust_runtime: dict) -> None:
    base_url = str(rust_runtime["base_url"])
    status, health = rust_runtime["fetch_json"](f"{base_url}/api/health")  # type: ignore[misc]
    assert status == 200
    assert health.get("runtime") == "rust"
    assert health.get("storage_writes_ok") is True
    assert health.get("status") == "ok"


def test_rust_probe_endpoints_are_minimal_without_api_auth(rust_runtime: dict) -> None:
    base_url = str(rust_runtime["base_url"])
    fetch_json = rust_runtime["fetch_json"]

    status, healthz = fetch_json(f"{base_url}/healthz")
    assert status == 200
    assert healthz == {"status": "ok", "runtime": "rust"}

    status, readyz = fetch_json(f"{base_url}/readyz")
    assert status == 200
    assert readyz.get("status") == "ready"
    assert readyz.get("runtime") == "rust"
    assert readyz.get("storage_writes_ok") is True
    assert "events_db" not in readyz
    assert "config_path" not in readyz


def test_rust_metrics_are_disabled_by_default(rust_runtime: dict) -> None:
    base_url = str(rust_runtime["base_url"])
    fetch_json = rust_runtime["fetch_json"]

    status, body = fetch_json(f"{base_url}/api/metrics")
    assert status == 404
    assert body.get("detail") == "metrics endpoint disabled"


def test_rust_api_auth_token_env_requires_bearer(rust_runtime_auth: dict) -> None:
    base_url = str(rust_runtime_auth["base_url"])
    fetch_json = rust_runtime_auth["fetch_json"]

    status, healthz = fetch_json(f"{base_url}/healthz")
    assert status == 200
    assert healthz.get("runtime") == "rust"

    status, denied = fetch_json(f"{base_url}/api/health")
    assert status == 401
    assert denied.get("detail") == "unauthorized"

    status, denied = fetch_json(f"{base_url}/api/health", headers={"Authorization": "Bearer wrong-token"})
    assert status == 401
    assert denied.get("detail") == "unauthorized"

    status, health = fetch_json(f"{base_url}/api/health", headers={"Authorization": "Bearer secret-token"})
    assert status == 200
    assert health.get("runtime") == "rust"


def test_rust_api_auth_token_env_fails_closed_when_unset(rust_runtime_auth_unset: dict) -> None:
    base_url = str(rust_runtime_auth_unset["base_url"])
    fetch_json = rust_runtime_auth_unset["fetch_json"]

    status, healthz = fetch_json(f"{base_url}/healthz")
    assert status == 200
    assert healthz.get("runtime") == "rust"

    status, denied = fetch_json(f"{base_url}/api/health")
    assert status == 503
    assert "auth_token_env" in denied.get("detail", "")


def test_rust_ingest_creates_incident(rust_runtime: dict) -> None:
    base_url = str(rust_runtime["base_url"])
    post_json = rust_runtime["post_json"]
    fetch_json = rust_runtime["fetch_json"]

    for message in ("timeout calling postgres", "connection refused from postgres"):
        status, body = post_json(
            f"{base_url}/api/ingest",
            {"service": "api", "level": "error", "message": message},
        )
        assert status == 200, body
        assert body.get("accepted") is True

    status, incidents_payload = fetch_json(f"{base_url}/api/incidents")
    assert status == 200
    incidents = incidents_payload.get("incidents", [])
    assert len(incidents) >= 1
    incident_id = incidents[0]["incident_id"]

    status, detail = fetch_json(f"{base_url}/api/incidents/{incident_id}")
    assert status == 200
    assert detail.get("hypotheses")


def test_rust_anomaly_status(rust_runtime: dict) -> None:
    base_url = str(rust_runtime["base_url"])
    post_json = rust_runtime["post_json"]
    fetch_json = rust_runtime["fetch_json"]

    for _ in range(3):
        post_json(
            f"{base_url}/api/ingest",
            {"service": "api", "level": "error", "message": "failure burst"},
        )

    status, body = fetch_json(f"{base_url}/api/anomaly/api/status")
    assert status == 200
    assert body.get("service_id") == "api"
    assert "buckets" in body


def test_rust_topology_edges_persist(rust_runtime: dict) -> None:
    base_url = str(rust_runtime["base_url"])
    post_json = rust_runtime["post_json"]
    fetch_json = rust_runtime["fetch_json"]

    status, created = post_json(
        f"{base_url}/api/topology/edges",
        {"source": "api", "target": "postgres", "relation_type": "depends_on"},
    )
    assert status == 200, created

    status, topology = fetch_json(f"{base_url}/api/topology")
    assert status == 200
    edges = topology.get("edges", [])
    assert {"source": "api", "target": "postgres"} in [
        {"source": e.get("source"), "target": e.get("target")} for e in edges
    ]


def test_rust_investigate_fallback_without_ai(rust_runtime: dict) -> None:
    base_url = str(rust_runtime["base_url"])
    post_json = rust_runtime["post_json"]
    fetch_json = rust_runtime["fetch_json"]

    post_json(
        f"{base_url}/api/ingest",
        {"service": "api", "level": "error", "message": "timeout calling postgres"},
    )
    post_json(
        f"{base_url}/api/ingest",
        {"service": "api", "level": "error", "message": "connection refused from postgres"},
    )

    status, payload = fetch_json(f"{base_url}/api/investigate/now?monitor_seconds=0")
    assert status == 200
    assert payload.get("used_ai") is False
    assert payload.get("fallback_reason")
    output = payload.get("output", {})
    assert output.get("risk_level") in {"low", "medium", "high", "critical"}
    for step in output.get("next_steps", []):
        assert step.get("safety") == "read_only"
        assert step.get("requires_user_action") is True


def test_rust_config_put_persists_experience(rust_runtime: dict) -> None:
    base_url = str(rust_runtime["base_url"])
    config_path = rust_runtime["config_path"]
    put_json = rust_runtime["put_json"]

    status, updated = put_json(
        f"{base_url}/api/config",
        {
            "config": {
                "experience": {
                    "mode": "developer",
                    "show_raw_evidence_by_default": True,
                }
            }
        },
    )
    assert status == 200, updated
    assert updated.get("applied") is True
    assert updated["config"]["experience"]["mode"] == "developer"

    text = config_path.read_text(encoding="utf-8")
    assert 'mode = "developer"' in text


def test_rust_ingest_requires_shared_token(rust_runtime: dict) -> None:
    base_url = str(rust_runtime["base_url"])
    put_json = rust_runtime["put_json"]
    post_json = rust_runtime["post_json"]

    status, updated = put_json(
        f"{base_url}/api/config",
        {"config": {"collectors": {"app": {"shared_token": "secret-token"}}}},
    )
    assert status == 200, updated

    status, denied = post_json(
        f"{base_url}/api/ingest",
        {"service": "api", "level": "error", "message": "unauthorized attempt"},
    )
    assert status == 401, denied

    status, accepted = post_json(
        f"{base_url}/api/ingest",
        {"service": "api", "level": "error", "message": "authorized attempt"},
        headers={"Authorization": "Bearer secret-token"},
    )
    assert status == 200, accepted
    assert accepted.get("accepted") is True
