"""Storage commands: init-db, verify, vacuum, backup."""

from __future__ import annotations

import argparse
from pathlib import Path
from typing import Any

from cli_core.result import CommandResult


async def handle_init_db(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    from storage.migrations import CURRENT_SCHEMA_VERSION, integrity_check, migrate

    _config_path_for_logging, config = cli._load_config_for_command(args)
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
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def handle_storage_verify(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    from storage.migrations import integrity_check

    _config_path_for_logging, config = cli._load_config_for_command(args)
    results: list[dict[str, Any]] = []
    stdout_lines: list[str] = []
    stderr_lines: list[str] = []
    exit_code = 0
    for db_path in cli._database_paths(config):
        row: dict[str, Any] = {"path": str(db_path), "name": db_path.name, "exists": db_path.exists()}
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
    return cli._emit_result(
        args,
        CommandResult(
            payload=payload,
            stdout_lines=stdout_lines,
            stderr_lines=stderr_lines,
            exit_code=exit_code,
        ),
    )


async def handle_storage_vacuum(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    from storage.migrations import vacuum_db

    _config_path_for_logging, config = cli._load_config_for_command(args)
    results: list[dict[str, Any]] = []
    stdout_lines: list[str] = []
    for db_path in cli._database_paths(config):
        if not db_path.exists():
            results.append({"path": str(db_path), "name": db_path.name, "status": "missing"})
            continue
        vacuum_db(db_path)
        results.append({"path": str(db_path), "name": db_path.name, "status": "vacuumed"})
        stdout_lines.append(f"Vacuumed {db_path.name}")
    payload = {"command": "storage vacuum", "databases": results}
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def handle_storage_backup(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    from storage.migrations import backup_db

    _config_path_for_logging, config = cli._load_config_for_command(args)
    dest_dir = Path(args.path)
    dest_dir.mkdir(parents=True, exist_ok=True)
    results: list[dict[str, Any]] = []
    stdout_lines: list[str] = []
    for db_path in cli._database_paths(config):
        if not db_path.exists():
            results.append({"path": str(db_path), "name": db_path.name, "status": "missing"})
            continue
        dest_path = backup_db(db_path, dest_dir / db_path.name)
        results.append({"path": str(db_path), "backup_path": str(dest_path), "status": "backed_up"})
        stdout_lines.append(f"Backed up {db_path.name} -> {dest_path}")
    payload = {
        "command": "storage backup",
        "destination": str(dest_dir),
        "databases": results,
    }
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))
