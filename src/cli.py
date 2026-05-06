# PYTHON_ARGCOMPLETE_OK
from __future__ import annotations

import argparse
import asyncio
import json
import os
import platform
import shutil
import sys
from dataclasses import dataclass, field, replace
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from typing import Any

import aiohttp

try:
    import argcomplete
except ModuleNotFoundError:  # pragma: no cover - optional dependency for normal runtime
    argcomplete = None  # type: ignore[assignment]

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python <3.11 compatibility for source users
    import tomli as tomllib  # type: ignore[no-redef]

SRC_DIR = Path(__file__).resolve().parent
if str(SRC_DIR) not in sys.path:
    sys.path.insert(0, str(SRC_DIR))

from config import (  # noqa: E402
    apply_preset,
    config_to_dict,
    dump_config,
    get_config_value,
    load_config,
    set_config_value,
    validate_config,
    write_config,
)
from core.errors import ConfigError  # noqa: E402
from core.logging import configure_logging, get_logger  # noqa: E402

_LOCAL_API_TIMEOUT_SECONDS = 2.0
_RUN_COMMANDS = ("run", "serve", "run-collectors")
_ACTIVE_INCIDENT_STATES = ("open", "investigating", "explained")

_ONE_SHOT_COMMANDS: dict[str, dict[str, str]] = {
    "collect-host": {
        "source_type": "host_metrics",
        "label": "host metrics",
        "config_key": "collectors.host_metrics.enabled",
    },
    "collect-processes": {
        "source_type": "process_snapshot",
        "label": "process snapshots",
        "config_key": "collectors.process.enabled",
    },
    "collect-services": {
        "source_type": "windows_service",
        "label": "Windows services",
        "config_key": "collectors.windows_service.enabled",
        "platform": "windows",
    },
    "collect-eventlog": {
        "source_type": "windows_eventlog",
        "label": "Windows Event Log",
        "config_key": "collectors.windows_eventlog.enabled",
        "platform": "windows",
    },
    "collect-syslog": {
        "source_type": "linux_syslog",
        "label": "Linux syslog",
        "config_key": "collectors.linux_syslog.enabled",
        "platform": "linux",
    },
    "collect-journald": {
        "source_type": "linux_journald",
        "label": "journald",
        "config_key": "collectors.journald.enabled",
        "platform": "linux",
    },
    "collect-kubernetes": {
        "source_type": "kubernetes",
        "label": "Kubernetes",
        "config_key": "collectors.kubernetes.enabled",
    },
}

_README_HELP_COMMANDS = [
    "inferra setup --yes --skip-connection-test",
    "inferra serve",
    "inferra init-db",
    "inferra reason-incident inc-abcdef1234567890",
    "inferra check-config",
    "inferra check-config --json",
    "inferra ai status",
    "inferra ai models",
    "inferra ai test",
    "inferra ai pull gemma4:e4b",
    "inferra collectors status",
    "inferra collectors start",
    "inferra collectors stop",
    "inferra run-collectors",
    "inferra collect-host",
    "inferra collect-processes",
    "inferra collect-services",
    "inferra collect-eventlog",
    "inferra collect-syslog",
    "inferra collect-journald",
    "inferra collect-kubernetes",
    "inferra config show",
    "inferra config get ai.model",
    "inferra config set ai.enabled true",
    "inferra config preset windows-server",
    "inferra calibration show",
    "inferra completion powershell",
]


@dataclass(slots=True)
class CommandResult:
    payload: dict[str, Any]
    stdout_lines: list[str] = field(default_factory=list)
    stderr_lines: list[str] = field(default_factory=list)
    exit_code: int = 0


class CommandError(RuntimeError):
    """Raised for user-facing CLI failures."""


