"""Reasoning and storage maintenance commands.

Covers reason-incident, reset-baselines, reset-weights, calibration show,
and the platform-aware collect-once helper used by collector subcommands.
"""

from __future__ import annotations

import argparse
import platform
import shutil
from dataclasses import replace
from pathlib import Path
from typing import Any

from cli_core.result import CommandError, CommandResult


async def handle_reason_incident(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from core.time import utc_now
    from reasoning.engine import HypothesisEngine, hypothesis_dict_to_scored
    from runtime.service_graph import ServiceGraph
    from storage.event_store import SqliteEventStore
    from storage.incident_store import SqliteIncidentStore

    config_path, config = cli._load_config_for_command(args)
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
        payloads = engine.generate(
            args.incident_id,
            events,
            incident=incident,
            incident_event_ids=list(incident.events),
        )
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
        return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))
    finally:
        event_store.close()
        incident_store.close()


async def handle_collect_once(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from inferra_legacy.app import InferraRuntime

    expected_platform = args.__dict__.get("platform")
    current_platform = platform.system().lower()
    if expected_platform and current_platform != expected_platform:
        raise CommandError(f"`{args.command}` is only available on {expected_platform}.")

    config_path, config = cli._load_config_for_command(args)
    runtime = InferraRuntime(config)
    try:
        await runtime.start(start_collectors=False)
        try:
            summary = await runtime.collect_source_once(args.source_type)
        except ValueError as exc:
            raise CommandError(
                f"No enabled {args.label} collector is configured. "
                f"Enable `{args.config_key}` or apply a preset."
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
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=stdout_lines))


async def handle_reset_baselines(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    _config_path_for_logging, config = cli._load_config_for_command(args)
    baseline_dir = Path(config.storage.data_dir) / "baselines"
    if baseline_dir.exists():
        shutil.rmtree(baseline_dir)
    baseline_dir.mkdir(parents=True, exist_ok=True)
    payload = {"command": "reset-baselines", "baseline_dir": str(baseline_dir), "deleted": True}
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=[f"Deleted baseline data under {baseline_dir}"]),
    )


async def handle_reset_weights(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from reasoning.scoring import merge_config_weights
    from storage.weight_store import WeightStore, reset_weights

    _config_path_for_logging, config = cli._load_config_for_command(args)
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
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=[f"Reset scoring weights at {path}"]),
    )


async def handle_calibration_show(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from storage.calibration_store import CalibrationStore, check_calibration_staleness

    _config_path_for_logging, config = cli._load_config_for_command(args)
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
    lines = [
        f"Calibration file: {path}",
        f"staleness={stale}",
        f"total_feedback_count={model.total_feedback_count}",
    ]
    for bucket in model.buckets:
        lines.append(
            f"  [{bucket.score_lower},{bucket.score_upper}) n={bucket.total_predictions} "
            f"correct={bucket.correct_predictions} acc={bucket.accuracy:.3f} {bucket.sample_confidence}"
        )
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))
