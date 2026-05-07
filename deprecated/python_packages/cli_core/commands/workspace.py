"""Workspace intelligence commands: scan, map, services, inspect."""

from __future__ import annotations

import argparse
from typing import Any

from cli_core.result import CommandResult


async def handle_workspace_scan(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from inferra_legacy.cli_display import render_workspace_local
    from runtime.workspace_scan import discover_code_projects, projects_to_json

    config_path, _config = cli._load_config_for_command(args)
    hits = discover_code_projects(max_depth=4, max_results=80)
    rows = projects_to_json(hits)
    payload: dict[str, Any] = {"command": "workspace", "config_path": str(config_path), "projects": rows}

    def hook() -> None:
        render_workspace_local(rows)

    use_rich = not getattr(args, "json", False)
    return cli._emit_result(
        args,
        CommandResult(
            payload=payload,
            stdout_lines=[f"Found {len(rows)} projects"],
            rich_hook=hook if use_rich else None,
        ),
    )


async def handle_workspace_map(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    payload = await cli._require_local_api(config, "GET", "/api/workspace/map")
    payload["command"] = "workspace map"
    payload["config_path"] = str(config_path)
    mappings = list(payload.get("service_mappings") or [])
    lines = [
        f"Discovered projects: {len(payload.get('projects') or [])}",
        f"Service mappings: {len(mappings)}",
    ]
    for mapping in mappings[:50]:
        signals = ", ".join(s.get("name", "") for s in mapping.get("signals") or [])
        lines.append(
            f"  {mapping.get('service_id')} -> {mapping.get('project_path')} "
            f"confidence={mapping.get('confidence')} source={mapping.get('source')} signals={signals or '-'}"
        )
    unmapped = list(payload.get("unmapped_services") or [])
    if unmapped:
        lines.append(f"Unmapped services: {', '.join(unmapped)}")
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))


async def handle_workspace_services(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    payload = await cli._require_local_api(config, "GET", "/api/workspace/services")
    payload["command"] = "workspace services"
    payload["config_path"] = str(config_path)
    mappings = list(payload.get("service_mappings") or [])
    lines = [f"Service mappings: {len(mappings)}"]
    for mapping in mappings[:50]:
        lines.append(
            f"  {mapping.get('service_id')} -> {mapping.get('project_path')} "
            f"confidence={mapping.get('confidence')} source={mapping.get('source')}"
        )
    unmapped = list(payload.get("unmapped_services") or [])
    if unmapped:
        lines.append(f"Unmapped services: {', '.join(unmapped)}")
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))


async def handle_workspace_inspect(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from runtime.workspace_map import inspect_project

    config_path, _config = cli._load_config_for_command(args)
    payload = inspect_project(str(args.path))
    payload["command"] = "workspace inspect"
    payload["config_path"] = str(config_path)
    if not payload.get("exists"):
        return cli._emit_result(
            args,
            CommandResult(
                payload=payload,
                stdout_lines=[f"Project path not found: {args.path}"],
                exit_code=1,
            ),
        )
    markers = list(payload.get("markers") or [])
    lines = [f"path={payload['path']}", f"markers: {', '.join(markers) if markers else '-'}"]
    if payload.get("has_compose"):
        lines.append("compose: yes")
    if payload.get("has_dockerfile"):
        lines.append("dockerfile: yes")
    if payload.get("has_env_file"):
        lines.append("env file: yes (values redacted; metadata only)")
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))