def main(argv: list[str] | None = None) -> int:
    parser = _build_parser()
    if argcomplete is not None:  # pragma: no branch
        argcomplete.autocomplete(parser)
    try:
        args = parser.parse_args(argv)
    except SystemExit:  # pragma: no cover - argparse owns help/version exits
        raise
    if args.version:
        print(_project_version())
        return 0
    if not hasattr(args, "handler"):
        parser.print_help()
        return 0
    try:
        return asyncio.run(args.handler(args, parser))
    except ConfigError as exc:
        print(f"Config error: {exc}", file=sys.stderr)
        return 1
    except CommandError as exc:
        print(str(exc), file=sys.stderr)
        return 1


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="inferra",
        description="Inferra local-first runtime failure explanation CLI.",
        epilog=_help_epilog(),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--version", action="store_true", help="Print version and exit")
    parser.add_argument("--config", default="inferra.toml", help="Path to inferra.toml")
    parser.add_argument("--json", action="store_true", help="Print machine-readable JSON output")
    sub = parser.add_subparsers(dest="command")

    run = sub.add_parser("run", help="Start the Inferra web UI and live pipeline")
    _add_shared_command_options(run)
    run.add_argument("--collectors-only", action="store_true", help="Start collectors and normalization only")
    _add_server_bind_options(run)
    run.set_defaults(handler=_handle_run)

    serve = sub.add_parser("serve", help="Alias for `inferra run`")
    _add_shared_command_options(serve)
    _add_server_bind_options(serve)
    serve.set_defaults(handler=_handle_run, collectors_only=False)

    run_collectors = sub.add_parser("run-collectors", help="Start supervised collectors and normalization only")
    _add_shared_command_options(run_collectors)
    _add_server_bind_options(run_collectors)
    run_collectors.set_defaults(handler=_handle_run_collectors)

    setup = sub.add_parser("setup", help="Create local config, storage, and first-run defaults")
    _add_shared_command_options(setup)
    setup.add_argument("--yes", action="store_true", help="Accept defaults and skip interactive prompts")
    setup.add_argument("--model", default=None, help="Ollama model tag to configure")
    setup.add_argument("--skip-connection-test", action="store_true", help="Skip Ollama connectivity checks")
    setup.set_defaults(handler=_handle_setup)

    check = sub.add_parser("check-config", help="Validate inferra.toml and local prerequisites")
    _add_shared_command_options(check)
    check.set_defaults(handler=_handle_check_config)

    config = sub.add_parser("config", help="Read or update configuration")
    _add_shared_command_options(config)
    config_sub = config.add_subparsers(dest="config_command")

    config_show = config_sub.add_parser("show", help="Print the fully typed configuration as TOML")
    config_show.set_defaults(handler=_handle_config_show)

    config_get = config_sub.add_parser("get", help="Print a dotted config value")
    config_get.add_argument("key")
    config_get.set_defaults(handler=_handle_config_get)

    config_set = config_sub.add_parser("set", help="Set a dotted config value")
    config_set.add_argument("key")
    config_set.add_argument("value")
    config_set.set_defaults(handler=_handle_config_set)

    config_preset = config_sub.add_parser("preset", help="Apply a collector preset")
    config_preset.add_argument("name")
    config_preset.set_defaults(handler=_handle_config_preset)

    collectors = sub.add_parser("collectors", help="Talk to the running collector supervisor or inspect config")
    _add_shared_command_options(collectors)
    collectors_sub = collectors.add_subparsers(dest="collectors_command")

    collectors_status = collectors_sub.add_parser("status", help="Show live collector status or configured collectors")
    collectors_status.set_defaults(handler=_handle_collectors_status)

    collectors_start = collectors_sub.add_parser("start", help="Ask a running Inferra daemon to start collectors")
    collectors_start.set_defaults(handler=_handle_collectors_start)

    collectors_stop = collectors_sub.add_parser("stop", help="Ask a running Inferra daemon to stop collectors")
    collectors_stop.set_defaults(handler=_handle_collectors_stop)

    ai = sub.add_parser("ai", help="Ollama AI provider controls")
    _add_shared_command_options(ai)
    ai_sub = ai.add_subparsers(dest="ai_command")

    ai_status = ai_sub.add_parser("status", help="Probe the configured Ollama provider")
    ai_status.set_defaults(handler=_handle_ai_status)

    ai_models = ai_sub.add_parser("models", help="List Gemma registry entries and installed Ollama models")
    ai_models.set_defaults(handler=_handle_ai_models)

    ai_pull = ai_sub.add_parser("pull", help="Pull an Ollama model with streamed progress")
    ai_pull.add_argument("model", nargs="?", default=None, help="Model tag to pull")
    ai_pull.set_defaults(handler=_handle_ai_pull)

    ai_test = ai_sub.add_parser("test", help="Run a short provider health prompt")
    ai_test.set_defaults(handler=_handle_ai_test)

    init_db = sub.add_parser("init-db", help="Idempotently create or upgrade database schemas")
    _add_shared_command_options(init_db)
    init_db.set_defaults(handler=_handle_init_db)

    reason_incident = sub.add_parser(
        "reason-incident",
        help="Re-run deterministic hypothesis reasoning for an incident and print ranked results",
    )
    _add_shared_command_options(reason_incident)
    reason_incident.add_argument("incident_id", help="Incident id, for example inc-abcdef1234567890")
    reason_incident.set_defaults(handler=_handle_reason_incident)

    storage = sub.add_parser("storage", help="Storage maintenance commands")
    _add_shared_command_options(storage)
    storage_sub = storage.add_subparsers(dest="storage_command")

    storage_verify = storage_sub.add_parser("verify", help="Run integrity checks on all databases")
    storage_verify.set_defaults(handler=_handle_storage_verify)

    storage_vacuum = storage_sub.add_parser("vacuum", help="Run incremental vacuum on all databases")
    storage_vacuum.set_defaults(handler=_handle_storage_vacuum)

    storage_backup = storage_sub.add_parser("backup", help="Back up databases to a directory")
    storage_backup.add_argument("path", help="Destination directory for backups")
    storage_backup.set_defaults(handler=_handle_storage_backup)

    for command_name, metadata in _ONE_SHOT_COMMANDS.items():
        one_shot = sub.add_parser(command_name, help=f"Run the {metadata['label']} collector once")
        _add_shared_command_options(one_shot)
        one_shot.set_defaults(handler=_handle_collect_once, **metadata)

    reset_baselines = sub.add_parser("reset-baselines", help="Delete learned baseline data")
    _add_shared_command_options(reset_baselines)
    reset_baselines.set_defaults(handler=_handle_reset_baselines)

    reset_weights = sub.add_parser("reset-weights", help="Reset scoring weights to defaults")
    _add_shared_command_options(reset_weights)
    reset_weights.set_defaults(handler=_handle_reset_weights)

    calibration = sub.add_parser("calibration", help="Inspect confidence calibration state")
    _add_shared_command_options(calibration)
    calib_sub = calibration.add_subparsers(dest="calibration_command", required=True)
    calib_show = calib_sub.add_parser("show", help="Print calibration buckets and staleness")
    calib_show.set_defaults(handler=_handle_calibration_show)

    completion = sub.add_parser("completion", help="Generate shell completion for bash, zsh, fish, or PowerShell")
    completion.add_argument("shell", choices=("bash", "zsh", "fish", "powershell"))
    completion.set_defaults(handler=_handle_completion)
    return parser


