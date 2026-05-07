"""AI provider commands: status, setup, models, pull, test."""

from __future__ import annotations

import argparse
import os
from dataclasses import replace

from cli_core.result import CommandError, CommandResult


async def handle_ai_status(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from ai import AIService

    config_path, config = cli._load_config_for_command(args)
    payload = await AIService(config).status()
    payload["command"] = "ai status"
    payload["config_path"] = str(config_path)
    stdout_lines = [
        f"AI enabled={payload['enabled']} provider={payload['provider']} model={payload['model']}",
        f"available={payload.get('available', False)} installed={payload.get('installed', False)} base_url={payload['base_url']}",
    ]
    if payload.get("resolved_model"):
        stdout_lines.append(f"resolved_model={payload['resolved_model']}")
    if payload.get("reason"):
        stdout_lines.append(f"reason={payload['reason']}")
    exit_code = 0 if payload.get("available") or not payload.get("enabled") else 1
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=stdout_lines, exit_code=exit_code),
    )


async def handle_ai_setup(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from ai import AIService
    from config import config_to_dict, load_config, write_config

    config_path = cli._config_path(args)
    current = load_config(config_path)
    updated = cli._apply_setup_overrides(replace(current, ai=replace(current.ai, provider="ollama")), args)
    current_data = config_to_dict(current)
    updated_data = config_to_dict(updated)
    wrote_config = current_data != updated_data
    if wrote_config:
        write_config(updated, config_path)

    connection_test: dict
    stderr_lines: list[str] = []
    exit_code = 0
    if not updated.ai.enabled:
        connection_test = {"skipped": True, "reason": "AI is disabled in config."}
    elif args.skip_connection_test:
        connection_test = {"skipped": True}
    else:
        probe = await AIService(updated).status()
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
                f"Ollama probe failed at {updated.ai.base_url}: "
                f"{probe.get('error') or probe.get('reason') or 'unknown error'}"
            )

    payload = {
        "command": "ai setup",
        "config_path": str(config_path),
        "wrote_config": wrote_config,
        "ai": {
            "enabled": updated.ai.enabled,
            "provider": updated.ai.provider,
            "model": updated.ai.model,
            "base_url": updated.ai.base_url,
            "allow_remote": updated.ai.allow_remote,
            "token_env": updated.ai.token_env,
        },
        "connection_test": connection_test,
        "next_steps": cli._onboarding_next_steps(config_path, updated, connection_test),
    }
    stdout_lines = [
        f"{'Updated' if wrote_config else 'Validated'} AI config at {config_path}",
        "AI disabled in config." if not updated.ai.enabled else (
            "Skipped Ollama connection test"
            if connection_test.get("skipped")
            else cli._human_connection_line(connection_test, updated.ai.base_url)
        ),
    ]
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


async def handle_ai_models(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from ai import AIService

    config_path, config = cli._load_config_for_command(args)
    service = AIService(config)
    installed: list[str] = []
    error: str | None = None
    if config.ai.enabled:
        try:
            installed = await service.installed_models()
        except Exception as exc:
            error = str(exc)
    registry = service.registry()
    installed_set = set(installed)
    payload = {
        "command": "ai models",
        "config_path": str(config_path),
        "installed": installed,
        "error": error,
        "registry": [
            {
                **model,
                "installed": bool(
                    model["name"] in installed_set
                    or (model.get("resolves_to") in installed_set)
                ),
            }
            for model in registry
        ],
    }
    stdout_lines = [
        f"{model['name']:<34} {model['size']:<7} {model['context_window']:<5} {model['quantization']:<8} "
        f"{'installed' if model['installed'] else 'available'}"
        for model in payload["registry"]
    ]
    stderr_lines = [f"Ollama unavailable: {error}"] if error else []
    return cli._emit_result(
        args,
        CommandResult(
            payload=payload,
            stdout_lines=stdout_lines,
            stderr_lines=stderr_lines,
            exit_code=1 if error else 0,
        ),
    )


async def handle_ai_pull(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from ai import AIService

    config_path, config = cli._load_config_for_command(args)
    service = AIService(config)
    model = (args.model or config.ai.model).strip()
    if not model:
        raise CommandError("Model tag is required.")
    if args.json:
        await service.pull_model(model)
        payload = {
            "command": "ai pull",
            "config_path": str(config_path),
            "model": model,
            "complete": True,
        }
        return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=[]))

    if os.environ.get("INFERRA_PLAIN") == "1":
        async for progress in service.pull_model_stream(model):
            percent = f"{progress.percent:5.1f}%" if progress.percent is not None else "  ... "
            status = progress.status or "pulling"
            digest = f" {progress.digest[:12]}" if progress.digest else ""
            print(f"\r{model} {percent} {status}{digest}", end="", flush=True)
        print(f"\r{model} 100.0% complete{' ' * 24}")
        return 0

    from rich.progress import Progress, SpinnerColumn, TextColumn, TimeElapsedColumn

    with Progress(
        SpinnerColumn(),
        TextColumn("{task.description}"),
        TimeElapsedColumn(),
        transient=False,
    ) as prog:
        task_id = prog.add_task(f"{model} starting…", total=None)
        async for chunk in service.pull_model_stream(model):
            pct = f"{chunk.percent:.1f}%" if chunk.percent is not None else "…"
            tail = chunk.status or "pulling"
            if chunk.digest:
                tail = f"{tail} {chunk.digest[:12]}"
            prog.update(task_id, description=f"{model} {pct} {tail}")
    return 0


async def handle_ai_test(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from ai import AIService

    config_path, config = cli._load_config_for_command(args)
    if not config.ai.enabled:
        payload = {
            "command": "ai test",
            "config_path": str(config_path),
            "enabled": False,
            "ok": False,
            "reason": "AI is disabled in config.",
        }
        return cli._emit_result(
            args,
            CommandResult(
                payload=payload,
                stdout_lines=["AI is disabled in config."],
                exit_code=1,
            ),
        )
    response = await AIService(config).test()
    payload = {
        "command": "ai test",
        "config_path": str(config_path),
        "enabled": True,
        "ok": True,
        "model": config.ai.model,
        "response": response,
    }
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=[response]))
