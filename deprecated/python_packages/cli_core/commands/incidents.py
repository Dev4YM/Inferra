"""Incident, event and service display commands.

These are read-only listings/details that talk to the local API and render
short human summaries plus structured payloads.
"""

from __future__ import annotations

import argparse
from urllib.parse import quote

from cli_core.result import CommandResult


async def handle_incidents_list(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    payload = await cli._require_local_api(config, "GET", "/api/incidents")
    incidents = list(payload.get("incidents") or [])
    result = {"command": "incidents list", "config_path": str(config_path), "incidents": incidents}
    lines = [f"Active incidents: {len(incidents)}"]
    lines.extend(cli._format_incident_line(item) for item in incidents[:50])
    if not incidents:
        lines.append("No active incidents. Try `inferra events list --limit 25` for recent signal.")
    return cli._emit_result(args, CommandResult(payload=result, stdout_lines=lines))


async def handle_incident_show(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    incident_id = quote(str(args.incident_id), safe="")
    payload = await cli._require_local_api(config, "GET", f"/api/incidents/{incident_id}")
    payload["command"] = "incidents show"
    payload["config_path"] = str(config_path)
    incident = payload.get("incident") or {}
    hypotheses = list(payload.get("hypotheses") or [])
    events = list(payload.get("events") or [])
    lines = [
        cli._format_incident_line(incident),
        f"events={len(events)} hypotheses={len(hypotheses)} clusters={len(payload.get('clusters') or [])}",
    ]
    if hypotheses:
        top = hypotheses[0]
        lines.append(
            "top_hypothesis="
            f"{top.get('cause_type', 'unknown')} score={top.get('total_score', '?')} confidence={top.get('confidence', '?')}"
        )
    lines.extend(f"event: {cli._format_event_line(item)}" for item in events[:10])
    lines.extend(cli._safe_next_steps_for_incident(str(args.incident_id), incident, hypotheses))
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))


async def handle_events_list(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    limit = cli._cli_limit(getattr(args, "limit", 25), maximum=500)
    payload = await cli._require_local_api(config, "GET", f"/api/events?limit={limit}")
    events = list(payload.get("events") or [])
    result = {
        "command": "events list",
        "config_path": str(config_path),
        "limit": limit,
        "events": events,
    }
    lines = [f"Recent events: {len(events)}"]
    lines.extend(cli._format_event_line(item) for item in events[-limit:])
    if not events:
        lines.append("No events found. Check collectors with `inferra collectors status`.")
    return cli._emit_result(args, CommandResult(payload=result, stdout_lines=lines))


async def handle_event_show(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    event_id = quote(str(args.event_id), safe="")
    payload = await cli._require_local_api(config, "GET", f"/api/events/{event_id}")
    payload["command"] = "events show"
    payload["config_path"] = str(config_path)
    event = payload.get("event") or {}
    lines = [cli._format_event_line(event), f"source={event.get('source_ref', {}).get('source_type', '?')}"]
    if event.get("tags"):
        lines.append("tags=" + ", ".join(str(tag) for tag in event.get("tags") or []))
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))


async def handle_services_list(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    payload = await cli._require_local_api(config, "GET", "/api/services")
    services = list(payload.get("services") or [])
    result = {"command": "services list", "config_path": str(config_path), "services": services}
    lines = [f"Observed services: {len(services)}"]
    lines.extend(cli._format_service_line(item) for item in services[:80])
    if not services:
        lines.append("No services observed yet. Start collectors or ingest app events.")
    return cli._emit_result(args, CommandResult(payload=result, stdout_lines=lines))


async def handle_service_show(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    service_id = quote(str(args.service_id), safe="")
    limit = cli._cli_limit(getattr(args, "limit", 50), maximum=500)
    payload = await cli._require_local_api(config, "GET", f"/api/services/{service_id}?limit={limit}")
    payload["command"] = "services show"
    payload["config_path"] = str(config_path)
    service = payload.get("service") or {}
    events = list(payload.get("events") or [])
    incidents = list(payload.get("incidents") or [])
    lines = [
        cli._format_service_line(service),
        f"related_events={len(events)} active_incidents={len(incidents)}",
    ]
    lines.extend(f"incident: {cli._format_incident_line(item)}" for item in incidents[:10])
    lines.extend(f"event: {cli._format_event_line(item)}" for item in events[:10])
    lines.extend(cli._safe_next_steps_for_service(str(args.service_id), service, incidents))
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))


