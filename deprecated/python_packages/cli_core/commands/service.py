"""Windows service management commands and the platform-agnostic
service repair / doctor commands.
"""

from __future__ import annotations

import argparse
import importlib.metadata
import os
import platform
import sys
from pathlib import Path
from typing import Any

from cli_core.result import CommandError, CommandResult


def _repo_root() -> Path:
    return Path(__file__).resolve().parents[3]


def build_release_readiness(root: Path | None = None) -> dict[str, Any]:
    """Check repository polish without mutating files or requiring services."""

    root = (root or _repo_root()).resolve()
    checks: list[dict[str, Any]] = []
    warnings: list[dict[str, str]] = []

    def add_check(name: str, ok: bool, message: str, *, path: str | None = None) -> None:
        item: dict[str, Any] = {"name": name, "ok": ok, "message": message}
        if path is not None:
            item["path"] = path
        checks.append(item)

    def exists(rel: str) -> bool:
        return (root / rel).exists()

    required_paths = {
        "README product entry": "README.md",
        "MkDocs config": "mkdocs.yml",
        "Docs home": "docs/index.md",
        "Dossier home": "docs/dossier/README.md",
        "React frontend source": "src/web/frontend/package.json",
        "React lockfile": "src/web/frontend/package-lock.json",
        "Windows web build script": "scripts/build-web.ps1",
        "Unix web build script": "scripts/build-web.sh",
        "CLI command modules": "src/cli_core/commands",
    }
    for name, rel in required_paths.items():
        add_check(name, exists(rel), f"{rel} {'exists' if exists(rel) else 'is missing'}", path=rel)

    forbidden_paths = {
        "Duplicate top-level webui": "webui",
        "Old static web source": "src/web/static",
        "Frontend dependency folder": "src/web/frontend/node_modules",
        "Root dependency folder": "node_modules",
        "TypeScript build cache": "src/web/frontend/tsconfig.tsbuildinfo",
        "Committed web UI build output": "src/web/ui_dist",
        "MkDocs site build output": "site",
        "Dropped version artifact": "1.41",
        "Duplicate top-level planning scratch": "planning",
        "Python coverage artifact": ".coverage",
        "Python package metadata artifact": "inferra.egg-info",
        "Nested package metadata artifact": "src/inferra.egg-info",
        "Build directory artifact": "build",
    }
    for name, rel in forbidden_paths.items():
        present = exists(rel)
        add_check(name, not present, f"{rel} {'is present' if present else 'is absent'}", path=rel)

    ignored_artifacts = ["dist", ".pytest_cache", ".ruff_cache", ".ruff_cache_tmp", ".venv-inferra-build", "data", "src/web/ui_dist", "site"]
    for rel in ignored_artifacts:
        if exists(rel):
            warnings.append(
                {
                    "name": "ignored_artifact_present",
                    "path": rel,
                    "message": f"{rel} is present locally; it should stay ignored and not be committed.",
                }
            )

    readme = root / "README.md"
    if readme.exists():
        text = readme.read_text(encoding="utf-8", errors="replace").lower()
        has_identity = "runtime intelligence control plane" in text and "operator" in text and "developer" in text
        add_check(
            "README product identity",
            has_identity,
            "README describes the control-plane identity and experience modes"
            if has_identity
            else "README is missing the current product identity or mode language",
            path="README.md",
        )

    pyproject = root / "pyproject.toml"
    if pyproject.exists():
        try:
            import tomllib
        except ModuleNotFoundError:  # pragma: no cover - Python <3.11
            import tomli as tomllib  # type: ignore[no-redef]

        try:
            metadata = tomllib.loads(pyproject.read_text(encoding="utf-8"))
        except Exception as exc:
            add_check("Package metadata parse", False, f"pyproject.toml could not be parsed: {exc}", path="pyproject.toml")
        else:
            project = metadata.get("project", {})
            scripts = project.get("scripts", {})
            description = str(project.get("description", "")).lower()
            add_check(
                "Package script",
                scripts.get("inferra-python-legacy") == "inferra_legacy.cli:main" and "inferra" not in scripts,
                "Python package exposes only the legacy compatibility script",
                path="pyproject.toml",
            )
            add_check(
                "Package product description",
                "runtime intelligence control plane" in description,
                "pyproject description uses current product identity",
                path="pyproject.toml",
            )

    try:
        import inferra_legacy.cli as _cli

        version_value = _cli._project_version()
    except Exception as exc:  # pragma: no cover - environment dependent
        add_check("CLI version resolution", False, f"CLI version could not be resolved: {exc}")
    else:
        add_check("CLI version resolution", bool(version_value), f"inferra --version resolves to {version_value}")

    for package in ("fastapi", "uvicorn", "pydantic"):
        try:
            resolved = importlib.metadata.version(package)
        except importlib.metadata.PackageNotFoundError:
            add_check(f"Runtime dependency {package}", False, f"{package} is not installed")
        else:
            add_check(f"Runtime dependency {package}", True, f"{package} {resolved} is installed")

    if sys.platform.startswith("win"):
        try:
            import win32serviceutil  # type: ignore[import-not-found]  # noqa: F401
        except Exception as exc:  # pragma: no cover - environment dependent
            add_check("Windows service dependency", False, f"pywin32 unavailable: {exc}")
        else:
            add_check("Windows service dependency", True, "pywin32 is available")

    git_dir = root / ".git"
    if git_dir.exists():
        try:
            import subprocess

            completed = subprocess.run(
                ["git", "diff", "--cached", "--name-status", "--", "site", "perf_report.json"],
                cwd=root,
                check=False,
                capture_output=True,
                text=True,
                timeout=5,
            )
            staged_generated = [line.strip() for line in completed.stdout.splitlines() if line.strip()]
        except Exception as exc:  # pragma: no cover - environment dependent
            warnings.append(
                {
                    "name": "git_status_unavailable",
                    "path": ".git",
                    "message": f"Could not inspect staged generated artifacts: {exc}",
                }
            )
        else:
            staged_deletions = [line for line in staged_generated if line.startswith("D")]
            add_check(
                "Generated artifact staging",
                not staged_deletions,
                "No staged generated artifact deletions"
                if not staged_deletions
                else "Generated artifacts have staged deletions: " + "; ".join(staged_deletions[:5]),
                path="site",
            )
            if staged_generated and not staged_deletions:
                warnings.append(
                    {
                        "name": "generated_artifacts_staged",
                        "path": "site",
                        "message": "Generated docs/perf artifacts are staged; confirm this is intentional.",
                    }
                )

    ok = all(item["ok"] for item in checks)
    return {"ok": ok, "checks": checks, "warnings": warnings, "root": str(root)}


