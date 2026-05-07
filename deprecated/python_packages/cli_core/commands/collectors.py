"""Collector commands: status, start, stop."""

from __future__ import annotations

import argparse

from cli_core.result import CommandError, CommandResult


async def handle_collectors_status(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    server_url = cli._server_url(config)
    live_payload: dict | None = None
    live_error: str | None = None
    try:
        live_payload = await cli._local_api_json(config, "GET", "/api/collectors")
    except CommandError as exc:
        live_error = str(exc)

    if live_payload is not None:
        payload = {
            "command": "collectors status",
            "mode": "live",
            "config_path": str(config_path),
            "server_url": server_url,
            "running": True,
            "queue_depth": int(live_payload.get("queue_depth", 0)),
            "collectors": list(live_payload.get("collectors", [])),
        }
        stdout_lines = [f"Live collectors: {len(payload['collectors'])} ({server_url})"]
        stdout_lines.extend(cli._format_collector_line(item) for item in payload["collectors"])
        return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))

    payload = {
        "command": "collectors status",
        "mode": "configured",
        "config_path": str(config_path),
        "server_url": server_url,
        "running": False,
        "queue_depth": 0,
        "collectors": cli._configured_collectors(config),
        "hint": "Start the live supervisor with `inferra run`.",
    }
    stdout_lines = [f"Configured collectors: {len(payload['collectors'])}"]
    stdout_lines.extend(cli._format_collector_line(item) for item in payload["collectors"])
    stderr_lines = (
        [f"No running Inferra supervisor found at {server_url}. Start it with `inferra run`."]
        if live_error
        else []
    )
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=stdout_lines, stderr_lines=stderr_lines),
    )


async def handle_collectors_start(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    payload = await cli._require_local_api(config, "POST", "/api/collectors/start")
    payload = {
        "command": "collectors start",
        "config_path": str(config_path),
        "server_url": cli._server_url(config),
        **payload,
    }
    stdout_lines = [f"Started collectors through {cli._server_url(config)}"]
    stdout_lines.extend(cli._format_collector_line(item) for item in payload.get("collectors", []))
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def handle_collectors_stop(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    payload = await cli._require_local_api(config, "POST", "/api/collectors/stop")
    payload = {
        "command": "collectors stop",
        "config_path": str(config_path),
        "server_url": cli._server_url(config),
        **payload,
    }
    stdout_lines = [f"Stopped collectors through {cli._server_url(config)}"]
    stdout_lines.extend(cli._format_collector_line(item) for item in payload.get("collectors", []))
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))
