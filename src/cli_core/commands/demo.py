"""Demo data handlers: seed and clear synthetic events.

These commands write demo events into the events store under
``source_type = "demo"`` with ``fingerprint`` prefix ``demo-`` so that
``demo clear`` can remove them in isolation.
"""

from __future__ import annotations

import argparse
import sqlite3
from datetime import timedelta
from pathlib import Path

from cli_core.result import CommandResult


async def handle_demo_seed(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli  # late binding for monkeypatch compatibility

    from core.enums import EventType, Severity
    from core.ids import new_id
    from core.time import utc_now
    from events.models import DataQuality, NormalizedEvent, SourceRef
    from storage.event_store import SqliteEventStore
    from storage.migrations import migrate

    config_path, config = cli._load_config_for_command(args)
    data_dir = Path(config.storage.data_dir)
    data_dir.mkdir(parents=True, exist_ok=True)
    events_path = data_dir / config.storage.events_db
    incidents_path = data_dir / config.storage.incidents_db
    migrate(events_path)
    migrate(incidents_path)
    service_id = str(args.service or "api").strip() or "api"
    count = max(1, min(int(args.count or 8), 200))
    now = utc_now()
    samples: tuple[tuple[str, str, tuple[str, ...]], ...] = (
        ("INFO", "request handled in 12ms", ()),
        ("INFO", "cache miss for key user:42 (expected)", ("cache",)),
        ("WARN", "slow database query observed (220ms)", ("slow_query",)),
        ("WARN", "retrying outbound call to redis (attempt 2)", ("retry",)),
        ("ERROR", "connection refused from postgres", ("connection_refused",)),
        ("ERROR", "timeout calling postgres", ("timeout",)),
        ("ERROR", "OOM killed worker process (PID 4012)", ("oom",)),
        ("CRITICAL", "service entered failed state and stopped accepting traffic", ("downtime",)),
    )
    severity_lookup = {
        "DEBUG": Severity.DEBUG,
        "INFO": Severity.INFO,
        "WARN": Severity.WARN,
        "ERROR": Severity.ERROR,
        "CRITICAL": Severity.CRITICAL,
    }
    mmap_bytes = int(config.storage.mmap_size_mb) * 1024 * 1024 if config.storage.enable_mmap else 0
    store = SqliteEventStore(
        events_path,
        batch_size=config.storage.batch_size,
        retention_hours=config.storage.retention_hours,
        prune_interval_seconds=config.storage.prune_interval_seconds,
        wal_mode=config.storage.wal_mode,
        start_pruner=False,
        mmap_size_bytes=mmap_bytes,
    )
    written = 0
    try:
        for index in range(count):
            severity_label, message, sample_tags = samples[index % len(samples)]
            severity_value = severity_lookup[severity_label]
            timestamp = now - timedelta(minutes=count - index)
            event = NormalizedEvent(
                event_id=new_id("evt"),
                timestamp=timestamp,
                timestamp_source="demo",
                service_id=service_id,
                host_id=config.normalization.host_id or "demo-host",
                severity=severity_value,
                event_type=EventType.LOG,
                message=message,
                structured_data={},
                tags=frozenset(("demo", *sample_tags)),
                fingerprint=f"demo-{index:03d}-{severity_label.lower()}",
                quality=DataQuality(1.0, 1.0, 1.0, 1.0, 1.0),
                source_ref=SourceRef(
                    source_type="demo",
                    source_id="inferra-demo",
                    raw_offset=None,
                    collected_at=timestamp,
                ),
            )
            store.add_event(event)
            written += 1
    finally:
        store.close()
    payload = {
        "command": "demo seed",
        "config_path": str(config_path),
        "service_id": service_id,
        "events_written": written,
        "events_db": str(events_path),
        "incidents_db": str(incidents_path),
    }
    lines = [
        f"Inserted {written} demo events for service {service_id} into {events_path}",
        "Run `inferra serve` then `inferra investigate now` to see them in the dashboard.",
    ]
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))


async def handle_demo_clear(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import cli

    config_path, config = cli._load_config_for_command(args)
    data_dir = Path(config.storage.data_dir)
    events_path = data_dir / config.storage.events_db
    if not events_path.exists():
        return cli._emit_result(
            args,
            CommandResult(
                payload={"command": "demo clear", "events_db": str(events_path), "removed": 0},
                stdout_lines=["No events database found; nothing to clear."],
            ),
        )
    removed = 0
    with sqlite3.connect(events_path) as conn:
        cursor = conn.execute(
            "DELETE FROM events WHERE source_type = 'demo' OR fingerprint LIKE 'demo-%'"
        )
        removed = cursor.rowcount or 0
        conn.commit()
    payload = {
        "command": "demo clear",
        "config_path": str(config_path),
        "events_db": str(events_path),
        "removed": int(removed),
    }
    lines = [f"Removed {removed} demo events from {events_path}"]
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))
