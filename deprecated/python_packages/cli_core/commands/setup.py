"""Onboarding command: writes config, migrates databases, probes Ollama."""

from __future__ import annotations

import argparse
import sys
from dataclasses import replace
from pathlib import Path
from typing import Any

from cli_core.result import CommandError, CommandResult


async def handle_setup(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from ai import AIService
    from config import config_to_dict, load_config, write_config
    from storage.migrations import migrate

    config_path = cli._config_path(args)
    config_exists = config_path.exists()
    current = load_config(config_path)
    config = replace(current, ai=replace(current.ai, enabled=True, provider="ollama"))
    config = cli._apply_setup_overrides(config, args)

    skip_connection_test = bool(args.skip_connection_test)
    if not args.yes:
        if not sys.stdin.isatty():
            raise CommandError(
                "Interactive setup requires a TTY. Re-run with `--yes` for non-interactive setup."
            )
        config, skip_connection_test = cli._interactive_setup(
            config, config_path, config_exists, skip_connection_test
        )

    data_dir = Path(config.storage.data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)
    events_path = data_dir / config.storage.events_db
    incidents_path = data_dir / config.storage.incidents_db
    events_version = migrate(events_path)
    incidents_version = migrate(incidents_path)

    current_data = config_to_dict(current)
    updated_data = config_to_dict(config)
    wrote_config = not config_exists or current_data != updated_data
    if wrote_config:
        write_config(config, config_path)

    connection_test: dict[str, Any]
    exit_code = 0
    stderr_lines: list[str] = []
    if not config.ai.enabled:
        connection_test = {"skipped": True, "reason": "AI is disabled in config."}
    elif skip_connection_test:
        connection_test = {"skipped": True}
    else:
        probe = await AIService(config).status()
        connection_test = {
            "skipped": False,
            "available": probe.get("available", False),
            "reason": probe.get("reason"),
            "error": probe.get("error"),
            "installed": probe.get("installed", False),
            "resolved_model": probe.get("resolved_model"),
            "version": probe.get("version"),
        }
        if not probe.get("available", False):
            exit_code = 1
            stderr_lines.append(
                f"Ollama probe failed at {config.ai.base_url}: "
                f"{probe.get('error') or probe.get('reason') or 'unknown error'}"
            )

    payload = {
        "command": "setup",
        "config_path": str(config_path),
        "created_config": not config_exists,
        "wrote_config": wrote_config,
        "data_dir": str(data_dir),
        "events_db": {"path": str(events_path), "schema_version": events_version},
        "incidents_db": {"path": str(incidents_path), "schema_version": incidents_version},
        "ai": {
            "enabled": config.ai.enabled,
            "provider": config.ai.provider,
            "model": config.ai.model,
            "base_url": config.ai.base_url,
            "allow_remote": config.ai.allow_remote,
            "token_env": config.ai.token_env,
        },
        "experience": cli._experience_payload(config),
        "preset": getattr(args, "preset", None),
        "connection_test": connection_test,
        "next_steps": cli._onboarding_next_steps(config_path, config, connection_test),
    }
    stdout_lines = [
        f"{'Wrote' if wrote_config else 'Validated'} config at {config_path}",
        f"Initialized storage under {data_dir}",
        f"{events_path.name}: schema version {events_version}",
        f"{incidents_path.name}: schema version {incidents_version}",
        f"Control-plane mode: {config.experience.mode} (AI role: {config.experience.ai_role})",
        "AI disabled in config." if not config.ai.enabled else (
            "Skipped Ollama connection test"
            if skip_connection_test
            else cli._human_connection_line(connection_test, config.ai.base_url)
        ),
    ]
    if getattr(args, "preset", None):
        stdout_lines.append(f"Applied preset {args.preset}")
    stdout_lines.extend(f"Next: {step}" for step in payload["next_steps"][:4])
    return cli._emit_result(
        args,
        CommandResult(
            payload=payload,
            stdout_lines=stdout_lines,
            stderr_lines=stderr_lines,
            exit_code=exit_code,
        ),
    )
