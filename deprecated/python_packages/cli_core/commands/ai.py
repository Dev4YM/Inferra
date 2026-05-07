"""AI investigation commands: ask, investigate, report, trace, doctor.

These call the local API for live investigations and fall back to a
deterministic offline summary for `ai doctor` when the API is unreachable.
"""

from __future__ import annotations

import argparse
from urllib.parse import quote

from cli_core.result import CommandError, CommandResult


async def handle_ai_ask(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    payload_in = {
        "question": str(args.question),
        "scope": str(args.scope or "overview"),
        "mode": str(args.mode) if getattr(args, "mode", None) else "",
    }
    payload = await cli._require_local_api(config, "POST", "/api/ai/ask", payload_in)
    payload["command"] = "ai ask"
    payload["config_path"] = str(config_path)
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=cli._investigation_output_lines(payload)),
    )


async def handle_ai_investigate(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    target = str(getattr(args, "target", "latest")).lower()
    identifier = getattr(args, "identifier", None)
    mode_arg = f"?mode={quote(args.mode)}" if getattr(args, "mode", None) else ""
    if target == "latest":
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
            payload = await cli._require_local_api(config, "GET", f"/api/investigate/now{mode_arg}")
            payload["focus"] = "no_active_incident"
        else:
            incident_id = incidents[0].get("incident_id")
            payload = await cli._require_local_api(
                config,
                "GET",
                f"/api/investigate/incident/{quote(str(incident_id), safe='')}{mode_arg}",
            )
    elif target == "incident":
        if not identifier:
            raise CommandError("`ai investigate incident <id>` requires an incident id.")
        payload = await cli._require_local_api(
            config,
            "GET",
            f"/api/investigate/incident/{quote(str(identifier), safe='')}{mode_arg}",
        )
    elif target == "service":
        if not identifier:
            raise CommandError("`ai investigate service <id>` requires a service id.")
        payload = await cli._require_local_api(
            config,
            "GET",
            f"/api/investigate/service/{quote(str(identifier), safe='')}{mode_arg}",
        )
    else:
        raise CommandError("Unknown investigate target. Use latest, incident <id>, or service <id>.")
    payload["command"] = "ai investigate"
    payload["config_path"] = str(config_path)
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=cli._investigation_output_lines(payload)),
    )


async def handle_ai_report(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    incident_id = quote(str(args.incident_id), safe="")
    mode_arg = f"?mode={quote(args.mode)}" if getattr(args, "mode", None) else ""
    payload = await cli._require_local_api(config, "GET", f"/api/ai/report/{incident_id}{mode_arg}")
    payload["command"] = "ai report"
    payload["config_path"] = str(config_path)
    return cli._emit_result(
        args,
        CommandResult(payload=payload, stdout_lines=cli._investigation_output_lines(payload)),
    )


async def handle_ai_trace(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    incident_id = quote(str(args.incident_id), safe="")
    payload = await cli._require_local_api(config, "GET", f"/api/ai/trace/{incident_id}")
    payload["command"] = "ai trace"
    payload["config_path"] = str(config_path)
    audit = payload.get("prompt_audit") or {}
    contract = payload.get("prompt_contract") or {}
    redaction = payload.get("redaction") or {}
    lines = [
        f"incident_id={args.incident_id}",
        f"raw_logs_sent={audit.get('raw_logs_sent', False)}",
        f"allowed_fields={','.join(map(str, contract.get('allowed') or []))}",
        f"blocked_fields={','.join(map(str, contract.get('blocked') or []))}",
        f"max_context_events={redaction.get('max_events', '?')}",
    ]
    return cli._emit_result(args, CommandResult(payload=payload, stdout_lines=lines))


async def handle_ai_doctor(args: argparse.Namespace, parser: argparse.ArgumentParser) -> int:
    import inferra_legacy.cli as cli

    config_path, config = cli._load_config_for_command(args)
    try:
        payload = await cli._local_api_json(config, "GET", "/api/ai/doctor")
        payload["mode"] = "live"
    except CommandError:
        from ai import AIService

        provider_status = await AIService(config).status()
        warnings: list[str] = []
        ai = config.ai
        if ai.enabled and not provider_status.get("available"):
            warnings.append(
                f"Ollama not reachable at {ai.base_url}: "
                f"{provider_status.get('reason') or provider_status.get('error') or 'unknown'}"
            )
        if ai.allow_remote and not ai.token_env:
            warnings.append("Remote provider allowed but no auth token env is configured.")
        if ai.allow_remote and ai.base_url.startswith("http://"):
            warnings.append("Remote provider over plaintext HTTP; prefer HTTPS for off-host access.")
        payload = {
            "mode": "offline",
            "ok": bool(provider_status.get("available")) or not ai.enabled,
            "enabled": ai.enabled,
            "provider": ai.provider,
            "base_url": ai.base_url,
            "model": ai.model,
            "allow_remote": ai.allow_remote,
            "token_env_set": bool(ai.token_env),
            "redact_raw_logs": ai.redact_raw_logs,
            "available": bool(provider_status.get("available")),
            "warnings": warnings,
        }
    payload["command"] = "ai doctor"
    payload["config_path"] = str(config_path)
    lines = [
        f"ok={payload.get('ok', False)}",
        f"enabled={payload.get('enabled', False)} provider={payload.get('provider', '?')} model={payload.get('model', '?')}",
        f"available={payload.get('available', False)} allow_remote={payload.get('allow_remote', False)}",
        f"redact_raw_logs={payload.get('redact_raw_logs', True)} token_env_set={payload.get('token_env_set', False)}",
    ]
    for warning in payload.get("warnings", []) or []:
        lines.append(f"warning: {warning}")
    return cli._emit_result(
        args,
        CommandResult(
            payload=payload,
            stdout_lines=lines,
            exit_code=0 if payload.get("ok") else 1,
        ),
    )
