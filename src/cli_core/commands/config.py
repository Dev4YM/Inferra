"""Configuration commands: show, get, set, preset, mode show/set."""

from __future__ import annotations

import argparse
from dataclasses import replace

from cli_core.result import CommandError, CommandResult


async def handle_check_config(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    config_path, config = cli._load_config_for_command(args)
    report = await cli._build_check_report(config_path, config=config)
    stdout_lines: list[str] = []
    stderr_lines: list[str] = []
    if report["errors"]:
        stderr_lines.extend(f"Error: {error['message']}" for error in report["errors"])
    else:
        stdout_lines.append(f"Inferra config OK: {report['config_path']}")
        stdout_lines.extend(f"{key}={value}" for key, value in report["summary"].items())
    stderr_lines.extend(f"Warning: {warning['message']}" for warning in report["warnings"])
    return cli._emit_result(
        args,
        CommandResult(
            payload=report,
            stdout_lines=stdout_lines,
            stderr_lines=stderr_lines,
            exit_code=0 if report["ok"] else 1,
        ),
    )


async def handle_config_show(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    from config import config_to_dict, dump_config

    config_path, config = cli._load_config_for_command(args)
    payload = {
        "command": "config show",
        "config_path": str(config_path),
        "config": config_to_dict(config),
    }
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=[dump_config(config).rstrip("\n")]),
    )


async def handle_config_get(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    from config import get_config_value

    config_path, config = cli._load_config_for_command(args)
    value = get_config_value(config, args.key)
    payload = {
        "command": "config get",
        "config_path": str(config_path),
        "key": args.key,
        "value": cli._json_ready(value),
    }
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=[cli._format_config_value(value)]),
    )


async def handle_config_set(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    from config import get_config_value, set_config_value

    config_path, config = cli._load_config_for_command(args)
    previous = get_config_value(config, args.key)
    updated = set_config_value(config_path, args.key, args.value)
    new_value = get_config_value(updated, args.key)
    payload = {
        "command": "config set",
        "config_path": str(config_path),
        "key": args.key,
        "previous": cli._json_ready(previous),
        "value": cli._json_ready(new_value),
    }
    return cli._emit_result(
        args,
        CommandResult(
            payload=payload,
            stdout_lines=[f"Updated {args.key}={cli._format_config_value(new_value)}"],
        ),
    )


async def handle_config_preset(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    from config import apply_preset, config_to_dict, write_config

    config_path, config = cli._load_config_for_command(args)
    try:
        updated = apply_preset(config, args.name)
    except ValueError as exc:
        raise CommandError(str(exc)) from exc
    write_config(updated, config_path)
    payload = {
        "command": "config preset",
        "config_path": str(config_path),
        "preset": args.name,
        "collectors": config_to_dict(updated)["collectors"],
    }
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=[f"Applied preset {args.name}"]),
    )


async def handle_mode_show(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    config_path, config = cli._load_config_for_command(args)
    payload = {
        "command": "mode show",
        "config_path": str(config_path),
        "experience": cli._experience_payload(config),
    }
    exp = payload["experience"]
    lines = [
        f"mode={exp['mode']}",
        f"ai_role={exp['ai_role']}",
        f"safe_actions=suggest:{exp['suggest_safe_actions']} execute:{exp['execute_actions']}",
        f"raw_evidence_default={exp['show_raw_evidence_by_default']}",
    ]
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))


async def handle_mode_set(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    from config import write_config

    config_path, config = cli._load_config_for_command(args)
    show_raw = args.value == "developer"
    updated = replace(
        config,
        experience=replace(
            config.experience,
            mode=args.value,
            show_raw_evidence_by_default=show_raw,
        ),
    )
    write_config(updated, config_path)
    payload = {
        "command": "mode set",
        "config_path": str(config_path),
        "experience": cli._experience_payload(updated),
    }
    mode_descriptions = {
        "operator": "Operator mode keeps the default view concise.",
        "developer": "Developer mode exposes raw detail and connects runtime to local workspace context.",
    }
    lines = [
        f"Set control-plane mode to {updated.experience.mode}",
        mode_descriptions.get(updated.experience.mode, ""),
    ]
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))
