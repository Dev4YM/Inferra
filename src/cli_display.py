"""Rich terminal rendering for the Inferra CLI (human mode only)."""

from __future__ import annotations

import json
from typing import Any

from rich.console import Console
from rich.panel import Panel
from rich.table import Table
from rich.theme import Theme

inferra_theme = Theme(
    {
        "info": "cyan",
        "warning": "yellow",
        "danger": "bold red",
        "ok": "bold green",
        "muted": "dim",
    }
)

console = Console(theme=inferra_theme)


def render_health_panel(payload: dict[str, Any]) -> None:
    degraded = bool(payload.get("degraded"))
    title = "[danger]Degraded[/]" if degraded else "[ok]Observing[/]"
    lines = [
        f"Events DB: {payload.get('events_db', '')}",
        f"Incidents DB: {payload.get('incidents_db', '')}",
        f"Active incidents: {payload.get('active_incidents', 0)}",
        f"Queue depth: {payload.get('queue_depth', 0)}",
        f"Collectors: {payload.get('collectors', 0)} (errors: {payload.get('collector_errors', 0)})",
        f"AI enabled: {payload.get('ai_enabled')} · available: {payload.get('ai_available')}",
    ]
    if payload.get("ai_reason"):
        lines.append(f"AI note: {payload.get('ai_reason')}")
    reasons = payload.get("degraded_reasons") or []
    if reasons:
        lines.append("Reasons: " + ", ".join(str(r) for r in reasons))
    console.print(Panel("\n".join(lines), title=title, border_style="cyan"))


def render_overview(payload: dict[str, Any]) -> None:
    qa = payload.get("quick_analysis") or {}
    console.print(
        Panel(
            str(qa.get("headline", "")),
            title="[bold]Quick analysis[/]",
            subtitle=f"risk={qa.get('risk_level','?')}",
            border_style="green" if qa.get("risk_level") != "high" else "red",
        )
    )
    rt = payload.get("runtime") or {}
    containers = rt.get("containers") or []
    table = Table(title="Docker containers (docker ps)")
    table.add_column("Name")
    table.add_column("Image")
    table.add_column("State")
    for c in containers[:40]:
        table.add_row(str(c.get("name", "")), str(c.get("image", "")), str(c.get("state", "")))
    if not containers:
        table.add_row("—", "none detected", "")
    console.print(table)

    proj = payload.get("workspace_projects") or []
    pt = Table(title="Detected code projects")
    pt.add_column("Kind")
    pt.add_column("Path")
    for p in proj[:30]:
        pt.add_row(str(p.get("kind", "")), str(p.get("path", "")))
    if not proj:
        pt.add_row("—", "none in scan budget")
    console.print(pt)


def render_workspace_local(rows: list[dict[str, str]]) -> None:
    table = Table(title="Local code projects (marker scan)")
    table.add_column("Kind")
    table.add_column("Path")
    for row in rows:
        table.add_row(row.get("kind", ""), row.get("path", ""))
    console.print(table)


def render_offline_notice(snapshot: dict[str, Any]) -> None:
    console.print("[warning]Inferra HTTP API unreachable — host snapshot only[/]")
    text = json.dumps(snapshot, indent=2, sort_keys=True)
    if len(text) > 12000:
        text = text[:12000] + "\n…"
    console.print(Panel(text, title="Runtime snapshot", border_style="yellow"))
