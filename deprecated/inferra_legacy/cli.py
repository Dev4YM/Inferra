# PYTHON_ARGCOMPLETE_OK
from __future__ import annotations

import argparse
import asyncio
import json
import os
import platform
import subprocess
import sys
from dataclasses import replace
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path
from typing import Any

try:
    import argcomplete
except ModuleNotFoundError:  # pragma: no cover - optional dependency for normal runtime
    argcomplete = None  # type: ignore[assignment]

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python <3.11 compatibility for source users
    import tomli as tomllib  # type: ignore[no-redef]

_REPO_ROOT = Path(__file__).resolve().parents[2]
_PY_PACKAGES = _REPO_ROOT / "deprecated" / "python_packages"
if str(_PY_PACKAGES) not in sys.path:
    sys.path.insert(0, str(_PY_PACKAGES))

from config import (  # noqa: E402
    PRESET_NAMES,
    apply_preset,
    load_config,
    validate_config,
)
from core.errors import ConfigError  # noqa: E402
from core.logging import configure_logging, get_logger  # noqa: E402

# These are re-exported here so command modules and tests can reach them via
# the ``cli`` module surface (e.g. ``cli._local_api_json``); ruff would flag
# them as unused otherwise.
from cli_core import (  # noqa: E402, F401
    CommandError,
    CommandResult,
    LOCAL_API_TIMEOUT_SECONDS as _LOCAL_API_TIMEOUT_SECONDS,
    emit_result as _emit_result,
    json_ready as _json_ready,
    local_api_json as _local_api_json,
    print_json as _print_json,
    server_url as _server_url,
)
from cli_core.commands import ai as _ai_cmds  # noqa: E402
from cli_core.commands import ai_provider as _ai_provider_cmds  # noqa: E402
from cli_core.commands import collectors as _collectors_cmds  # noqa: E402
from cli_core.commands import config as _config_cmds  # noqa: E402
from cli_core.commands import dashboard as _dashboard_cmds  # noqa: E402
from cli_core.commands import demo as _demo_cmds  # noqa: E402
from cli_core.commands import guide as _guide_cmds  # noqa: E402
from cli_core.commands import incidents as _incidents_cmds  # noqa: E402
from cli_core.commands import reasoning as _reasoning_cmds  # noqa: E402
from cli_core.commands import service as _service_cmds  # noqa: E402
from cli_core.commands import setup as _setup_cmds  # noqa: E402
from cli_core.commands import storage as _storage_cmds  # noqa: E402
from cli_core.commands import workspace as _workspace_cmds  # noqa: E402