async def handle_service_events(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    service_id = quote(str(args.service_id), safe="")
    limit = cli._cli_limit(getattr(args, "limit", 25), maximum=500)
    payload = await cli._require_local_api(config, "GET", f"/api/services/{service_id}/events?limit={limit}")
    events = list(payload.get("events") or [])
    result = {
        "command": "services events",
        "config_path": str(config_path),
        "service_id": args.service_id,
        "limit": limit,
        "events": events,
    }
    lines = [f"Recent events for {args.service_id}: {len(events)}"]
    lines.extend(cli._format_event_line(item) for item in events)
    return cli._emit_result(args, CommandResult(payload=result, stdout_lines=lines))


async def handle_investigate_now(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    overview = await cli._require_local_api(config, "GET", "/api/overview")
    payload = cli._investigation_from_overview("investigate now", str(config_path), overview)
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=cli._investigation_lines(payload)))


async def handle_investigate_latest(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    incidents_payload = await cli._require_local_api(config, "GET", "/api/incidents")
    incidents = sorted(
        list(incidents_payload.get("incidents") or []),
        key=lambda item: (
            int(item.get("severity", 0)),
            str(item.get("updated_at") or item.get("created_at") or ""),
        ),
        reverse=True,
    )
    if not incidents:
        overview = await cli._require_local_api(config, "GET", "/api/overview")
        payload = cli._investigation_from_overview("investigate latest", str(config_path), overview)
        payload["focus"] = "no_active_incident"
        payload["summary"] = "No active incidents. Reviewing the current overview instead."
        return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=cli._investigation_lines(payload)))
    args.incident_id = incidents[0].get("incident_id")
    return await handle_investigate_incident(args, parser)


async def handle_investigate_incident(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    incident_id = quote(str(args.incident_id), safe="")
    detail = await cli._require_local_api(config, "GET", f"/api/incidents/{incident_id}")
    incident = detail.get("incident") or {}
    hypotheses = list(detail.get("hypotheses") or [])
    events = list(detail.get("events") or [])
    top = hypotheses[0] if hypotheses else {}
    summary = (
        f"Incident {args.incident_id} affects {incident.get('primary_service') or 'unknown'} "
        f"with severity {incident.get('severity', '?')}."
    )
    if top:
        summary += f" Top hypothesis: {top.get('cause_type', 'unknown')}."
    payload = {
        "command": "investigate incident",
        "config_path": str(config_path),
        "focus": args.incident_id,
        "summary": summary,
        "priority": "high" if int(incident.get("severity", 0) or 0) >= 3 else "normal",
        "evidence": {
            "incident": incident,
            "top_hypothesis": top,
            "event_count": len(events),
            "sample_events": events[:10],
        },
        "safe_next_steps": cli._safe_next_steps_for_incident(str(args.incident_id), incident, hypotheses),
    }
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=cli._investigation_lines(payload)))


async def handle_investigate_service(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    service_id = quote(str(args.service_id), safe="")
    detail = await cli._require_local_api(config, "GET", f"/api/services/{service_id}?limit=50")
    service = detail.get("service") or {}
    incidents = list(detail.get("incidents") or [])
    events = list(detail.get("events") or [])
    payload = {
        "command": "investigate service",
        "config_path": str(config_path),
        "focus": args.service_id,
        "summary": f"Service {args.service_id} is {service.get('status', 'unknown')} with {len(incidents)} active incident(s).",
        "priority": "high" if service.get("status") in {"critical", "degraded"} else "normal",
        "evidence": {
            "service": service,
            "active_incidents": incidents,
            "event_count": len(events),
            "sample_events": events[:10],
        },
        "safe_next_steps": cli._safe_next_steps_for_service(str(args.service_id), service, incidents),
    }
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=cli._investigation_lines(payload)))


async def handle_status(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from cli_core.result import CommandError
    from inferra_legacy.cli_display import render_health_panel, render_offline_notice

    _config_path, config = cli._load_config_for_command(args)
    try:
        payload: dict = await cli._local_api_json(config, "GET", "/api/health")
        payload["mode"] = "live"
    except CommandError:
        from runtime.context import build_runtime_context_snapshot, runtime_context_to_correlation_dict

        snap = await build_runtime_context_snapshot()
        payload = {"mode": "offline", "runtime": runtime_context_to_correlation_dict(snap)}

    def hook() -> None:
        if payload.get("mode") == "offline":
            render_offline_notice(payload.get("runtime") or {})
        else:
            render_health_panel(payload)

    use_rich = not getattr(args, "json", False)
    return cli._emit_result(
        args,
        CommandResult(payload=payload, rich_hook=hook if use_rich else None),
    )


async def handle_overview(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    from cli_core.result import CommandError
    from inferra_legacy.cli_display import render_overview

    _config_path, config = cli._load_config_for_command(args)
    try:
        payload = await cli._local_api_json(config, "GET", "/api/overview")
    except CommandError as exc:
        raise CommandError(f"{exc} The unified overview needs `inferra run`.") from exc

    def hook() -> None:
        render_overview(payload)

    use_rich = not getattr(args, "json", False)
    return cli._emit_result(args, CommandResult(payload=payload, rich_hook=hook if use_rich else None))
