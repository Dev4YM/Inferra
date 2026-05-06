"""Investigation API: structured AI-driven investigation with safe defaults.

Endpoints under `/api/investigate/*` and a small `/api/ai/*` extension provide:
- `GET  /api/investigate/now`              — investigate current overview
- `GET  /api/investigate/incident/{id}`    — investigate one incident
- `GET  /api/investigate/service/{id}`     — investigate one service
- `POST /api/ai/ask`                       — answer a freeform question with cited evidence
- `GET  /api/ai/doctor`                    — provider readiness checks
- `GET  /api/ai/report/{incident_id}`      — operator/developer investigation report

All endpoints are read-only and never mutate observed systems.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta
from typing import Any, Awaitable, Callable

from fastapi import APIRouter, Body, HTTPException, Query

from ai.investigation import (
    EvidenceBundle,
    InvestigationResult,
    investigation_result_to_dict,
    run_investigation,
)
from ai.service import AIService
from app import InferraRuntime
from events.serialization import event_to_dict
from runtime.context import build_runtime_context_snapshot, runtime_context_to_correlation_dict
from runtime.workspace_map import build_workspace_map
from web._shared import active_incidents as _active_incidents
from web._shared import hypothesis_to_dict as _hypothesis_to_dict
from web._shared import incident_to_dict as _incident_to_dict


DashboardPayloadBuilder = Callable[[InferraRuntime, list[AIService]], Awaitable[dict[str, Any]]]


@dataclass(frozen=True)
class InvestigationDeps:
    runtime: InferraRuntime
    ai_holder: list[AIService]
    dashboard_payload: DashboardPayloadBuilder


async def _build_overview_bundle(deps: InvestigationDeps, *, mode: str, question: str = "") -> EvidenceBundle:
    dash = await deps.dashboard_payload(deps.runtime, deps.ai_holder)
    snapshot = await build_runtime_context_snapshot(process_limit=40)
    runtime_dict = runtime_context_to_correlation_dict(snapshot)
    services = list(dash.get("services") or [])
    workspace = build_workspace_map(
        deps.runtime.config,
        services=[str(item.get("service_id")) for item in services if item.get("service_id")],
    )
    incidents = list(dash.get("incidents") or [])
    return EvidenceBundle(
        mode=mode,
        incident=incidents[0] if incidents else None,
        hypotheses=[],
        events=[event_to_dict(event) for event in deps.runtime.event_store.latest_events(limit=20)],
        services=services[:30],
        runtime={
            "containers": runtime_dict.get("containers", [])[:20],
            "process_sample_size": len(runtime_dict.get("processes", [])),
            "host_id": runtime_dict.get("hostname"),
        },
        workspace={
            "projects": workspace.get("projects", [])[:20],
            "service_mappings": workspace.get("service_mappings", [])[:20],
        },
        user_question=question,
    )


async def _build_incident_bundle(
    deps: InvestigationDeps,
    incident_id: str,
    *,
    mode: str,
    question: str = "",
) -> EvidenceBundle:
    runtime = deps.runtime
    item = runtime.incident_store.get_incident(incident_id)
    if item is None:
        raise HTTPException(status_code=404, detail="Incident not found")
    events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
    events_for_incident = [event for event in events_for_incident if event is not None]
    hypotheses = [_hypothesis_to_dict(h) for h in runtime.incident_store.get_hypotheses(incident_id)]
    services_payload = list((await deps.dashboard_payload(runtime, deps.ai_holder)).get("services") or [])
    affected = set(item.affected_services) | ({item.primary_service} if item.primary_service else set())
    services = [row for row in services_payload if str(row.get("service_id")) in affected]
    workspace = build_workspace_map(runtime.config, services=list(affected))
    return EvidenceBundle(
        mode=mode,
        incident=_incident_to_dict(item),
        hypotheses=hypotheses,
        events=[event_to_dict(event) for event in events_for_incident][:30],
        services=services,
        workspace={
            "service_mappings": workspace.get("service_mappings", []),
            "unmapped_services": workspace.get("unmapped_services", []),
        },
        user_question=question,
    )


async def _build_service_bundle(
    deps: InvestigationDeps,
    service_id: str,
    *,
    mode: str,
    question: str = "",
) -> EvidenceBundle:
    runtime = deps.runtime
    services_payload = list((await deps.dashboard_payload(runtime, deps.ai_holder)).get("services") or [])
    service = next((row for row in services_payload if str(row.get("service_id")) == service_id), None)
    if service is None:
        raise HTTPException(status_code=404, detail="Service not found")
    related_events = list(runtime.event_store.query_by_service(service_id, timedelta(hours=24), limit=30))
    workspace = build_workspace_map(runtime.config, services=[service_id])
    return EvidenceBundle(
        mode=mode,
        incident=None,
        hypotheses=[],
        events=[event_to_dict(event) for event in related_events][:30],
        services=[service],
        workspace={
            "service_mappings": workspace.get("service_mappings", []),
        },
        user_question=question,
    )


def build_investigation_router(deps: InvestigationDeps) -> APIRouter:
    router = APIRouter()
    runtime = deps.runtime

    def _resolve_mode(override: str | None) -> str:
        if override and override in {"operator", "developer"}:
            return override
        return runtime.config.experience.mode

    @router.get("/api/investigate/now")
    async def investigate_now(mode: str | None = Query(default=None)) -> dict[str, Any]:
        bundle = await _build_overview_bundle(deps, mode=_resolve_mode(mode))
        result = await run_investigation(runtime.config, bundle, ai_service=deps.ai_holder[0])
        return _wrap_result(result, focus="overview")

    @router.get("/api/investigate/incident/{incident_id}")
    async def investigate_incident(incident_id: str, mode: str | None = Query(default=None)) -> dict[str, Any]:
        bundle = await _build_incident_bundle(deps, incident_id, mode=_resolve_mode(mode))
        result = await run_investigation(runtime.config, bundle, ai_service=deps.ai_holder[0])
        return _wrap_result(result, focus=f"incident:{incident_id}")

    @router.get("/api/investigate/service/{service_id}")
    async def investigate_service(service_id: str, mode: str | None = Query(default=None)) -> dict[str, Any]:
        bundle = await _build_service_bundle(deps, service_id, mode=_resolve_mode(mode))
        result = await run_investigation(runtime.config, bundle, ai_service=deps.ai_holder[0])
        return _wrap_result(result, focus=f"service:{service_id}")

    @router.post("/api/ai/ask")
    async def ai_ask(payload: dict[str, Any] = Body(...)) -> dict[str, Any]:
        question = str(payload.get("question") or "").strip()
        if not question:
            raise HTTPException(status_code=400, detail="'question' is required")
        scope = str(payload.get("scope") or "overview").strip().lower()
        mode = _resolve_mode(str(payload.get("mode") or "") or None)
        if scope.startswith("incident:"):
            incident_id = scope.split(":", 1)[1]
            bundle = await _build_incident_bundle(deps, incident_id, mode=mode, question=question)
            focus = f"incident:{incident_id}"
        elif scope.startswith("service:"):
            service_id = scope.split(":", 1)[1]
            bundle = await _build_service_bundle(deps, service_id, mode=mode, question=question)
            focus = f"service:{service_id}"
        else:
            bundle = await _build_overview_bundle(deps, mode=mode, question=question)
            focus = "overview"
        result = await run_investigation(runtime.config, bundle, ai_service=deps.ai_holder[0])
        return _wrap_result(result, focus=focus, question=question)

    @router.get("/api/ai/doctor")
    async def ai_doctor() -> dict[str, Any]:
        ai = runtime.config.ai
        provider_status = await deps.ai_holder[0].status()
        warnings: list[str] = []
        ok = bool(provider_status.get("available")) or not ai.enabled
        if ai.enabled and not provider_status.get("available"):
            warnings.append(f"Ollama not reachable at {ai.base_url}: {provider_status.get('reason') or provider_status.get('error') or 'unknown'}")
        if ai.allow_remote and not ai.token_env:
            warnings.append("Remote provider allowed but no auth token env is configured.")
        if ai.allow_remote and ai.base_url.startswith("http://"):
            warnings.append("Remote provider over plaintext HTTP; prefer HTTPS for off-host access.")
        if not ai.redact_raw_logs:
            warnings.append("Raw log redaction is disabled; remote providers may receive sensitive content.")
        return {
            "ok": ok,
            "enabled": ai.enabled,
            "provider": ai.provider,
            "base_url": ai.base_url,
            "model": ai.model,
            "allow_remote": ai.allow_remote,
            "token_env_set": bool(ai.token_env),
            "redact_raw_logs": ai.redact_raw_logs,
            "available": bool(provider_status.get("available")),
            "resolved_model": provider_status.get("resolved_model"),
            "latency_ms": provider_status.get("latency_ms"),
            "version": provider_status.get("version"),
            "warnings": warnings,
            "guidance": [
                "AI is presentation-only; deterministic scores are never silently changed.",
                "All AI suggestions are read-only; no command is executed automatically.",
            ],
        }

    @router.get("/api/ai/report/{incident_id}")
    async def ai_report(incident_id: str, mode: str | None = Query(default=None)) -> dict[str, Any]:
        bundle = await _build_incident_bundle(deps, incident_id, mode=_resolve_mode(mode))
        result = await run_investigation(runtime.config, bundle, ai_service=deps.ai_holder[0])
        return _wrap_result(result, focus=f"incident:{incident_id}", report=True)

    return router


def _wrap_result(result: InvestigationResult, *, focus: str, question: str = "", report: bool = False) -> dict[str, Any]:
    payload = investigation_result_to_dict(result)
    payload["focus"] = focus
    if question:
        payload["question"] = question
    if report:
        payload["report"] = True
    return payload
