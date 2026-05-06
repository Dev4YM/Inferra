from __future__ import annotations

from fastapi.testclient import TestClient

from config import InferraConfig, StorageConfig, config_to_dict
from web import create_app


def test_config_api_persists_experience_mode_runtime_and_file(tmp_path):
    config_path = tmp_path / "inferra.toml"
    config = InferraConfig(storage=StorageConfig(data_dir=tmp_path))
    app = create_app(config, config_path=config_path)

    with TestClient(app) as client:
        current = client.get("/api/config")
        assert current.status_code == 200
        payload = current.json()["config"]
        payload["experience"]["mode"] = "developer"
        payload["experience"]["show_raw_evidence_by_default"] = True

        response = client.put("/api/config", json={"config": payload})
        assert response.status_code == 200
        updated = response.json()
        assert updated["applied"] is True
        assert updated["config"]["experience"]["mode"] == "developer"
        assert updated["config"]["experience"]["show_raw_evidence_by_default"] is True

        reread = client.get("/api/config").json()["config"]
        assert reread["experience"]["mode"] == "developer"
        assert "mode = \"developer\"" in config_path.read_text(encoding="utf-8")


def test_config_api_rejects_runtime_storage_root_change(tmp_path):
    config = InferraConfig(storage=StorageConfig(data_dir=tmp_path))
    app = create_app(config)

    with TestClient(app) as client:
        payload = config_to_dict(config)
        payload["storage"]["data_dir"] = str(tmp_path / "other")
        response = client.put("/api/config", json={"config": payload})
        assert response.status_code == 409
