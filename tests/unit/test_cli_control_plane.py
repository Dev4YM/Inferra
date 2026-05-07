from __future__ import annotations

import json

import inferra_legacy.cli as cli
import cli_core.commands.dashboard as dashboard_cmds
from cli_core.commands.service import build_release_readiness
from inferra_legacy.cli import CommandError, main


def test_incidents_list_json_uses_live_api(monkeypatch, tmp_path, capsys) -> None:
    async def fake_local_api_json(config, method, path, payload=None):
        assert method == "GET"
        assert path == "/api/incidents"
        return {
            "incidents": [
                {
                    "incident_id": "inc-1",
                    "state": "open",
                    "severity": 3,
                    "primary_service": "api",
                    "events": ["evt-1"],
                }
            ]
        }

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    result = main(["--json", "--config", str(tmp_path / "inferra.toml"), "incidents", "list"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["command"] == "incidents list"
    assert payload["incidents"][0]["incident_id"] == "inc-1"


def test_events_list_applies_limit(monkeypatch, tmp_path, capsys) -> None:
    async def fake_local_api_json(config, method, path, payload=None):
        assert path == "/api/events?limit=5"
        return {"events": [{"event_id": "evt-1", "severity": 2, "service_id": "api", "message": "warn"}]}

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    result = main(["--json", "--config", str(tmp_path / "inferra.toml"), "events", "list", "--limit", "5"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["limit"] == 5
    assert payload["events"][0]["event_id"] == "evt-1"


def test_investigate_latest_prioritizes_highest_severity_incident(monkeypatch, tmp_path, capsys) -> None:
    async def fake_local_api_json(config, method, path, payload=None):
        if path == "/api/incidents":
            return {
                "incidents": [
                    {"incident_id": "inc-low", "severity": 1, "updated_at": "2026-01-01T00:00:00Z"},
                    {"incident_id": "inc-high", "severity": 4, "updated_at": "2026-01-01T00:00:01Z"},
                ]
            }
        if path == "/api/incidents/inc-high":
            return {
                "incident": {
                    "incident_id": "inc-high",
                    "state": "open",
                    "severity": 4,
                    "primary_service": "api",
                },
                "events": [{"event_id": "evt-1", "severity": 4, "service_id": "api", "message": "boom"}],
                "hypotheses": [{"cause_type": "dependency_failure", "total_score": 0.91, "confidence": "high"}],
                "clusters": [],
            }
        raise AssertionError(path)

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    result = main(["--json", "--config", str(tmp_path / "inferra.toml"), "investigate", "latest"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["command"] == "investigate incident"
    assert payload["focus"] == "inc-high"
    assert payload["priority"] == "high"
    assert "safe_next_steps" in payload


def test_doctor_reports_api_unreachable_without_failing_config(monkeypatch, tmp_path, capsys) -> None:
    async def fake_local_api_json(config, method, path, payload=None):
        raise CommandError("No running Inferra supervisor found.")

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    result = main(["--json", "--config", str(tmp_path / "inferra.toml"), "doctor"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["ok"] is True
    assert payload["api"]["reachable"] is False
    assert any("inferra serve" in step for step in payload["safe_next_steps"])


def test_release_readiness_detects_required_and_forbidden_paths(tmp_path) -> None:
    for rel in (
        "README.md",
        "mkdocs.yml",
        "docs/index.md",
        "docs/dossier/README.md",
        "src/web/frontend/package.json",
        "src/web/frontend/package-lock.json",
        "scripts/build-web.ps1",
        "scripts/build-web.sh",
        "src/web/ui_dist/index.html",
        "src/cli_core/commands",
    ):
        path = tmp_path / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        if "." in path.name:
            path.write_text("runtime intelligence control plane operator developer", encoding="utf-8")
        else:
            path.mkdir(exist_ok=True)

    clean = build_release_readiness(tmp_path)
    assert clean["ok"] is True

    (tmp_path / "webui").mkdir()
    dirty = build_release_readiness(tmp_path)
    assert dirty["ok"] is False
    assert any(item["name"] == "Duplicate top-level webui" and not item["ok"] for item in dirty["checks"])


def test_guide_json_returns_profiled_next_steps(monkeypatch, tmp_path, capsys) -> None:
    async def fake_local_api_json(config, method, path, payload=None):
        raise CommandError("No running Inferra supervisor found.")

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    result = main(["--json", "--config", str(tmp_path / "inferra.toml"), "guide", "--profile", "developer"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["command"] == "guide"
    assert payload["profile"] == "developer"
    assert payload["config_exists"] is False
    assert payload["safety_boundary"]["ai_executes_commands"] is False
    assert any("onboard" in step["command"] for step in payload["steps"])
    assert any("mode set developer" in step["command"] for step in payload["steps"])


def test_guide_prefers_investigation_when_api_has_incidents(monkeypatch, tmp_path, capsys) -> None:
    async def fake_local_api_json(config, method, path, payload=None):
        if path == "/api/health":
            return {"degraded": False, "active_incidents": 1, "queue_depth": 0, "ai_available": True}
        if path == "/api/overview":
            return {"quick_analysis": {"headline": "One incident needs attention."}}
        raise AssertionError(path)

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    result = main(["--json", "--config", str(tmp_path / "inferra.toml"), "guide"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["live"]["reachable"] is True
    assert payload["overview_headline"] == "One incident needs attention."
    assert any("investigate latest" in step["command"] for step in payload["steps"])


def test_dashboard_no_open_reports_url_and_unreachable_api(monkeypatch, tmp_path, capsys) -> None:
    async def fake_local_api_json(config, method, path, payload=None):
        raise CommandError("No running Inferra supervisor found.")

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    result = main(["--json", "--config", str(tmp_path / "inferra.toml"), "dashboard", "--section", "ai", "--no-open"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["command"] == "dashboard"
    assert payload["url"].endswith("/ai")
    assert payload["opened"] is False
    assert payload["api"]["reachable"] is False
    assert any("serve" in step for step in payload["safe_next_steps"])


def test_dashboard_opens_browser_when_requested(monkeypatch, tmp_path, capsys) -> None:
    async def fake_local_api_json(config, method, path, payload=None):
        assert path == "/api/health"
        return {"degraded": False, "active_incidents": 0, "queue_depth": 0, "ai_available": False}

    opened: list[str] = []

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)
    monkeypatch.setattr(dashboard_cmds.webbrowser, "open", lambda url: opened.append(url) or True)

    result = main(["--json", "--config", str(tmp_path / "inferra.toml"), "dashboard", "--section", "workspace"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["api"]["reachable"] is True
    assert payload["opened"] is True
    assert opened and opened[0].endswith("/workspace")


def test_doctor_release_includes_repo_readiness(monkeypatch, tmp_path, capsys) -> None:
    async def fake_local_api_json(config, method, path, payload=None):
        raise CommandError("No running Inferra supervisor found.")

    monkeypatch.setattr(cli, "_local_api_json", fake_local_api_json)

    import cli_core.commands.service as service_cmds

    monkeypatch.setattr(
        service_cmds,
        "build_release_readiness",
        lambda: {"ok": True, "checks": [], "warnings": [], "root": str(tmp_path)},
    )

    result = main(["--json", "--config", str(tmp_path / "inferra.toml"), "doctor", "--release"])
    payload = json.loads(capsys.readouterr().out)

    assert result == 0
    assert payload["release"]["ok"] is True