async def _require_local_api(
    config: Any,
    method: str,
    path: str,
    payload: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Thin wrapper so test monkeypatching on `cli._local_api_json` is honored."""
    try:
        return await _local_api_json(config, method, path, payload)
    except CommandError as exc:
        raise CommandError(
            f"{exc} Start it with `inferra serve` or `inferra service start`."
        ) from exc
_RUN_COMMANDS = ("run", "serve", "run-collectors")
_ACTIVE_INCIDENT_STATES = ("open", "investigating", "explained")
_EXPERIENCE_MODES = ("operator", "expert", "developer")
_AI_ROLES = ("observer", "investigator", "researcher")

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
    "inferra setup --preset windows-server --model gemma4:e4b",
    "inferra onboard --mode operator --ai-role investigator",
    "inferra guide",
    "inferra guide --profile developer",
    "inferra guide --profile server",
    "inferra dashboard",
    "inferra dashboard --section ai",
    "inferra dashboard --no-open",
    "inferra serve",
    "inferra status",
    "inferra overview",
    "inferra investigate latest",
    "inferra ai ask \"what should I inspect first?\"",
    "inferra ai investigate latest",
    "inferra ai report inc-abcdef1234567890 --mode operator",
    "inferra ai trace inc-abcdef1234567890",
    "inferra ai doctor",
    "inferra incidents list",
    "inferra events list --limit 25",
    "inferra services list",
    "inferra doctor",
    "inferra workspace",
    "inferra workspace map",
    "inferra workspace services",
    "inferra workspace inspect D:\\Projects\\app",
    "inferra demo seed",
    "inferra demo clear",
    "inferra init-db",
    "inferra reason-incident inc-abcdef1234567890",
    "inferra check-config",
    "inferra check-config --json",
    "inferra ai status",
    "inferra ai setup --model gemma4:e4b",
    "inferra ai models",
    "inferra ai test",
    "inferra ai pull gemma4:e4b",
    "inferra service status",
    "inferra service install --startup auto",
    "inferra service repair",
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
    "inferra mode show",
    "inferra mode set developer",
    "inferra calibration show",
    "inferra completion powershell",
]




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

    onboard = sub.add_parser("onboard", help="Guided first-run onboarding for CLI, AI, collectors, and mode")
    _add_shared_command_options(onboard)
    _add_setup_options(onboard)
    onboard.set_defaults(handler=_handle_setup)

    setup = sub.add_parser("setup", help="Create local config, storage, and first-run defaults")
    _add_shared_command_options(setup)
    _add_setup_options(setup)
    setup.set_defaults(handler=_handle_setup)

    guide = sub.add_parser("guide", help="Show the next best setup or operating path for a user profile")
    _add_shared_command_options(guide)
    guide.add_argument(
        "--profile",
        choices=("operator", "expert", "developer", "server", "contributor"),
        default=None,
        help="Guide profile; defaults to configured experience mode",
    )
    guide.set_defaults(handler=_handle_guide)

    dashboard_cmd = sub.add_parser("dashboard", help="Open or print the local web control-plane URL")
    _add_shared_command_options(dashboard_cmd)
    dashboard_cmd.add_argument(
        "--section",
        choices=("overview", "incidents", "systems", "evidence", "ai", "workspace", "control", "settings"),
        default="overview",
        help="Dashboard section to open",
    )
    dashboard_cmd.add_argument("--no-open", action="store_true", help="Print the URL without opening a browser")
    dashboard_cmd.set_defaults(handler=_handle_dashboard)

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

    ai_setup = ai_sub.add_parser("setup", help="Configure AI defaults and optionally probe the provider")
    ai_setup.add_argument("--model", default=None, help="Ollama model tag to configure")
    ai_setup.add_argument("--base-url", default=None, help="Override the Ollama-compatible base URL")
    ai_setup.add_argument("--token-env", default=None, help="Environment variable name for remote Ollama auth")
    ai_toggle = ai_setup.add_mutually_exclusive_group()
    ai_toggle.add_argument("--enable", dest="setup_ai_enabled", action="store_true", default=None, help="Enable AI in config")
    ai_toggle.add_argument("--disable", dest="setup_ai_enabled", action="store_false", help="Disable AI in config")
    remote_toggle = ai_setup.add_mutually_exclusive_group()
    remote_toggle.add_argument("--allow-remote", dest="setup_allow_remote", action="store_true", default=None, help="Allow non-loopback AI base URLs")
    remote_toggle.add_argument("--local-only", dest="setup_allow_remote", action="store_false", help="Require a local loopback AI base URL")
    ai_setup.add_argument("--skip-connection-test", action="store_true", help="Skip Ollama connectivity checks")
    ai_setup.set_defaults(handler=_handle_ai_setup)

    ai_models = ai_sub.add_parser("models", help="List Gemma registry entries and installed Ollama models")
    ai_models.set_defaults(handler=_handle_ai_models)

    ai_pull = ai_sub.add_parser("pull", help="Pull an Ollama model with streamed progress")
    ai_pull.add_argument("model", nargs="?", default=None, help="Model tag to pull")
    ai_pull.set_defaults(handler=_handle_ai_pull)

    ai_test = ai_sub.add_parser("test", help="Run a short provider health prompt")
    ai_test.set_defaults(handler=_handle_ai_test)

    ai_ask = ai_sub.add_parser("ask", help="Ask the AI investigator a question with cited evidence")
    ai_ask.add_argument("question", help="Free-form question, e.g. \"what should I inspect first?\"")
    ai_ask.add_argument("--scope", default="overview", help="Investigation scope (overview, incident:<id>, service:<id>)")
    ai_ask.add_argument("--mode", choices=_EXPERIENCE_MODES, default=None, help="Override the response density mode")
    ai_ask.set_defaults(handler=_handle_ai_ask)

    ai_investigate = ai_sub.add_parser("investigate", help="Run a structured AI investigation on the latest incident")
    ai_investigate.add_argument("target", nargs="?", default="latest", help="latest | incident <id> | service <id>")
    ai_investigate.add_argument("identifier", nargs="?", default=None, help="Optional id when target is 'incident' or 'service'")
    ai_investigate.add_argument("--mode", choices=_EXPERIENCE_MODES, default=None)
    ai_investigate.set_defaults(handler=_handle_ai_investigate)

    ai_report = ai_sub.add_parser("report", help="Produce an operator/developer investigation report for an incident")
    ai_report.add_argument("incident_id")
    ai_report.add_argument("--mode", choices=_EXPERIENCE_MODES, default="operator")
    ai_report.set_defaults(handler=_handle_ai_report)

    ai_trace = ai_sub.add_parser("trace", help="Show the most recent AI prompt trace for an incident")
    ai_trace.add_argument("incident_id")
    ai_trace.set_defaults(handler=_handle_ai_trace)

    ai_doctor = ai_sub.add_parser("doctor", help="Inspect AI provider readiness, redaction policy, and remote risk")
    ai_doctor.set_defaults(handler=_handle_ai_doctor)

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

    service = sub.add_parser("service", help="Manage the Windows Inferra service from the main CLI")
    _add_shared_command_options(service)
    service_sub = service.add_subparsers(dest="service_command")

    service_status = service_sub.add_parser("status", help="Show Windows service state, runtime config, and log path")
    service_status.set_defaults(handler=_handle_service_status)

    service_install = service_sub.add_parser("install", help="Install the Windows service using the current config path")
    service_install.add_argument("--startup", choices=("auto", "manual", "delayed"), default="auto")
    service_install.set_defaults(handler=_handle_service_install)

    for verb in ("start", "stop", "restart", "remove"):
        service_cmd = service_sub.add_parser(verb, help=f"{verb.capitalize()} the Windows Inferra service")
        service_cmd.set_defaults(handler=_handle_service_control, service_verb=verb)

    service_repair = service_sub.add_parser(
        "repair",
        help="Inspect Inferra service prerequisites and report safe next steps",
    )
    service_repair.set_defaults(handler=_handle_service_repair)

    mode = sub.add_parser("mode", help="Show or switch operator/developer control-plane mode")
    _add_shared_command_options(mode)
    mode.set_defaults(handler=_handle_mode_show)
    mode_sub = mode.add_subparsers(dest="mode_command")

    mode_show = mode_sub.add_parser("show", help="Show the active control-plane mode")
    mode_show.set_defaults(handler=_handle_mode_show)

    mode_set = mode_sub.add_parser("set", help="Switch the active control-plane mode")
    mode_set.add_argument("value", choices=_EXPERIENCE_MODES)
    mode_set.set_defaults(handler=_handle_mode_set)

    status_cmd = sub.add_parser("status", help="Rich health dashboard (live API, or host snapshot if offline)")
    _add_shared_command_options(status_cmd)
    status_cmd.set_defaults(handler=_handle_status)

    overview_cmd = sub.add_parser(
        "overview",
        help="Quick analysis across incidents, Docker, processes, and detected repos (requires inferra run)",
    )
    _add_shared_command_options(overview_cmd)
    overview_cmd.set_defaults(handler=_handle_overview)

    investigate = sub.add_parser("investigate", help="Drive a read-only investigation flow and suggest safe next steps")
    _add_shared_command_options(investigate)
    investigate_sub = investigate.add_subparsers(dest="investigate_command", required=True)

    investigate_now = investigate_sub.add_parser("now", help="Investigate the current system overview")
    investigate_now.set_defaults(handler=_handle_investigate_now)

    investigate_latest = investigate_sub.add_parser("latest", help="Investigate the highest-priority active incident")
    investigate_latest.set_defaults(handler=_handle_investigate_latest)

    investigate_incident = investigate_sub.add_parser("incident", help="Investigate a specific incident")
    investigate_incident.add_argument("incident_id")
    investigate_incident.set_defaults(handler=_handle_investigate_incident)

    investigate_service = investigate_sub.add_parser("service", help="Investigate a specific service")
    investigate_service.add_argument("service_id")
    investigate_service.set_defaults(handler=_handle_investigate_service)

    investigate_workspace = investigate_sub.add_parser("workspace", help="Inspect local workspace signals")
    investigate_workspace.set_defaults(handler=_handle_workspace)

    incidents = sub.add_parser("incidents", help="Inspect active incidents from the running supervisor")
    _add_shared_command_options(incidents)
    incidents_sub = incidents.add_subparsers(dest="incidents_command")
    incidents.set_defaults(handler=_handle_incidents_list)

    incidents_list = incidents_sub.add_parser("list", help="List active incidents")
    incidents_list.set_defaults(handler=_handle_incidents_list)

    incidents_show = incidents_sub.add_parser("show", help="Show incident evidence and hypotheses")
    incidents_show.add_argument("incident_id")
    incidents_show.set_defaults(handler=_handle_incident_show)

    events = sub.add_parser("events", help="Inspect normalized events from the running supervisor")
    _add_shared_command_options(events)
    events_sub = events.add_subparsers(dest="events_command")
    events.set_defaults(handler=_handle_events_list)

    events_list = events_sub.add_parser("list", help="List recent normalized events")
    events_list.add_argument("--limit", type=int, default=25, help="Maximum events to show")
    events_list.set_defaults(handler=_handle_events_list)

    events_show = events_sub.add_parser("show", help="Show one normalized event")
    events_show.add_argument("event_id")
    events_show.set_defaults(handler=_handle_event_show)

    services = sub.add_parser("services", help="Inspect services from the running supervisor")
    _add_shared_command_options(services)
    services_sub = services.add_subparsers(dest="services_command")
    services.set_defaults(handler=_handle_services_list)

    services_list = services_sub.add_parser("list", help="List observed services")
    services_list.set_defaults(handler=_handle_services_list)

    services_show = services_sub.add_parser("show", help="Show one service")
    services_show.add_argument("service_id")
    services_show.add_argument("--limit", type=int, default=50, help="Maximum related events to include")
    services_show.set_defaults(handler=_handle_service_show)

    services_events = services_sub.add_parser("events", help="List recent events for one service")
    services_events.add_argument("service_id")
    services_events.add_argument("--limit", type=int, default=25, help="Maximum events to show")
    services_events.set_defaults(handler=_handle_service_events)

    workspace_cmd = sub.add_parser("workspace", help="Scan disk for code-project markers (works offline)")
    _add_shared_command_options(workspace_cmd)
    workspace_cmd.set_defaults(handler=_handle_workspace)
    workspace_sub = workspace_cmd.add_subparsers(dest="workspace_command")

    workspace_scan = workspace_sub.add_parser("scan", help="Scan local disk for code projects (default action)")
    workspace_scan.set_defaults(handler=_handle_workspace)

    workspace_map = workspace_sub.add_parser("map", help="Show service-to-project mappings with confidence (requires inferra serve)")
    workspace_map.set_defaults(handler=_handle_workspace_map)

    workspace_services = workspace_sub.add_parser("services", help="Show service mapping coverage (requires inferra serve)")
    workspace_services.set_defaults(handler=_handle_workspace_services)

    workspace_inspect = workspace_sub.add_parser("inspect", help="Inspect markers and likely commands for one project path")
    workspace_inspect.add_argument("path")
    workspace_inspect.set_defaults(handler=_handle_workspace_inspect)

    demo = sub.add_parser("demo", help="Demo data and onboarding helpers")
    _add_shared_command_options(demo)
    demo_sub = demo.add_subparsers(dest="demo_command", required=True)
    demo_seed = demo_sub.add_parser("seed", help="Seed demo events into the local database for first-run review")
    demo_seed.add_argument("--service", default="api", help="Demo service id to attach events to")
    demo_seed.add_argument("--count", type=int, default=8, help="Number of demo events to insert")
    demo_seed.set_defaults(handler=_handle_demo_seed)
    demo_clear = demo_sub.add_parser("clear", help="Remove demo-tagged events and incidents from the local database")
    demo_clear.set_defaults(handler=_handle_demo_clear)

    doctor_cmd = sub.add_parser("doctor", help="Run local readiness checks and suggest safe next steps")
    _add_shared_command_options(doctor_cmd)
    doctor_cmd.add_argument(
        "--release",
        action="store_true",
        help="Also run repository release-readiness checks for docs, UI packaging, and dropped artifacts",
    )
    doctor_cmd.set_defaults(handler=_handle_doctor)

    completion = sub.add_parser("completion", help="Generate shell completion for bash, zsh, fish, or PowerShell")
    completion.add_argument("shell", choices=("bash", "zsh", "fish", "powershell"))
    completion.set_defaults(handler=_handle_completion)
    return parser


def _add_setup_options(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--yes", action="store_true", help="Accept defaults and skip interactive prompts")
    parser.add_argument("--preset", choices=PRESET_NAMES, default=None, help="Apply a collector preset during setup")
    parser.add_argument("--mode", choices=_EXPERIENCE_MODES, default=None, help="Set the control-plane experience mode")
    parser.add_argument("--ai-role", choices=_AI_ROLES, default=None, help="Set the primary AI role")
    parser.add_argument("--model", default=None, help="Ollama model tag to configure")
    parser.add_argument("--base-url", default=None, help="Override the Ollama-compatible base URL")
    parser.add_argument("--token-env", default=None, help="Environment variable name for remote Ollama auth")
    ai_toggle = parser.add_mutually_exclusive_group()
    ai_toggle.add_argument("--enable-ai", dest="setup_ai_enabled", action="store_true", default=None, help="Enable AI in the written config")
    ai_toggle.add_argument("--disable-ai", dest="setup_ai_enabled", action="store_false", help="Disable AI in the written config")
    remote_toggle = parser.add_mutually_exclusive_group()
    remote_toggle.add_argument("--allow-remote", dest="setup_allow_remote", action="store_true", default=None, help="Allow non-loopback AI base URLs")
    remote_toggle.add_argument("--local-only", dest="setup_allow_remote", action="store_false", help="Require a local loopback AI base URL")
    parser.add_argument("--skip-connection-test", action="store_true", help="Skip Ollama connectivity checks")


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


_handle_setup = _setup_cmds.handle_setup
_handle_guide = _guide_cmds.handle_guide
_handle_dashboard = _dashboard_cmds.handle_dashboard


async def _handle_run(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    from inferra_legacy.app import InferraRuntime
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


_handle_check_config = _config_cmds.handle_check_config
_handle_config_show = _config_cmds.handle_config_show
_handle_config_get = _config_cmds.handle_config_get
_handle_config_set = _config_cmds.handle_config_set
_handle_config_preset = _config_cmds.handle_config_preset
_handle_mode_show = _config_cmds.handle_mode_show
_handle_mode_set = _config_cmds.handle_mode_set


_handle_collectors_status = _collectors_cmds.handle_collectors_status
_handle_collectors_start = _collectors_cmds.handle_collectors_start
_handle_collectors_stop = _collectors_cmds.handle_collectors_stop


_handle_ai_status = _ai_provider_cmds.handle_ai_status


_handle_ai_setup = _ai_provider_cmds.handle_ai_setup


_handle_ai_models = _ai_provider_cmds.handle_ai_models
_handle_ai_pull = _ai_provider_cmds.handle_ai_pull
_handle_ai_test = _ai_provider_cmds.handle_ai_test


_handle_reason_incident = _reasoning_cmds.handle_reason_incident
_handle_init_db = _storage_cmds.handle_init_db
_handle_storage_verify = _storage_cmds.handle_storage_verify
_handle_storage_vacuum = _storage_cmds.handle_storage_vacuum
_handle_storage_backup = _storage_cmds.handle_storage_backup
_handle_collect_once = _reasoning_cmds.handle_collect_once
_handle_reset_baselines = _reasoning_cmds.handle_reset_baselines
_handle_reset_weights = _reasoning_cmds.handle_reset_weights
_handle_calibration_show = _reasoning_cmds.handle_calibration_show


_handle_service_status = _service_cmds.handle_service_status
_handle_service_install = _service_cmds.handle_service_install
_handle_service_control = _service_cmds.handle_service_control


_handle_status = _incidents_cmds.handle_status
_handle_overview = _incidents_cmds.handle_overview
_handle_incidents_list = _incidents_cmds.handle_incidents_list
_handle_incident_show = _incidents_cmds.handle_incident_show
_handle_events_list = _incidents_cmds.handle_events_list
_handle_event_show = _incidents_cmds.handle_event_show
_handle_services_list = _incidents_cmds.handle_services_list
_handle_service_show = _incidents_cmds.handle_service_show
_handle_service_events = _incidents_cmds.handle_service_events
_handle_investigate_now = _incidents_cmds.handle_investigate_now
_handle_investigate_latest = _incidents_cmds.handle_investigate_latest
_handle_investigate_incident = _incidents_cmds.handle_investigate_incident
_handle_investigate_service = _incidents_cmds.handle_investigate_service


# --- handlers extracted to cli_core.commands.* ----------------------------
# Workspace and AI investigation handlers now live in dedicated modules; we
# expose the public ``_handle_*`` names for argparse registration and existing
# tests that import them by attribute.
_handle_workspace = _workspace_cmds.handle_workspace_scan
_handle_workspace_map = _workspace_cmds.handle_workspace_map
_handle_workspace_services = _workspace_cmds.handle_workspace_services
_handle_workspace_inspect = _workspace_cmds.handle_workspace_inspect
_handle_ai_ask = _ai_cmds.handle_ai_ask
_handle_ai_investigate = _ai_cmds.handle_ai_investigate
_handle_ai_report = _ai_cmds.handle_ai_report
_handle_ai_trace = _ai_cmds.handle_ai_trace
_handle_ai_doctor = _ai_cmds.handle_ai_doctor


_handle_demo_seed = _demo_cmds.handle_demo_seed
_handle_demo_clear = _demo_cmds.handle_demo_clear


_handle_service_repair = _service_cmds.handle_service_repair


def _investigation_output_lines(payload: dict[str, Any]) -> list[str]:
    output = payload.get("output") or {}
    headline = str(output.get("headline") or "(no headline)")
    risk = str(output.get("risk_level") or "low")
    confidence = output.get("confidence", 0)
    lines = [f"risk={risk} confidence={confidence}", headline]
    for entry in output.get("what_happened") or []:
        lines.append(f"- {entry}")
    if output.get("why_it_matters"):
        lines.append("Why it matters:")
        lines.extend(f"  - {item}" for item in output["why_it_matters"])
    if output.get("likely_causes"):
        lines.append("Likely causes:")
        lines.extend(f"  - {item}" for item in output["likely_causes"])
    next_steps = output.get("next_steps") or []
    if next_steps:
        lines.append("Safe next steps:")
        for step in next_steps[:8]:
            command = step.get("command") or ""
            title = step.get("title") or "next step"
            lines.append(f"  - {title}" + (f" -> {command}" if command else ""))
    if output.get("uncertainty"):
        lines.append("Uncertainty:")
        lines.extend(f"  - {item}" for item in output["uncertainty"])
    if not payload.get("used_ai", True):
        reason = str(payload.get("fallback_reason") or "")
        lines.append(f"AI fallback: {reason}" if reason else "AI fallback used (deterministic).")
    provider = payload.get("provider") or {}
    if provider:
        lines.append(
            f"provider: enabled={provider.get('enabled')} available={provider.get('available')} "
            f"model={provider.get('model')} allow_remote={provider.get('allow_remote')}"
        )
    return lines


_handle_doctor = _service_cmds.handle_doctor
_handle_completion = _service_cmds.handle_completion


def _experience_payload(config: Any) -> dict[str, Any]:
    return {
        "mode": config.experience.mode,
        "ai_role": config.experience.ai_role,
        "suggest_safe_actions": config.experience.suggest_safe_actions,
        "execute_actions": config.experience.execute_actions,
        "show_raw_evidence_by_default": config.experience.show_raw_evidence_by_default,
    }


def _cli_limit(value: Any, *, maximum: int) -> int:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        parsed = maximum
    return max(1, min(parsed, maximum))


def _severity_label(value: Any) -> str:
    labels = {0: "debug", 1: "info", 2: "warn", 3: "error", 4: "critical"}
    try:
        return labels.get(int(value), str(value))
    except (TypeError, ValueError):
        return str(value or "unknown")


def _format_incident_line(item: dict[str, Any]) -> str:
    return (
        f"{item.get('incident_id', '?')} state={item.get('state', '?')} "
        f"severity={_severity_label(item.get('severity'))} service={item.get('primary_service') or 'unknown'} "
        f"events={len(item.get('events') or [])}"
    )


def _format_event_line(item: dict[str, Any]) -> str:
    message = str(item.get("message") or "").replace("\n", " ")
    if len(message) > 120:
        message = message[:117] + "..."
    return (
        f"{item.get('event_id', '?')} {item.get('timestamp', '?')} "
        f"{_severity_label(item.get('severity'))} {item.get('service_id', 'unknown')}: {message}"
    )


def _format_service_line(item: dict[str, Any]) -> str:
    return (
        f"{item.get('service_id', '?')} status={item.get('status', 'unknown')} "
        f"events={item.get('event_count', 0)} errors={item.get('error_count', 0)} "
        f"last={item.get('last_event_at') or '?'}"
    )


def _investigation_from_overview(command: str, config_path: str, overview: dict[str, Any]) -> dict[str, Any]:
    quick = overview.get("quick_analysis") or {}
    dashboard = overview.get("dashboard") or {}
    incidents = list(dashboard.get("incidents") or [])
    services = list(dashboard.get("services") or [])
    risky_services = [item for item in services if item.get("status") in {"critical", "degraded", "elevated"}]
    summary = str(quick.get("headline") or "No overview headline available.")
    return {
        "command": command,
        "config_path": config_path,
        "focus": "overview",
        "summary": summary,
        "priority": "high" if quick.get("risk_level") == "high" else "normal",
        "evidence": {
            "active_incident_count": len(incidents),
            "risky_services": risky_services[:10],
            "top_incident": incidents[0] if incidents else None,
            "mode": quick.get("mode"),
            "ai_role": quick.get("ai_role"),
        },
        "safe_next_steps": _safe_next_steps_for_overview(incidents, risky_services),
    }


def _investigation_lines(payload: dict[str, Any]) -> list[str]:
    lines = [
        f"priority={payload.get('priority', 'normal')}",
        str(payload.get("summary") or "No summary available."),
    ]
    evidence = payload.get("evidence") or {}
    if evidence.get("top_incident"):
        lines.append("top_incident: " + _format_incident_line(evidence["top_incident"]))
    for service in evidence.get("risky_services") or []:
        lines.append("service: " + _format_service_line(service))
    for event in evidence.get("sample_events") or []:
        lines.append("event: " + _format_event_line(event))
    lines.extend(payload.get("safe_next_steps") or [])
    return lines


def _safe_next_steps_for_overview(incidents: list[dict[str, Any]], services: list[dict[str, Any]]) -> list[str]:
    steps = ["Safe next steps:"]
    if incidents:
        incident_id = incidents[0].get("incident_id")
        steps.append(f"inferra investigate incident {incident_id}")
        steps.append(f"inferra incidents show {incident_id}")
    elif services:
        service_id = services[0].get("service_id")
        steps.append(f"inferra investigate service {service_id}")
        steps.append(f"inferra services show {service_id}")
    else:
        steps.append("inferra events list --limit 25")
        steps.append("inferra collectors status")
    return steps


def _safe_next_steps_for_incident(
    incident_id: str,
    incident: dict[str, Any],
    hypotheses: list[dict[str, Any]],
) -> list[str]:
    steps = ["Safe next steps:"]
    primary_service = incident.get("primary_service")
    if primary_service:
        steps.append(f"inferra services show {primary_service}")
        steps.append(f"inferra services events {primary_service} --limit 25")
    steps.append(f"inferra reason-incident {incident_id}")
    if hypotheses:
        steps.append("Review supporting and contradicting evidence before changing the observed system.")
    else:
        steps.append("No hypotheses recorded yet; keep collectors running and re-check the incident.")
    return steps


def _safe_next_steps_for_service(
    service_id: str,
    service: dict[str, Any],
    incidents: list[dict[str, Any]],
) -> list[str]:
    steps = ["Safe next steps:"]
    if incidents:
        incident_id = incidents[0].get("incident_id")
        steps.append(f"inferra investigate incident {incident_id}")
        steps.append(f"inferra incidents show {incident_id}")
    steps.append(f"inferra services events {service_id} --limit 25")
    if service.get("status") in {"critical", "degraded"}:
        steps.append("Inspect recent errors and dependencies before taking any manual action.")
    else:
        steps.append("No unsafe action suggested; continue observing or narrow by recent events.")
    return steps


def _doctor_next_steps(report: dict[str, Any], api_status: dict[str, Any]) -> list[str]:
    steps = ["Safe next steps:"]
    if report.get("errors"):
        steps.append("Fix config errors shown above, then run `inferra doctor` again.")
    elif not api_status.get("reachable"):
        steps.append("Start the supervisor with `inferra serve` or `inferra service start`.")
    elif api_status.get("degraded"):
        steps.append("Run `inferra investigate now` to prioritize the degraded area.")
    else:
        steps.append("Run `inferra overview` or `inferra investigate latest` for the current operating picture.")
    return steps


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
        "mode": config.experience.mode,
        "ai_role": config.experience.ai_role,
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
    print(f"Current mode: {config.experience.mode}")
    print(f"Current AI role: {config.experience.ai_role}")
    print(f"Current AI enabled: {config.ai.enabled}")
    print(f"Current AI model: {config.ai.model}")
    print(f"Current AI base_url: {config.ai.base_url}")
    data_dir = _prompt_value("Storage data_dir", str(config.storage.data_dir))
    mode = _prompt_choice("Control-plane mode", config.experience.mode, _EXPERIENCE_MODES)
    ai_role = _prompt_choice("Primary AI role", config.experience.ai_role, _AI_ROLES)
    preset = _prompt_value("Collector preset (blank to keep current)", "")
    ai_enabled = _prompt_yes_no("Enable AI explanations?", default=config.ai.enabled)
    model = _prompt_value("Ollama model", config.ai.model)
    base_url = _prompt_value("Ollama base URL", config.ai.base_url)
    allow_remote = _prompt_yes_no("Allow non-loopback AI base URL?", default=config.ai.allow_remote)
    run_probe = not skip_connection_test and _prompt_yes_no("Probe Ollama connection now?", default=True)
    if not _prompt_yes_no("Continue with setup?", default=True):
        raise CommandError("Setup cancelled.")
    updated = replace(
        config,
        storage=replace(config.storage, data_dir=Path(data_dir)),
        experience=replace(
            config.experience,
            mode=mode,
            ai_role=ai_role,
            show_raw_evidence_by_default=mode in {"expert", "developer"},
        ),
        ai=replace(config.ai, enabled=ai_enabled, provider="ollama", model=model, base_url=base_url, allow_remote=allow_remote),
    )
    if preset:
        try:
            updated = apply_preset(updated, preset)
        except ValueError as exc:
            raise CommandError(str(exc)) from exc
    return updated, not run_probe


def _prompt_value(label: str, default: str) -> str:
    response = input(f"{label} [{default}]: ").strip()
    return response or default


def _prompt_choice(label: str, default: str, choices: tuple[str, ...]) -> str:
    raw = _prompt_value(f"{label} ({'/'.join(choices)})", default)
    if raw not in choices:
        raise CommandError(f"{label} must be one of: {', '.join(choices)}")
    return raw


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


def _apply_setup_overrides(config: Any, args: argparse.Namespace) -> Any:
    updated = config
    preset = getattr(args, "preset", None)
    if preset:
        try:
            updated = apply_preset(updated, preset)
        except ValueError as exc:
            raise CommandError(str(exc)) from exc
    if getattr(args, "data_dir", None) is not None:
        updated = replace(updated, storage=replace(updated.storage, data_dir=Path(args.data_dir)))
    mode = getattr(args, "mode", None)
    ai_role = getattr(args, "ai_role", None)
    experience = updated.experience
    if mode is not None:
        experience = replace(experience, mode=mode, show_raw_evidence_by_default=mode in {"expert", "developer"})
    if ai_role is not None:
        experience = replace(experience, ai_role=ai_role)

    ai_enabled = getattr(args, "setup_ai_enabled", None)
    ai_allow_remote = getattr(args, "setup_allow_remote", None)
    ai = updated.ai
    if ai_enabled is not None:
        ai = replace(ai, enabled=bool(ai_enabled))
    if getattr(args, "model", None) is not None:
        ai = replace(ai, model=str(args.model).strip())
    if getattr(args, "base_url", None) is not None:
        ai = replace(ai, base_url=str(args.base_url).strip())
    if getattr(args, "token_env", None) is not None:
        ai = replace(ai, token_env=str(args.token_env).strip())
    if ai_allow_remote is not None:
        ai = replace(ai, allow_remote=bool(ai_allow_remote))
    return replace(updated, ai=ai, experience=experience)


def _onboarding_next_steps(config_path: Path, config: Any, connection_test: dict[str, Any]) -> list[str]:
    quoted = str(config_path)
    steps = [
        f'inferra --config "{quoted}" check-config',
        f'inferra --config "{quoted}" serve',
        f'inferra --config "{quoted}" status',
    ]
    if config.ai.enabled:
        steps.insert(1, f'inferra --config "{quoted}" ai status')
        if not connection_test.get("skipped") and not connection_test.get("available", False):
            steps.insert(2, f'inferra --config "{quoted}" ai pull {config.ai.model}')
        else:
            steps.insert(2, f'inferra --config "{quoted}" ai test')
    else:
        steps.insert(1, f'inferra --config "{quoted}" ai setup --enable --model {config.ai.model}')
    if getattr(config.collectors, "auto_start", False):
        steps.append(f'inferra --config "{quoted}" collectors status')
    if platform.system().lower() == "windows":
        steps.append(f'inferra --config "{quoted}" service install --startup auto')
    return steps


def _require_windows_service_support() -> None:
    if platform.system().lower() != "windows":
        raise CommandError("Windows service management is only available on Windows.")


def _build_windows_service_command(
    verb: str,
    config_path: Path,
    data_dir: Path | None,
    *,
    startup: str | None = None,
) -> list[str]:
    if getattr(sys, "frozen", False):
        command = [sys.executable, verb]
    else:
        command = [sys.executable, "-m", "inferra_legacy.windows_service", verb]
    if startup:
        command.extend(["--startup", startup])
    command.extend(["--config", str(config_path)])
    if data_dir is not None:
        command.extend(["--data-dir", str(data_dir)])
    return command


def _run_subprocess_capture(command: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(command, capture_output=True, text=True, check=False)


def _subprocess_failure_message(label: str, completed: subprocess.CompletedProcess[str]) -> str:
    stderr = (completed.stderr or "").strip()
    stdout = (completed.stdout or "").strip()
    detail = stderr or stdout or f"exit code {completed.returncode}"
    return f"{label} failed: {detail}"


def _parse_sc_state(output: str) -> str | None:
    for line in output.splitlines():
        if "STATE" not in line:
            continue
        _, _, tail = line.partition(":")
        parts = tail.strip().split()
        if len(parts) >= 2:
            return parts[1].lower()
        if parts:
            return parts[0].lower()
    return None


def _parse_sc_start_type(output: str) -> str | None:
    for line in output.splitlines():
        if "START_TYPE" not in line:
            continue
        _, _, tail = line.partition(":")
        parts = tail.strip().split()
        if len(parts) >= 2:
            return parts[1].lower()
        if parts:
            return parts[0].lower()
    return None


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

    pyproject = _REPO_ROOT / "pyproject.toml"
    if pyproject.exists():
        data = tomllib.loads(pyproject.read_text(encoding="utf-8"))
        return str(data.get("project", {}).get("version", "0.1.0"))
    try:
        return version("inferra")
    except PackageNotFoundError:
        return "0.1.0"


if __name__ == "__main__":
    raise SystemExit(main())
