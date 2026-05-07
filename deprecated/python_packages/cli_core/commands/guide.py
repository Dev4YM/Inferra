"""First-run and daily-use guidance for humans.

This command is intentionally read-only. It turns the current local state into
an ordered operating path so a new user does not have to infer the product flow
from scattered commands.
"""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Any

from cli_core.result import CommandError, CommandResult

_PROFILES = ("operator", "expert", "developer", "server", "contributor")


def _quote(path: Path) -> str:
    return f'"{path}"'


def _step(title: str, command: str, reason: str, *, safety: str = "read_only") -> dict[str, str]:
    return {"title": title, "command": command, "reason": reason, "safety": safety}


async def handle_guide(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    profile = str(getattr(args, "profile", None) or config.experience.mode or "operator")
    if profile not in _PROFILES:
        raise CommandError(f"Unknown guide profile {profile!r}. Use one of: {', '.join(_PROFILES)}")

    live: dict[str, Any] = {"reachable": False}
    overview: dict[str, Any] | None = None
    try:
        health = await cli._local_api_json(config, "GET", "/api/health")
        overview = await cli._local_api_json(config, "GET", "/api/overview")
        live = {
            "reachable": True,
            "degraded": bool(health.get("degraded")),
            "active_incidents": int(health.get("active_incidents", 0)),
            "queue_depth": int(health.get("queue_depth", 0)),
            "ai_available": bool(health.get("ai_available")),
        }
    except CommandError as exc:
        live["reason"] = str(exc)

    config_exists = config_path.exists()
    quoted_config = _quote(config_path)
    steps: list[dict[str, str]] = []

    if not config_exists:
        steps.append(
            _step(
                "Create the local control-plane config",
                f"inferra --config {quoted_config} onboard --yes --mode operator --preset windows-server --skip-connection-test",
                "Writes inferra.toml and initializes local SQLite storage without requiring AI to be reachable.",
                safety="writes_inferra_config_only",
            )
        )
    else:
        steps.append(
            _step(
                "Validate local configuration",
                f"inferra --config {quoted_config} check-config",
                "Catches config, storage, AI, and topology issues before starting the control plane.",
            )
        )

    if not config.ai.enabled:
        steps.append(
            _step(
                "Enable AI when ready",
                f"inferra --config {quoted_config} ai setup --enable --model {config.ai.model} --skip-connection-test",
                "AI is optional, but Inferra is designed around investigation guidance once a model is available.",
                safety="writes_inferra_config_only",
            )
        )
    else:
        steps.append(
            _step(
                "Check AI readiness",
                f"inferra --config {quoted_config} ai doctor",
                "Shows provider availability, remote-provider risk, and redaction policy.",
            )
        )

    if not live["reachable"]:
        steps.append(
            _step(
                "Start the control plane",
                f"inferra --config {quoted_config} serve",
                "Starts the API and web dashboard at the configured local address.",
                safety="starts_inferra_only",
            )
        )
    else:
        steps.append(
            _step(
                "Open the dashboard",
                f"inferra --config {quoted_config} dashboard",
                "Opens the local web control plane and verifies the API health first.",
            )
        )
        if live.get("active_incidents", 0):
            steps.append(
                _step(
                    "Investigate the top incident",
                    f"inferra --config {quoted_config} investigate latest",
                    "Uses the current evidence bundle to prioritize what to inspect next.",
                )
            )
        else:
            steps.append(
                _step(
                    "Open the operating picture",
                    f"inferra --config {quoted_config} overview",
                    "Shows incidents, service health, AI state, and workspace context from the running API.",
                )
            )

    if profile in {"developer", "contributor"}:
        steps.extend(
            [
                _step(
                    "Switch to developer mode",
                    f"inferra --config {quoted_config} mode set developer",
                    "Exposes raw evidence, prompt traces, mapping confidence, and diagnostics.",
                    safety="writes_inferra_config_only",
                ),
                _step(
                    "Inspect workspace mappings",
                    f"inferra --config {quoted_config} workspace map",
                    "Connects runtime services to local code projects with confidence signals.",
                ),
            ]
        )

    if profile == "server":
        steps.extend(
            [
                _step(
                    "Validate service prerequisites",
                    f"inferra --config {quoted_config} service repair",
                    "Checks config path, data dir, bind host, log path, and Windows service dependencies.",
                ),
                _step(
                    "Install Inferra as a service",
                    f"inferra --config {quoted_config} service install --startup auto",
                    "Installs the Inferra service only; it does not mutate observed systems.",
                    safety="installs_inferra_service_only",
                ),
            ]
        )

    if profile == "contributor":
        steps.extend(
            [
                _step(
                    "Run release readiness",
                    f"inferra --config {quoted_config} doctor --release",
                    "Checks docs, packaged UI, repo hygiene, and generated-artifact staging.",
                ),
                _step(
                    "Run tests",
                    "python -m pytest -q",
                    "Confirms the deterministic engine, CLI, API, and UI integration coverage still pass.",
                ),
            ]
        )

    dashboard_url = f"http://{config.server.host}:{config.server.port}"
    payload = {
        "command": "guide",
        "profile": profile,
        "config_path": str(config_path),
        "config_exists": config_exists,
        "dashboard_url": dashboard_url,
        "mode": config.experience.mode,
        "ai_role": config.experience.ai_role,
        "ai_enabled": config.ai.enabled,
        "live": live,
        "overview_headline": (overview or {}).get("quick_analysis", {}).get("headline"),
        "steps": steps,
        "safety_boundary": {
            "observed_system_mutation": False,
            "ai_executes_commands": False,
            "safe_actions_are_suggestions": True,
        },
    }

    lines = [
        f"Inferra guide ({profile})",
        f"config={config_path} exists={config_exists}",
        f"dashboard={dashboard_url}",
        f"mode={config.experience.mode} ai_role={config.experience.ai_role} ai_enabled={config.ai.enabled}",
        f"api_reachable={live['reachable']}",
    ]
    if payload["overview_headline"]:
        lines.append(str(payload["overview_headline"]))
    lines.append("Recommended path:")
    for idx, item in enumerate(steps, start=1):
        lines.append(f"{idx}. {item['title']}: {item['command']}")
        lines.append(f"   {item['reason']}")
    lines.append("Safety: AI suggests read-only checks and never executes remediation.")
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))