def _add_shared_command_options(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--config", default=argparse.SUPPRESS, help="Path to inferra.toml")
    parser.add_argument("--data-dir", default=None, help="Override [storage].data_dir for this process")
    parser.add_argument(
        "--json",
        action="store_true",
        default=argparse.SUPPRESS,
        help="Print machine-readable JSON output",
    )


def _add_server_bind_options(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--host", default=None, help="Override [server].host for this process (e.g. 0.0.0.0)")
    parser.add_argument("--port", type=int, default=None, help="Override [server].port for this process")


async def _handle_setup(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from ai import AIService
    from storage.migrations import migrate

    config_path = _config_path(args)
    config_exists = config_path.exists()
    current = load_config(config_path)
    config = replace(current, ai=replace(current.ai, enabled=True, provider="ollama"))

    if args.data_dir is not None:
        config = replace(config, storage=replace(config.storage, data_dir=Path(args.data_dir)))
    if args.model is not None:
        config = replace(config, ai=replace(config.ai, model=args.model.strip()))

    skip_connection_test = bool(args.skip_connection_test)
    if not args.yes:
        if not sys.stdin.isatty():
            raise CommandError("Interactive setup requires a TTY. Re-run with `--yes` for non-interactive setup.")
        config, skip_connection_test = _interactive_setup(config, config_path, config_exists, skip_connection_test)

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
    if skip_connection_test:
        connection_test = {"skipped": True}
    else:
        payload = await AIService(config).status()
        connection_test = {
            "skipped": False,
            "available": payload.get("available", False),
            "reason": payload.get("reason"),
            "error": payload.get("error"),
            "installed": payload.get("installed", False),
            "resolved_model": payload.get("resolved_model"),
            "version": payload.get("version"),
        }
        if not payload.get("available", False):
            exit_code = 1
            stderr_lines.append(
                f"Ollama probe failed at {config.ai.base_url}: {payload.get('error') or payload.get('reason') or 'unknown error'}"
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
        },
        "connection_test": connection_test,
    }
    stdout_lines = [
        f"{'Wrote' if wrote_config else 'Validated'} config at {config_path}",
        f"Initialized storage under {data_dir}",
        f"{events_path.name}: schema version {events_version}",
        f"{incidents_path.name}: schema version {incidents_version}",
        "Skipped Ollama connection test" if skip_connection_test else _human_connection_line(connection_test, config.ai.base_url),
    ]
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines, stderr_lines=stderr_lines, exit_code=exit_code))


async def _handle_run(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from app import InferraRuntime
    from web import create_app
    import uvicorn

    config_path, config = _load_config_for_command(args)
    if getattr(args, "host", None) is not None:
        config = replace(config, server=replace(config.server, host=str(args.host).strip()))
    if getattr(args, "port", None) is not None:
        config = replace(config, server=replace(config.server, port=int(args.port)))
    collectors_only = bool(getattr(args, "collectors_only", False))
    logger = get_logger(__name__)
    runtime = InferraRuntime(config)
    startup_payload = {
        "command": getattr(args, "command", "run"),
        "config_path": str(config_path),
        "collectors_only": collectors_only,
        "server": {"host": config.server.host, "port": config.server.port},
        "url": f"http://{config.server.host}:{config.server.port}",
    }
    if args.json:
        _print_json(startup_payload)
    try:
        await runtime.start(start_collectors=True)
        logger.info("Inferra collectors and normalization started", extra={"collectors_only": collectors_only})
        if collectors_only:
            if not args.json:
                print("Inferra collectors running. Press Ctrl+C to stop.")
            await _sleep_forever()
        else:
            app = create_app(runtime=runtime, config_path=config_path)
            server = uvicorn.Server(
                uvicorn.Config(app, host=config.server.host, port=config.server.port, log_config=None)
            )
            if not args.json:
                print(f"Inferra running at http://{config.server.host}:{config.server.port}")
            await server.serve()
    except KeyboardInterrupt:
        logger.info("Inferra shutdown requested")
    finally:
        await runtime.stop()
    return 0


async def _handle_run_collectors(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    setattr(args, "collectors_only", True)
    return await _handle_run(args, parser)


async def _handle_check_config(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    config_path, config = _load_config_for_command(args)
    report = await _build_check_report(config_path, config=config)
    stdout_lines: list[str] = []
    stderr_lines: list[str] = []
    if report["errors"]:
        stderr_lines.extend(f"Error: {error['message']}" for error in report["errors"])
    else:
        stdout_lines.append(f"Inferra config OK: {report['config_path']}")
        stdout_lines.extend(f"{key}={value}" for key, value in report["summary"].items())
    stderr_lines.extend(f"Warning: {warning['message']}" for warning in report["warnings"])
    return _emit_result(
        args,
        CommandResult(payload=report, stdout_lines=stdout_lines, stderr_lines=stderr_lines, exit_code=0 if report["ok"] else 1),
    )


async def _handle_config_show(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    config_path, config = _load_config_for_command(args)
    payload = {"command": "config show", "config_path": str(config_path), "config": config_to_dict(config)}
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=[dump_config(config).rstrip("\n")]))


async def _handle_config_get(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    config_path, config = _load_config_for_command(args)
    value = get_config_value(config, args.key)
    payload = {"command": "config get", "config_path": str(config_path), "key": args.key, "value": _json_ready(value)}
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=[_format_config_value(value)]))


async def _handle_config_set(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    config_path, config = _load_config_for_command(args)
    previous = get_config_value(config, args.key)
    updated = set_config_value(config_path, args.key, args.value)
    payload = {
        "command": "config set",
        "config_path": str(config_path),
        "key": args.key,
        "previous": _json_ready(previous),
        "value": _json_ready(get_config_value(updated, args.key)),
    }
    return _emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=[f"Updated {args.key}={_format_config_value(get_config_value(updated, args.key))}"]),
    )