async def handle_service_status(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    if platform.system().lower() != "windows":
        payload = {
            "command": "service status",
            "supported": False,
            "reason": "Windows service management is only available on Windows.",
        }
        return cli._emit_result(
            args,
            CommandResult(
                payload=payload,
                stdout_lines=["Windows service management is only available on Windows."],
                exit_code=1,
            ),
        )

    import inferra_legacy.windows_service as windows_service

    runtime = windows_service.read_service_runtime()
    query = cli._run_subprocess_capture(["sc.exe", "query", "Inferra"])
    installed = query.returncode == 0
    qc = cli._run_subprocess_capture(["sc.exe", "qc", "Inferra"]) if installed else None
    payload = {
        "command": "service status",
        "supported": True,
        "service_name": "Inferra",
        "installed": installed,
        "state": cli._parse_sc_state(query.stdout) if installed else None,
        "startup": cli._parse_sc_start_type(qc.stdout) if qc is not None and qc.returncode == 0 else None,
        "config_path": str(runtime.config_path) if runtime is not None else None,
        "data_dir": str(runtime.data_dir) if runtime is not None and runtime.data_dir is not None else None,
        "log_path": str(windows_service.serve_log_path()),
    }
    stdout_lines = [
        f"service=Inferra installed={payload['installed']} state={payload.get('state') or 'not_installed'}",
        f"log_path={payload['log_path']}",
    ]
    if payload.get("config_path"):
        stdout_lines.append(f"config_path={payload['config_path']}")
    if payload.get("data_dir"):
        stdout_lines.append(f"data_dir={payload['data_dir']}")
    if payload.get("startup"):
        stdout_lines.append(f"startup={payload['startup']}")
    stderr_lines: list[str] = []
    if not installed and query.stderr.strip():
        stderr_lines.append(query.stderr.strip())
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=stdout_lines, stderr_lines=stderr_lines),
    )


