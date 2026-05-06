"""Tests for the workspace intelligence routes."""

from __future__ import annotations

from fastapi.testclient import TestClient

from config import InferraConfig, StorageConfig, WorkspaceConfig, WorkspaceServiceMapping
from web import create_app


def test_workspace_projects_endpoint_returns_list(tmp_path):
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    with TestClient(app) as client:
        response = client.get("/api/workspace/projects", params={"max_depth": 1, "max_results": 5})
        assert response.status_code == 200
        payload = response.json()
        assert "projects" in payload
        assert isinstance(payload["projects"], list)


def test_workspace_map_uses_explicit_user_mappings(tmp_path):
    project = tmp_path / "demo-app"
    project.mkdir()
    (project / "package.json").write_text("{}", encoding="utf-8")

    workspace = WorkspaceConfig(
        enabled=True,
        roots=[str(tmp_path)],
        max_depth=2,
        max_results=10,
        service_mappings=[
            WorkspaceServiceMapping(
                service_id="api",
                project_path=str(project),
                confidence=1.0,
                source="user",
                notes="explicit",
            )
        ],
    )
    config = InferraConfig(storage=StorageConfig(data_dir=tmp_path), workspace=workspace)
    app = create_app(config)
    with TestClient(app) as client:
        response = client.get("/api/workspace/map")
        assert response.status_code == 200
        payload = response.json()
        assert payload["enabled"] is True
        mappings = payload["service_mappings"]
        assert any(m["service_id"] == "api" and m["project_path"] == str(project) for m in mappings)


def test_workspace_inspect_reports_markers(tmp_path):
    project = tmp_path / "myapp"
    project.mkdir()
    (project / "Dockerfile").write_text("FROM python:3.13", encoding="utf-8")
    (project / "package.json").write_text("{}", encoding="utf-8")

    config = InferraConfig(storage=StorageConfig(data_dir=tmp_path))
    app = create_app(config)
    with TestClient(app) as client:
        response = client.get("/api/workspace/inspect", params={"path": str(project)})
        assert response.status_code == 200
        payload = response.json()
        assert payload["exists"] is True
        assert "Dockerfile" in payload["markers"]
        assert "package.json" in payload["markers"]
        assert payload["has_dockerfile"] is True


def test_workspace_add_mapping_persists_when_config_path_present(tmp_path):
    config_path = tmp_path / "inferra.toml"
    config = InferraConfig(storage=StorageConfig(data_dir=tmp_path))
    app = create_app(config, config_path=config_path)
    with TestClient(app) as client:
        response = client.post(
            "/api/workspace/mappings",
            json={"service_id": "api", "project_path": str(tmp_path), "confidence": 0.9},
        )
        assert response.status_code == 200
        body = response.json()
        assert body["stored"] is True
        assert body["service_id"] == "api"
        assert body["persisted"] is True
        text = config_path.read_text(encoding="utf-8")
        assert "service_id = \"api\"" in text