async def _handle_config_preset(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    config_path, config = _load_config_for_command(args)
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
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=[f"Applied preset {args.name}"]))


async def _handle_collectors_status(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    config_path, config = _load_config_for_command(args)
    server_url = _server_url(config)
    live_payload: dict[str, Any] | None = None
    live_error: str | None = None
    try:
        live_payload = await _local_api_json(config, "GET", "/api/collectors")
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
        stdout_lines.extend(_format_collector_line(item) for item in payload["collectors"])
        return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))

    payload = {
        "command": "collectors status",
        "mode": "configured",
        "config_path": str(config_path),
        "server_url": server_url,
        "running": False,
        "queue_depth": 0,
        "collectors": _configured_collectors(config),
        "hint": "Start the live supervisor with `inferra run`.",
    }
    stdout_lines = [f"Configured collectors: {len(payload['collectors'])}"]
    stdout_lines.extend(_format_collector_line(item) for item in payload["collectors"])
    stderr_lines = [f"No running Inferra supervisor found at {server_url}. Start it with `inferra run`."] if live_error else []
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines, stderr_lines=stderr_lines))


async def _handle_collectors_start(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    config_path, config = _load_config_for_command(args)
    payload = await _require_local_api(config, "POST", "/api/collectors/start")
    payload = {
        "command": "collectors start",
        "config_path": str(config_path),
        "server_url": _server_url(config),
        **payload,
    }
    stdout_lines = [f"Started collectors through {_server_url(config)}"]
    stdout_lines.extend(_format_collector_line(item) for item in payload.get("collectors", []))
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def _handle_collectors_stop(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    config_path, config = _load_config_for_command(args)
    payload = await _require_local_api(config, "POST", "/api/collectors/stop")
    payload = {
        "command": "collectors stop",
        "config_path": str(config_path),
        "server_url": _server_url(config),
        **payload,
    }
    stdout_lines = [f"Stopped collectors through {_server_url(config)}"]
    stdout_lines.extend(_format_collector_line(item) for item in payload.get("collectors", []))
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def _handle_ai_status(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from ai import AIService

    config_path, config = _load_config_for_command(args)
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
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines, exit_code=exit_code))


async def _handle_ai_models(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from ai import AIService

    config_path, config = _load_config_for_command(args)
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
                "installed": bool(model["name"] in installed_set or (model.get("resolves_to") in installed_set)),
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
    return _emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=stdout_lines, stderr_lines=stderr_lines, exit_code=1 if error else 0),
    )


async def _handle_ai_pull(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from ai import AIService

    config_path, config = _load_config_for_command(args)
    service = AIService(config)
    model = (args.model or config.ai.model).strip()
    if not model:
        raise CommandError("Model tag is required.")
    if args.json:
        await service.pull_model(model)
        payload = {"command": "ai pull", "config_path": str(config_path), "model": model, "complete": True}
        return _emit_result(args, CommandResult(payload=payload, stdout_lines=[]))

    async for progress in service.pull_model_stream(model):
        percent = f"{progress.percent:5.1f}%" if progress.percent is not None else "  ... "
        status = progress.status or "pulling"
        digest = f" {progress.digest[:12]}" if progress.digest else ""
        print(f"\r{model} {percent} {status}{digest}", end="", flush=True)
    print(f"\r{model} 100.0% complete{' ' * 24}")
    return 0


async def _handle_ai_test(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from ai import AIService

    config_path, config = _load_config_for_command(args)
    if not config.ai.enabled:
        payload = {
            "command": "ai test",
            "config_path": str(config_path),
            "enabled": False,
            "ok": False,
            "reason": "AI is disabled in config.",
        }
        return _emit_result(args, CommandResult(payload=payload, stdout_lines=["AI is disabled in config."], exit_code=1))
    response = await AIService(config).test()
    payload = {
        "command": "ai test",
        "config_path": str(config_path),
        "enabled": True,
        "ok": True,
        "model": config.ai.model,
        "response": response,
    }
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=[response]))