async def handle_service_install(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    cli._require_windows_service_support()
    config_path = cli._config_path(args)
    data_dir = Path(args.data_dir) if getattr(args, "data_dir", None) else None
    command = cli._build_windows_service_command("install", config_path, data_dir, startup=args.startup)
    completed = cli._run_subprocess_capture(command)
    if completed.returncode != 0:
        raise CommandError(cli._subprocess_failure_message("service install", completed))
    payload = {
        "command": "service install",
        "service_name": "Inferra",
        "config_path": str(config_path),
        "data_dir": str(data_dir) if data_dir is not None else None,
        "startup": args.startup,
        "invocation": command,
    }
    stdout_lines = [
        f"Installed Windows service Inferra (startup={args.startup})",
        f"config_path={config_path}",
    ]
    if data_dir is not None:
        stdout_lines.append(f"data_dir={data_dir}")
    stdout_lines.append("Next: inferra service start")
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def handle_service_control(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    cli._require_windows_service_support()
    config_path = cli._config_path(args)
    data_dir = Path(args.data_dir) if getattr(args, "data_dir", None) else None
    verb = str(getattr(args, "service_verb", "")).strip().lower()
    command = cli._build_windows_service_command(verb, config_path, data_dir)
    completed = cli._run_subprocess_capture(command)
    if completed.returncode != 0:
        raise CommandError(cli._subprocess_failure_message(f"service {verb}", completed))
    payload = {
        "command": f"service {verb}",
        "service_name": "Inferra",
        "config_path": str(config_path),
        "invocation": command,
    }
    action = {
        "start": "Started",
        "stop": "Stopped",
        "restart": "Restarted",
        "remove": "Removed",
    }.get(verb, verb.capitalize())
    stdout_lines = [f"{action} Windows service Inferra"]
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def handle_service_repair(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    findings: list[dict[str, Any]] = []
    next_steps: list[str] = []

    if not config_path.exists():
        findings.append({"name": "config_path", "ok": False, "message": f"Config not found at {config_path}"})
        next_steps.append(f"inferra setup --config \"{config_path}\" --yes")
    else:
        findings.append({"name": "config_path", "ok": True, "message": f"Config present at {config_path}"})

    data_dir = Path(config.storage.data_dir)
    if data_dir.exists() and os.access(data_dir, os.W_OK):
        findings.append({"name": "data_dir", "ok": True, "message": f"Data dir writable: {data_dir}"})
    else:
        findings.append({"name": "data_dir", "ok": False, "message": f"Data dir missing or not writable: {data_dir}"})
        next_steps.append("inferra init-db")

    bind = (config.server.host, config.server.port)
    bind_ok = True
    bind_msg = f"Bind candidate {bind[0]}:{bind[1]}"
    try:
        import socket as _socket

        with _socket.socket(_socket.AF_INET, _socket.SOCK_STREAM) as probe:
            probe.settimeout(0.25)
            probe.bind((config.server.host, 0))
    except OSError as exc:
        bind_ok = False
        bind_msg = f"Cannot bind {config.server.host}: {exc}"
    findings.append({"name": "bind", "ok": bind_ok, "message": bind_msg})

    pywin32_ok = True
    pywin32_msg = "Not applicable on this platform"
    if platform.system().lower() == "windows":
        try:
            import win32serviceutil  # type: ignore[import-not-found]  # noqa: F401
        except Exception as exc:  # pragma: no cover - environment dependent
            pywin32_ok = False
            pywin32_msg = f"pywin32 unavailable: {exc}"
            next_steps.append("pip install pywin32")
        else:
            pywin32_msg = "pywin32 available"
        try:
            import inferra_legacy.windows_service as _ws

            log_path = _ws.serve_log_path()
            if log_path:
                findings.append(
                    {"name": "log_path", "ok": True, "message": f"Service log path: {log_path}"}
                )
        except Exception:
            pass
    findings.append({"name": "pywin32", "ok": pywin32_ok, "message": pywin32_msg})

    ok = all(
        item["ok"]
        for item in findings
        if item["name"] != "pywin32" or platform.system().lower() == "windows"
    )
    if not next_steps:
        next_steps.append("inferra service status")
        next_steps.append("inferra service install --startup auto")

    payload = {
        "command": "service repair",
        "config_path": str(config_path),
        "ok": ok,
        "findings": findings,
        "safe_next_steps": next_steps,
        "platform": platform.system().lower(),
    }
    lines = [f"service_repair_ok={ok}"]
    for finding in findings:
        marker = "OK  " if finding["ok"] else "WARN"
        lines.append(f"{marker} {finding['name']}: {finding['message']}")
    lines.extend(f"Next: {step}" for step in next_steps)
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=lines, exit_code=0 if ok else 1),
    )


async def handle_doctor(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    report = await cli._build_check_report(config_path, config=config)
    api_status: dict[str, Any] = {"reachable": False}
    collectors_status: dict[str, Any] = {"reachable": False}
    try:
        health = await cli._local_api_json(config, "GET", "/api/health")
        api_status = {
            "reachable": True,
            "degraded": bool(health.get("degraded")),
            "active_incidents": int(health.get("active_incidents", 0)),
            "queue_depth": int(health.get("queue_depth", 0)),
            "ai_available": bool(health.get("ai_available")),
        }
        collectors = await cli._local_api_json(config, "GET", "/api/collectors")
        collectors_status = {
            "reachable": True,
            "count": len(collectors.get("collectors") or []),
            "queue_depth": int(collectors.get("queue_depth", 0)),
        }
    except CommandError as exc:
        api_status["reason"] = str(exc)
        collectors_status["reason"] = str(exc)

    next_steps = cli._doctor_next_steps(report, api_status)
    release_report = build_release_readiness() if getattr(args, "release", False) else None
    payload = {
        "command": "doctor",
        "config_path": str(config_path),
        "ok": bool(report.get("ok")) and not report.get("errors"),
        "config_report": report,
        "api": api_status,
        "collectors": collectors_status,
        "safe_next_steps": next_steps,
    }
    if release_report is not None:
        payload["release"] = release_report
        payload["ok"] = bool(payload["ok"]) and bool(release_report["ok"])
    lines = [
        f"config_ok={report.get('ok')}",
        f"api_reachable={api_status.get('reachable')}",
        f"collectors_reachable={collectors_status.get('reachable')}",
    ]
    if release_report is not None:
        lines.append(f"release_ready={release_report['ok']}")
        for item in release_report["checks"]:
            marker = "OK  " if item["ok"] else "WARN"
            lines.append(f"{marker} release:{item['name']}: {item['message']}")
        for item in release_report["warnings"]:
            lines.append(f"note: {item['message']}")
    lines.extend(f"warning: {item['message']}" for item in report.get("warnings", []))
    lines.extend(f"error: {item['message']}" for item in report.get("errors", []))
    lines.extend(next_steps)
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=lines, exit_code=0 if payload["ok"] else 1),
    )


async def handle_completion(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    if cli.argcomplete is None:
        raise CommandError(
            "Shell completion requires `argcomplete`. Install the dev extras or add the dependency."
        )
    script = cli.argcomplete.shellcode(["inferra"], shell=args.shell)
    payload = {"command": "completion", "shell": args.shell, "script": script}
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=[script.rstrip("\n")]))
