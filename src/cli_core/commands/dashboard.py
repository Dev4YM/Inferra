"""Open or describe the local web control plane."""

from __future__ import annotations

import argparse
import webbrowser
from urllib.parse import urljoin

from cli_core.result import CommandResult

_PATHS = {
    "overview": "/",
    "incidents": "/incidents",
    "systems": "/systems",
    "evidence": "/evidence",
    "ai": "/ai",
    "workspace": "/workspace",
    "control": "/control",
    "settings": "/settings",
}


def dashboard_url(base_url: str, section: str) -> str:
    path = _PATHS.get(section, "/")
    return urljoin(base_url.rstrip("/") + "/", path.lstrip("/"))


async def handle_dashboard(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    config_path, config = cli._load_config_for_command(args)
    base_url = cli._server_url(config)
    section = str(getattr(args, "section", None) or "overview")
    url = dashboard_url(base_url, section)

    api_status = {"reachable": False}
    try:
        health = await cli._local_api_json(config, "GET", "/api/health")
        api_status = {
            "reachable": True,
            "degraded": bool(health.get("degraded")),
            "active_incidents": int(health.get("active_incidents", 0)),
            "queue_depth": int(health.get("queue_depth", 0)),
            "ai_available": bool(health.get("ai_available")),
        }
    except cli.CommandError as exc:
        api_status["reason"] = str(exc)

    should_open = not bool(getattr(args, "no_open", False))
    opened = False
    open_error: str | None = None
    if should_open:
        try:
            opened = bool(webbrowser.open(url))
        except Exception as exc:  # pragma: no cover - platform/browser dependent
            open_error = str(exc)

    payload = {
        "command": "dashboard",
        "config_path": str(config_path),
        "section": section,
        "url": url,
        "opened": opened,
        "api": api_status,
        "safe_next_steps": [
            f"inferra --config \"{config_path}\" serve" if not api_status["reachable"] else f"Open {url}",
            f"inferra --config \"{config_path}\" guide",
        ],
    }
    lines = [
        f"dashboard={url}",
        f"api_reachable={api_status['reachable']}",
    ]
    if opened:
        lines.append("opened_browser=true")
    elif should_open and open_error:
        lines.append(f"opened_browser=false ({open_error})")
    elif should_open:
        lines.append("opened_browser=false")
    if not api_status["reachable"]:
        lines.append("Next: " + payload["safe_next_steps"][0])
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))