async def _handle_reason_incident(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from dataclasses import replace

    from core.time import utc_now
    from reasoning.engine import HypothesisEngine, hypothesis_dict_to_scored
    from runtime.service_graph import ServiceGraph
    from storage.event_store import SqliteEventStore
    from storage.incident_store import SqliteIncidentStore

    config_path, config = _load_config_for_command(args)
    data_dir = Path(config.storage.data_dir)
    events_path = data_dir / config.storage.events_db
    incidents_path = data_dir / config.storage.incidents_db
    if not events_path.exists() or not incidents_path.exists():
        raise CommandError("Storage databases are missing. Run `inferra init-db` first.")

    service_graph = ServiceGraph()
    for edge in config.topology.edges:
        service_graph.add_relation(edge.source, edge.target, edge.type)

    mmap_bytes = int(config.storage.mmap_size_mb) * 1024 * 1024 if config.storage.enable_mmap else 0
    event_store = SqliteEventStore(
        events_path,
        batch_size=config.storage.batch_size,
        retention_hours=config.storage.retention_hours,
        prune_interval_seconds=config.storage.prune_interval_seconds,
        wal_mode=config.storage.wal_mode,
        start_pruner=False,
        mmap_size_bytes=mmap_bytes,
    )
    incident_store = SqliteIncidentStore(
        incidents_path,
        wal_mode=config.storage.wal_mode,
        mmap_size_bytes=mmap_bytes,
        start_archiver=False,
    )
    try:
        incident = incident_store.get_incident(args.incident_id)
        if incident is None:
            raise CommandError(f"Incident not found: {args.incident_id}")
        events: list[Any] = []
        for event_id in incident.events:
            stored = event_store.get_event(event_id)
            if stored is not None:
                events.append(stored)
        from storage.calibration_store import CalibrationStore
        from storage.weight_store import WeightStore

        engine = HypothesisEngine(
            service_graph,
            config,
            weight_store=WeightStore(data_dir / "scoring_weights.json", data_dir / "weight_history.jsonl"),
            calibration_store=CalibrationStore(data_dir / "calibration.json"),
        )
        payloads = engine.generate(args.incident_id, events, incident=incident, incident_event_ids=list(incident.events))
        scored = [hypothesis_dict_to_scored(item) for item in payloads]
        incident_store.add_hypotheses(args.incident_id, scored)
        if engine.last_inference_graph is not None:
            incident_store.save_inference_graph(args.incident_id, engine.last_inference_graph)
            incident_store.update_incident(
                replace(incident, inference_graph=engine.last_inference_graph, updated_at=utc_now())
            )
        payload: dict[str, Any] = {
            "command": "reason-incident",
            "config_path": str(config_path),
            "incident_id": args.incident_id,
            "hypothesis_count": len(payloads),
            "hypotheses": payloads,
        }
        stdout_lines = [
            f"{item['rank']}. {item['cause_type']} score={item['total_score']} {str(item['description'])[:120]}"
            for item in payloads
        ]
        return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))
    finally:
        event_store.close()
        incident_store.close()


async def _handle_init_db(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from storage.migrations import CURRENT_SCHEMA_VERSION, integrity_check, migrate

    _config_path_for_logging, config = _load_config_for_command(args)
    data_dir = Path(config.storage.data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)
    events_path = data_dir / config.storage.events_db
    incidents_path = data_dir / config.storage.incidents_db

    events_version = migrate(events_path)
    incidents_version = migrate(incidents_path)
    integrity_check(events_path)
    integrity_check(incidents_path)

    payload = {
        "command": "init-db",
        "data_dir": str(data_dir),
        "schema_version": CURRENT_SCHEMA_VERSION,
        "databases": [
            {"path": str(events_path), "schema_version": events_version, "integrity_ok": True},
            {"path": str(incidents_path), "schema_version": incidents_version, "integrity_ok": True},
        ],
    }
    stdout_lines = [
        f"{events_path.name}: schema version {events_version}",
        f"{incidents_path.name}: schema version {incidents_version}",
        f"Databases initialized at version {CURRENT_SCHEMA_VERSION} under {data_dir}",
    ]
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def _handle_storage_verify(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from storage.migrations import integrity_check

    _config_path_for_logging, config = _load_config_for_command(args)
    results: list[dict[str, Any]] = []
    stdout_lines: list[str] = []
    stderr_lines: list[str] = []
    exit_code = 0
    for db_path in _database_paths(config):
        row = {"path": str(db_path), "name": db_path.name, "exists": db_path.exists()}
        if not db_path.exists():
            row["status"] = "missing"
            stderr_lines.append(f"SKIP {db_path.name}: file does not exist")
            results.append(row)
            continue
        try:
            integrity_check(db_path)
            row["status"] = "ok"
            stdout_lines.append(f"OK   {db_path.name}")
        except Exception as exc:  # pragma: no cover - exercised in storage tests elsewhere
            row["status"] = "failed"
            row["error"] = str(exc)
            stderr_lines.append(f"FAIL {db_path.name}: {exc}")
            exit_code = 1
        results.append(row)
    payload = {"command": "storage verify", "databases": results}
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines, stderr_lines=stderr_lines, exit_code=exit_code))


async def _handle_storage_vacuum(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from storage.migrations import vacuum_db

    _config_path_for_logging, config = _load_config_for_command(args)
    results: list[dict[str, Any]] = []
    stdout_lines: list[str] = []
    for db_path in _database_paths(config):
        if not db_path.exists():
            results.append({"path": str(db_path), "name": db_path.name, "status": "missing"})
            continue
        vacuum_db(db_path)
        results.append({"path": str(db_path), "name": db_path.name, "status": "vacuumed"})
        stdout_lines.append(f"Vacuumed {db_path.name}")
    payload = {"command": "storage vacuum", "databases": results}
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def _handle_storage_backup(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from storage.migrations import backup_db

    _config_path_for_logging, config = _load_config_for_command(args)
    dest_dir = Path(args.path)
    dest_dir.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, Any]] = []
    stdout_lines: list[str] = []
    for db_path in _database_paths(config):
        if not db_path.exists():
            results.append({"path": str(db_path), "name": db_path.name, "status": "missing"})
            continue
        dest_path = backup_db(db_path, dest_dir / db_path.name)
        results.append({"path": str(db_path), "backup_path": str(dest_path), "status": "backed_up"})
        stdout_lines.append(f"Backed up {db_path.name} -> {dest_path}")
    payload = {"command": "storage backup", "destination": str(dest_dir), "databases": results}
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def _handle_collect_once(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from app import InferraRuntime

    expected_platform = args.__dict__.get("platform")
    current_platform = platform.system().lower()
    if expected_platform and current_platform != expected_platform:
        raise CommandError(f"`{args.command}` is only available on {expected_platform}.")

    config_path, config = _load_config_for_command(args)
    runtime = InferraRuntime(config)
    try:
        await runtime.start(start_collectors=False)
        try:
            summary = await runtime.collect_source_once(args.source_type)
        except ValueError as exc:
            raise CommandError(
                f"No enabled {args.label} collector is configured. Enable `{args.config_key}` or apply a preset."
            ) from exc
    finally:
        await runtime.stop()

    payload = {
        "command": args.command,
        "config_path": str(config_path),
        "label": args.label,
        **summary,
    }
    stdout_lines = [
        f"Ran {args.label} collector once.",
        f"raw_events_emitted={summary['raw_events_emitted']}",
        f"events_stored={summary['events_stored']}",
        f"collector_count={summary['collector_count']}",
    ]
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def _handle_reset_baselines(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    _config_path_for_logging, config = _load_config_for_command(args)
    baseline_dir = Path(config.storage.data_dir) / "baselines"
    if baseline_dir.exists():
        shutil.rmtree(baseline_dir)
    baseline_dir.mkdir(parents=True, exist_ok=True)
    payload = {"command": "reset-baselines", "baseline_dir": str(baseline_dir), "deleted": True}
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=[f"Deleted baseline data under {baseline_dir}"]))


async def _handle_reset_weights(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from reasoning.scoring import merge_config_weights
    from storage.weight_store import WeightStore, reset_weights

    _config_path_for_logging, config = _load_config_for_command(args)
    data_dir = Path(config.storage.data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)
    defaults = merge_config_weights({}, config)
    store = WeightStore(data_dir / "scoring_weights.json", data_dir / "weight_history.jsonl")
    state = store.load()
    state.default_weights = dict(defaults)
    reset_weights(state)
    store.save(state)
    path = store.path
    payload = {"command": "reset-weights", "path": str(path), "weights": dict(state.weights)}
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=[f"Reset scoring weights at {path}"]))


async def _handle_calibration_show(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from storage.calibration_store import CalibrationStore, check_calibration_staleness

    _config_path_for_logging, config = _load_config_for_command(args)
    path = Path(config.storage.data_dir) / "calibration.json"
    store = CalibrationStore(path)
    model = store.load()
    stale = check_calibration_staleness(
        model,
        staleness_days=int(config.calibration.staleness_threshold_days),
        min_feedback=20,
    )
    payload = {
        "command": "calibration show",
        "path": str(path),
        "staleness": stale,
        "total_feedback_count": model.total_feedback_count,
        "buckets": [
            {
                "score_lower": bucket.score_lower,
                "score_upper": bucket.score_upper,
                "total_predictions": bucket.total_predictions,
                "correct_predictions": bucket.correct_predictions,
                "accuracy": bucket.accuracy,
                "sample_confidence": bucket.sample_confidence,
            }
            for bucket in model.buckets
        ],
    }
    lines = [f"Calibration file: {path}", f"staleness={stale}", f"total_feedback_count={model.total_feedback_count}"]
    for bucket in model.buckets:
        lines.append(
            f"  [{bucket.score_lower},{bucket.score_upper}) n={bucket.total_predictions} "
            f"correct={bucket.correct_predictions} acc={bucket.accuracy:.3f} {bucket.sample_confidence}"
        )
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=lines))


async def _handle_completion(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    if argcomplete is None:
        raise CommandError("Shell completion requires `argcomplete`. Install the dev extras or add the dependency.")
    script = argcomplete.shellcode(["inferra"], shell=args.shell)
    payload = {"command": "completion", "shell": args.shell, "script": script}
    return _emit_result(args, CommandResult(payload=payload, stdout_lines=[script.rstrip("\n")]))


def _emit_result(args: argparse.Namespace, result: CommandResult) -> int:
    if getattr(args, "json", False):
        _print_json(result.payload)
    else:
        for line in result.stdout_lines:
            print(line)
        for line in result.stderr_lines:
            print(line, file=sys.stderr)
    return result.exit_code


def _print_json(payload: Any) -> None:
    print(json.dumps(_json_ready(payload), indent=2, sort_keys=True))


def _json_ready(value: Any) -> Any:
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, tuple):
        return [_json_ready(item) for item in value]
    if isinstance(value, list):
        return [_json_ready(item) for item in value]
    if isinstance(value, set):
        return [_json_ready(item) for item in sorted(value)]
    if isinstance(value, dict):
        return {str(key): _json_ready(item) for key, item in value.items()}
    return value


def _config_path(args: argparse.Namespace) -> Path:
    return Path(getattr(args, "config", "inferra.toml") or "inferra.toml")


def _load_config_for_command(args: argparse.Namespace) -> tuple[Path, Any]:
    config_path = _config_path(args)
    config = load_config(config_path)
    data_dir = getattr(args, "data_dir", None)
    if data_dir is not None:
        config = replace(config, storage=replace(config.storage, data_dir=Path(data_dir)))
    configure_logging(config)
    return config_path, config


async def _build_check_report(config_path: Path, config: Any | None = None) -> dict[str, Any]:
    report: dict[str, Any] = {
        "ok": False,
        "config_path": str(config_path),
        "warnings": [],
        "errors": [],
        "summary": {},
    }
    try:
        if config is None:
            config = load_config(config_path)
        validate_config(config)
    except ConfigError as exc:
        report["errors"].append({"code": "invalid_config", "message": str(exc), "details": {}})
        return report

    report["summary"] = {
        "server": f"{config.server.host}:{config.server.port}",
        "data_dir": str(config.storage.data_dir),
        "ai_enabled": config.ai.enabled,
        "topology_edges": len(config.topology.edges),
    }
    _check_data_dir_writable(config, report)
    if config.ai.enabled:
        await _check_ai_reachable(config, report)
    _check_topology_event_coverage(config, report)
    report["ok"] = not report["errors"]
    return report


def _interactive_setup(config: Any, config_path: Path, config_exists: bool, skip_connection_test: bool) -> tuple[Any, bool]:
    print(f"Config path: {config_path}")
    print(f"Current data_dir: {config.storage.data_dir}")
    print(f"Current AI model: {config.ai.model}")
    data_dir = _prompt_value("Storage data_dir", str(config.storage.data_dir))
    model = _prompt_value("Ollama model", config.ai.model)
    run_probe = not skip_connection_test and _prompt_yes_no("Probe Ollama connection now?", default=True)
    if not _prompt_yes_no("Continue with setup?", default=True):
        raise CommandError("Setup cancelled.")
    updated = replace(
        config,
        storage=replace(config.storage, data_dir=Path(data_dir)),
        ai=replace(config.ai, enabled=True, provider="ollama", model=model),
    )
    return updated, not run_probe


def _prompt_value(label: str, default: str) -> str:
    response = input(f"{label} [{default}]: ").strip()
    return response or default


def _prompt_yes_no(label: str, *, default: bool) -> bool:
    suffix = "Y/n" if default else "y/N"
    response = input(f"{label} [{suffix}]: ").strip().lower()
    if not response:
        return default
    return response in {"y", "yes"}


def _human_connection_line(connection_test: dict[str, Any], base_url: str) -> str:
    if connection_test.get("available"):
        resolved = connection_test.get("resolved_model")
        if resolved:
            return f"Ollama reachable at {base_url} (resolved_model={resolved})"
        return f"Ollama reachable at {base_url}"
    return f"Ollama unavailable at {base_url}"


def _configured_collectors(config: Any) -> list[dict[str, Any]]:
    from collectors import build_collectors

    rows: list[dict[str, Any]] = []
    for collector in build_collectors(config):
        health = collector.health()
        rows.append(
            {
                "collector_id": health.collector_id,
                "source_type": health.source_type,
                "status": "not_running",
                "is_running": health.is_running,
                "events_emitted": health.events_emitted,
                "events_per_second": health.events_per_second,
                "last_event_at": None,
                "error_count": health.error_count,
                "dropped_events": health.dropped_events,
                "queue_depth": 0,
                "last_error": health.last_error,
                "lag_seconds": health.lag_seconds,
            }
        )
    return rows


def _format_collector_line(item: dict[str, Any]) -> str:
    return " | ".join(
        [
            str(item["collector_id"]),
            f"status={item.get('status', 'unknown')}",
            f"queue_depth={item.get('queue_depth', 0)}",
            f"errors={item.get('error_count', 0)}",
            f"dropped={item.get('dropped_events', 0)}",
            f"last_event_at={item.get('last_event_at') or '-'}",
        ]
    )


def _server_url(config: Any) -> str:
    return f"http://{config.server.host}:{config.server.port}"


async def _require_local_api(config: Any, method: str, path: str, payload: dict[str, Any] | None = None) -> dict[str, Any]:
    try:
        return await _local_api_json(config, method, path, payload)
    except CommandError as exc:
        raise CommandError(f"{exc} Start it with `inferra run`.") from exc


async def _local_api_json(
    config: Any,
    method: str,
    path: str,
    payload: dict[str, Any] | None = None,
) -> dict[str, Any]:
    url = f"{_server_url(config)}{path}"
    timeout = aiohttp.ClientTimeout(total=_LOCAL_API_TIMEOUT_SECONDS)
    try:
        async with aiohttp.ClientSession(timeout=timeout) as session:
            async with session.request(method, url, json=payload) as response:
                body = await response.text()
                if response.status >= 400:
                    raise CommandError(f"Inferra is reachable at {url} but returned HTTP {response.status}: {body}")
    except (aiohttp.ClientError, asyncio.TimeoutError) as exc:
        raise CommandError(f"No running Inferra supervisor found at {url}.") from exc
    try:
        decoded = json.loads(body or "{}")
    except json.JSONDecodeError as exc:
        raise CommandError(f"Inferra returned invalid JSON from {url}.") from exc
    if not isinstance(decoded, dict):
        raise CommandError(f"Inferra returned an unexpected payload from {url}.")
    return decoded


def _database_paths(config: Any) -> list[Path]:
    data_dir = Path(config.storage.data_dir)
    return [data_dir / config.storage.events_db, data_dir / config.storage.incidents_db]


def _format_config_value(value: Any) -> str:
    if isinstance(value, (dict, list, tuple, set)):
        return json.dumps(_json_ready(value), indent=2, sort_keys=True)
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def _check_data_dir_writable(config: Any, report: dict[str, Any]) -> None:
    data_dir = Path(config.storage.data_dir)
    target = data_dir if data_dir.exists() else _nearest_existing_parent(data_dir)
    if not target.exists() or not target.is_dir() or not _path_is_writable(target):
        _add_warning(
            report,
            "data_dir_not_writable",
            f"Storage data_dir is not writable: {data_dir}",
            {"data_dir": str(data_dir), "checked_path": str(target)},
        )


async def _check_ai_reachable(config: Any, report: dict[str, Any]) -> None:
    from ai import AIService
    from ai.ollama import OllamaError

    try:
        payload = await AIService(config).status()
    except OllamaError as exc:
        _add_warning(
            report,
            "ai_unreachable",
            f"AI is enabled but Ollama is not reachable at {config.ai.base_url}: {exc}",
            {"base_url": config.ai.base_url, "reason": getattr(exc, "reason_code", "ollama_error")},
        )
        return
    if not payload.get("available", False):
        _add_warning(
            report,
            "ai_unreachable",
            f"AI is enabled but Ollama is not reachable at {config.ai.base_url}: {payload.get('error') or payload.get('reason')}",
            {"base_url": config.ai.base_url, "reason": payload.get("reason")},
        )


def _check_topology_event_coverage(config: Any, report: dict[str, Any]) -> None:
    services = sorted(
        {edge.source for edge in config.topology.edges if edge.source}
        | {edge.target for edge in config.topology.edges if edge.target}
    )
    if not services:
        return
    counts = _topology_service_counts(Path(config.storage.data_dir) / config.storage.events_db, services)
    for service in services:
        if counts.get(service, 0) == 0:
            _add_warning(
                report,
                "topology_service_has_no_events",
                f"Topology references service with zero events: {service}",
                {"service_id": service},
            )


def _topology_service_counts(db_path: Path, services: list[str]) -> dict[str, int]:
    from storage.event_store import count_events_by_service

    return count_events_by_service(db_path, services)


def _nearest_existing_parent(path: Path) -> Path:
    current = path
    while not current.exists() and current.parent != current:
        current = current.parent
    return current


def _path_is_writable(path: Path) -> bool:
    return os.access(path, os.W_OK)


def _add_warning(report: dict[str, Any], code: str, message: str, details: dict[str, Any]) -> None:
    report["warnings"].append({"code": code, "message": message, "details": details})


def _help_epilog() -> str:
    return "Common commands:\n  " + "\n  ".join(_README_HELP_COMMANDS)


async def _sleep_forever() -> None:
    while True:
        await asyncio.sleep(3600)


def _project_version() -> str:
    """Resolve package version for CLI `--version` and frozen PyInstaller builds."""
    frozen = getattr(sys, "frozen", False)
    meipass = getattr(sys, "_MEIPASS", None)
    if frozen and isinstance(meipass, str) and meipass:
        bundled = Path(meipass) / "pyproject.toml"
        if bundled.is_file():
            try:
                data = tomllib.loads(bundled.read_text(encoding="utf-8"))
                return str(data.get("project", {}).get("version", "0.1.0"))
            except (OSError, UnicodeError, TypeError, ValueError):
                pass

    pyproject = SRC_DIR.parent / "pyproject.toml"
    if pyproject.exists():
        data = tomllib.loads(pyproject.read_text(encoding="utf-8"))
        return str(data.get("project", {}).get("version", "0.1.0"))
    try:
        return version("inferra")
    except PackageNotFoundError:
        return "0.1.0"


if __name__ == "__main__":
    raise SystemExit(main())
