"""FastAPI application factory.

This module is now a thin assembler. Domain logic lives in `web.routers.*`
and shared helpers live in `web._shared`. The single thing we keep here is:

- `create_app`: builds the FastAPI app, lifespan, middleware, and includes routers.
- `_build_dashboard_payload` and `_runtime_health_bundle`: shared by multiple routers.
- The `/ws` realtime channel: tightly coupled with multiple stores and live-hub state.
"""

from __future__ import annotations

import importlib.metadata
import os
from contextlib import asynccontextmanager
from pathlib import Path
from typing import Any

from pydantic import ValidationError

from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from starlette.middleware.cors import CORSMiddleware

from ai.explainer import chat_answer_from_model_output
from ai import AIService
from ai.ollama import OllamaError
from ai.prompts import ExplainIncidentOutput, extract_json_object
from app import InferraRuntime
from config import InferraConfig
from core.enums import IncidentState
from core.ids import new_id
from core.logging import get_logger
from core.models import ResolutionInfo
from core.time import utc_now
from explanation import TemplateExplanationEngine
from explanation.cache_key import explanation_cache_key_hashes
from explanation.finalize import explanation_result_from_dict, finalize_explanation_payload
from web._shared import (
    active_incidents,
    event_rate,
    explanation_to_dict,
    hypothesis_to_dict,
    incident_to_dict,
    persist_ai_prompt_trace,
    service_health,
    severity_counts,
)
from web.frontend_assets import mount_frontend_assets, register_frontend_routes
from web.http_security import ContentSecurityPolicyMiddleware, LocalSecurityMiddleware
from web.live_hub import LiveHub
from web.rate_limit import HostRateLimiter
from web.routers import (
    AiDeps,
    CollectorsDeps,
    EventsDeps,
    IncidentsDeps,
    InvestigationDeps,
    ServicesDeps,
    TopologyDeps,
    WorkspaceDeps,
    build_ai_router,
    build_collectors_router,
    build_events_router,
    build_incidents_router,
    build_investigation_router,
    build_services_router,
    build_topology_router,
    build_workspace_router,
)
from web.routes.system import SystemRouteDeps, build_system_router

_log = get_logger(__name__)

_CSP_POLICY = (
    "default-src 'self'; base-uri 'self'; frame-ancestors 'none'; img-src 'self' data:; "
    "font-src 'self' data:; connect-src 'self' ws: wss:; script-src 'self'; style-src 'self' 'unsafe-inline'"
)


async def _build_dashboard_payload(runtime: InferraRuntime, ai_holder: list[AIService]) -> dict[str, Any]:
    events = runtime.event_store.latest_events(limit=500)
    incidents = active_incidents(runtime)
    services = runtime.event_store.list_services()
    dedup_stats = runtime.dedup.stats()
    noise_stats = runtime.noise_filter.stats()
    health_row = await _runtime_health_bundle(runtime, ai_holder)
    return {
        "health": {
            "status": health_row.get("status"),
            "active_incidents": len(incidents),
            "queue_depth": health_row.get("queue_depth"),
            "collector_errors": health_row.get("collector_errors"),
            "degraded": health_row.get("degraded"),
            "degraded_reasons": health_row.get("degraded_reasons"),
            "storage_writes_ok": health_row.get("storage_writes_ok"),
            "data_dir_bytes_free": health_row.get("data_dir_bytes_free"),
            "ai_enabled": health_row.get("ai_enabled"),
            "ai_available": health_row.get("ai_available"),
            "ai_reason": health_row.get("ai_reason"),
        },
        "dedup": {
            "tracked_fingerprints": dedup_stats.tracked_fingerprints,
            "total_suppressed": dedup_stats.total_suppressed,
            "total_summaries_emitted": dedup_stats.total_summaries_emitted,
            "evictions": dedup_stats.evictions,
        },
        "noise": {
            "blocklist_hits": noise_stats.blocklist_hits,
            "allowlist_hits": noise_stats.allowlist_hits,
            "adaptive_demotions": noise_stats.adaptive_demotions,
            "routine_fingerprints": noise_stats.routine_fingerprints,
            "total_filtered": noise_stats.total_filtered,
        },
        "incidents": [incident_to_dict(item) for item in incidents[:10]],
        "services": service_health(services, incidents),
        "event_rate": event_rate(events),
        "severity_counts": severity_counts(events),
    }


async def _runtime_health_bundle(runtime: InferraRuntime, ai_holder: list[AIService]) -> dict[str, Any]:
    snap = runtime.degradation_snapshot()
    ai_enabled = bool(runtime.config.ai.enabled)
    ai_available = True
    ai_reason: str | None = None
    if ai_enabled:
        status = await ai_holder[0].status()
        ai_available = bool(status.get("available"))
        ai_reason = str(status.get("reason") or status.get("error") or "") or None
    else:
        ai_available = False
        ai_reason = "disabled"
    degraded = bool(snap["degraded"]) or (ai_enabled and not ai_available)
    reasons = sorted(set(snap["degraded_reasons"]))
    if ai_enabled and not ai_available:
        reasons = sorted(set([*reasons, "ai_unavailable"]))
    return {
        "status": "observing",
        "events_db": str(runtime.event_store.path),
        "incidents_db": str(runtime.incident_store.path),
        "active_incidents": len(active_incidents(runtime)),
        "queue_depth": runtime.raw_queue.qsize(),
        "collectors": len(runtime.collector_health()),
        "collector_errors": sum(int(item.get("error_count", 0)) for item in runtime.collector_health()),
        "degraded": degraded,
        "degraded_reasons": reasons,
        "storage_writes_ok": snap["storage_writes_ok"],
        "data_dir_bytes_free": snap["data_dir_bytes_free"],
        "raw_queue_depth": snap["raw_queue_depth"],
        "raw_queue_maxsize": snap["raw_queue_maxsize"],
        "ai_enabled": ai_enabled,
        "ai_available": ai_available,
        "ai_reason": ai_reason,
    }


def _prometheus_text(runtime: InferraRuntime) -> str:
    active = active_incidents(runtime)
    lines = [
        "# HELP inferra_events_total Approximate stored normalized events.",
        "# TYPE inferra_events_total counter",
        f"inferra_events_total {runtime.event_store.count_events()}",
        "# HELP inferra_active_incidents Active incidents (open, investigating, explained).",
        "# TYPE inferra_active_incidents gauge",
        f"inferra_active_incidents {len(active)}",
        "# HELP inferra_raw_queue_depth Raw ingestion queue depth.",
        "# TYPE inferra_raw_queue_depth gauge",
        f"inferra_raw_queue_depth {runtime.raw_queue.qsize()}",
    ]
    return "\n".join(lines) + "\n"


def create_app(
    config: InferraConfig | None = None,
    config_path: str | Path | None = None,
    runtime: InferraRuntime | None = None,
) -> FastAPI:
    owns_runtime = runtime is None
    runtime = runtime or InferraRuntime(config)
    explanations = TemplateExplanationEngine()
    ai_holder: list[AIService] = [AIService(runtime.config)]
    live_hub = LiveHub()
    mounted_http_collector = next(
        (collector for collector in runtime.app_http_collectors() if collector.enable_main_api),
        None,
    )

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        if owns_runtime:
            runtime.attach_live_hub(live_hub)
            await runtime.start()
        app.state.runtime = runtime
        app.state.live_hub = live_hub
        app.state.ai_holder = ai_holder
        yield
        if owns_runtime:
            await runtime.stop()

    try:
        _app_version = importlib.metadata.version("inferra")
    except importlib.metadata.PackageNotFoundError:
        _app_version = "0.2.0"
    app = FastAPI(title="Inferra", version=_app_version, lifespan=lifespan)
    srv = runtime.config.server
    app.state.rate_chat = HostRateLimiter(srv.rate_limit_chat_tokens_per_minute)
    app.state.rate_explain = HostRateLimiter(srv.rate_limit_explain_tokens_per_minute)
    app.state.ws_rate_chat = HostRateLimiter(srv.rate_limit_chat_tokens_per_minute)
    app.state.ws_rate_explain = HostRateLimiter(srv.rate_limit_explain_tokens_per_minute)
    app.add_middleware(
        CORSMiddleware,
        allow_origins=srv.cors_origins,
        allow_credentials=True,
        allow_methods=["*"],
        allow_headers=["*"],
    )
    app.add_middleware(
        LocalSecurityMiddleware,
        require_loopback=srv.require_loopback,
        auth_token_env=srv.auth_token_env,
        allow_paths=frozenset({"/"}),
    )
    app.add_middleware(ContentSecurityPolicyMiddleware, policy=_CSP_POLICY)
    ui_dist = mount_frontend_assets(app)
    if mounted_http_collector is not None:
        app.include_router(mounted_http_collector.router())

    app.include_router(
        build_system_router(
            SystemRouteDeps(
                runtime=runtime,
                ai_holder=ai_holder,
                config_path=config_path,
                dashboard_payload=_build_dashboard_payload,
                health_bundle=_runtime_health_bundle,
                prometheus_text=_prometheus_text,
            )
        )
    )
    app.include_router(build_collectors_router(CollectorsDeps(runtime=runtime, live_hub=live_hub)))
    app.include_router(build_ai_router(AiDeps(runtime=runtime, ai_holder=ai_holder, config_path=config_path)))
    app.include_router(build_events_router(EventsDeps(runtime=runtime, ai_holder=ai_holder)))
    app.include_router(
        build_incidents_router(
            IncidentsDeps(
                runtime=runtime,
                ai_holder=ai_holder,
                explanations=explanations,
                live_hub=live_hub,
            )
        )
    )
    app.include_router(build_services_router(ServicesDeps(runtime=runtime)))
    app.include_router(build_topology_router(TopologyDeps(runtime=runtime)))
    app.include_router(
        build_investigation_router(
            InvestigationDeps(
                runtime=runtime,
                ai_holder=ai_holder,
                dashboard_payload=_build_dashboard_payload,
            )
        )
    )
    app.include_router(
        build_workspace_router(
            WorkspaceDeps(
                runtime=runtime,
                config_path=str(config_path) if config_path is not None else None,
            )
        )
    )

    @app.websocket("/ws")
    async def live_updates(websocket: WebSocket) -> None:
        await websocket.accept()
        srv = runtime.config.server
        client_host = websocket.client.host if websocket.client else None
        if srv.require_loopback and client_host not in (None, "127.0.0.1", "::1", "testclient"):
            await websocket.send_json({"type": "error", "error": "local clients only"})
            await websocket.close(code=4403)
            return
        if srv.auth_token_env:
            expected = os.environ.get(srv.auth_token_env, "").strip()
            if not expected:
                await websocket.send_json({"type": "error", "error": "server auth token env is unset"})
                await websocket.close(code=14503)
                return
            token = (websocket.query_params.get("access_token") or "").strip()
            header = websocket.headers.get("authorization") or websocket.headers.get("Authorization") or ""
            if header.lower().startswith("bearer "):
                token = token or header.partition(" ")[2].strip()
            if token != expected:
                await websocket.send_json({"type": "error", "error": "unauthorized"})
                await websocket.close(code=4401)
                return
        await live_hub.register(websocket)
        try:
            while True:
                raw = await websocket.receive_json()
                mtype = str(raw.get("type") or "")
                if mtype == "subscribe_incident":
                    live_hub.subscribe_incident(websocket, str(raw.get("incident_id") or ""))
                    await websocket.send_json({"type": "subscribed", "incident_id": raw.get("incident_id")})
                elif mtype == "unsubscribe_incident":
                    live_hub.unsubscribe_incident(websocket, str(raw.get("incident_id") or ""))
                    await websocket.send_json({"type": "unsubscribed", "incident_id": raw.get("incident_id")})
                elif mtype == "resolve_incident":
                    incident_id = str(raw.get("incident_id") or "")
                    item = runtime.incident_store.get_incident(incident_id)
                    if item is None:
                        await websocket.send_json({"type": "error", "error": "Incident not found"})
                        continue
                    runtime.incident_store.resolve_incident(
                        incident_id,
                        ResolutionInfo(
                            resolved_by=str(raw.get("resolved_by") or "operator"),
                            correct_hypothesis_id=raw.get("correct_hypothesis_id"),
                            feedback_type=str(raw.get("feedback_type") or "confirmed"),
                            notes=raw.get("notes"),
                            resolved_at=utc_now(),
                        ),
                    )
                    await live_hub.broadcast(
                        "incident_resolved",
                        {"incident_id": incident_id, "reason": "operator_resolve"},
                    )
                    await websocket.send_json({"type": "resolve_ack", "incident_id": incident_id})
                elif mtype == "chat_send":
                    incident_id = str(raw.get("incident_id") or "")
                    if not app.state.ws_rate_chat.consume(f"ws:{id(websocket)}"):
                        await websocket.send_json({"type": "error", "error": "chat rate limit exceeded"})
                        continue
                    item = runtime.incident_store.get_incident(incident_id)
                    if item is None:
                        await websocket.send_json({"type": "error", "error": "Incident not found"})
                        continue
                    question = str(raw.get("question") or "").strip()
                    if not question:
                        await websocket.send_json({"type": "error", "error": "'question' is required"})
                        continue
                    live_hub.subscribe_incident(websocket, incident_id)
                    events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
                    events_for_incident = [event for event in events_for_incident if event is not None]
                    hypotheses = [
                        hypothesis_to_dict(hypothesis)
                        for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
                    ]
                    prior_messages = runtime.incident_store.list_chat_messages(incident_id)
                    history_rows = [{"role": message.role, "content": message.content} for message in prior_messages]
                    runtime.incident_store.append_chat_message(incident_id, "user", question)
                    stream_iter, prompt_trace = ai_holder[0].incident_chat_stream(
                        question,
                        incident_to_dict(item),
                        hypotheses,
                        events_for_incident,
                        history=history_rows,
                    )
                    aggregated: list[str] = []
                    try:
                        async for chunk in stream_iter:
                            aggregated.append(chunk.content)
                            if runtime.config.ai.stream:
                                await live_hub.broadcast(
                                    "ai_stream_token",
                                    {"incident_id": incident_id, "token": chunk.content, "done": chunk.done},
                                )
                    except OllamaError:
                        full_text = ""
                    else:
                        full_text = "".join(aggregated)
                    answer_text = chat_answer_from_model_output(full_text, hypotheses)
                    runtime.incident_store.append_chat_message(incident_id, "assistant", answer_text)
                    if prompt_trace is not None:
                        persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
                    await websocket.send_json({"type": "chat_done", "incident_id": incident_id})
                elif mtype == "explain_request":
                    incident_id = str(raw.get("incident_id") or "")
                    if not app.state.ws_rate_explain.consume(f"ws:{id(websocket)}"):
                        await websocket.send_json({"type": "error", "error": "explain rate limit exceeded"})
                        continue
                    item = runtime.incident_store.get_incident(incident_id)
                    if item is None:
                        await websocket.send_json({"type": "error", "error": "Incident not found"})
                        continue
                    live_hub.subscribe_incident(websocket, incident_id)
                    events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
                    events_for_incident = [event for event in events_for_incident if event is not None]
                    hypotheses = [
                        hypothesis_to_dict(hypothesis)
                        for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
                    ]
                    incident_payload = incident_to_dict(item)
                    hyp_h, evt_h = explanation_cache_key_hashes(hypotheses, events_for_incident)
                    cached = runtime.incident_store.get_cached_explanation(incident_id, hyp_h, evt_h)
                    if cached is not None:
                        if item.state == IncidentState.INVESTIGATING:
                            runtime.incident_store.transition_state(
                                incident_id,
                                IncidentState.EXPLAINED,
                                "explanation persisted",
                            )
                        exp_dict = explanation_to_dict(cached)
                        await live_hub.broadcast(
                            "explanation_ready",
                            {"incident_id": incident_id, "explanation": exp_dict},
                        )
                        await websocket.send_json({"type": "explain_done", "incident_id": incident_id})
                        continue
                    if not runtime.config.ai.enabled:
                        explanation_result = explanations.generate(
                            incident_payload,
                            hypotheses,
                            events_for_incident,
                        )
                    elif not runtime.config.ai.stream:
                        explanation_job = await ai_holder[0].explain(
                            incident_payload,
                            hypotheses,
                            events_for_incident,
                        )
                        if explanation_job is None:
                            explanation_result = explanations.generate(
                                incident_payload,
                                hypotheses,
                                events_for_incident,
                            )
                            prompt_trace = None
                        else:
                            explanation_payload, prompt_trace = explanation_job
                            explanation_result = explanation_result_from_dict(explanation_payload)
                            if prompt_trace is not None:
                                persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
                    else:
                        stream_iter, prompt_trace = ai_holder[0].incident_explain_stream(
                            incident_payload,
                            hypotheses,
                            events_for_incident,
                        )
                        if stream_iter is None:
                            explanation_result = explanations.generate(
                                incident_payload,
                                hypotheses,
                                events_for_incident,
                            )
                        else:
                            parts: list[str] = []
                            stream_failed = False
                            try:
                                async for chunk in stream_iter:
                                    parts.append(chunk.content)
                                    await live_hub.broadcast(
                                        "ai_stream_token",
                                        {"incident_id": incident_id, "token": chunk.content, "done": chunk.done},
                                    )
                            except OllamaError:
                                stream_failed = True
                                explanation_result = explanations.generate(
                                    incident_payload,
                                    hypotheses,
                                    events_for_incident,
                                )
                            if not stream_failed:
                                raw = "".join(parts)
                                data = extract_json_object(raw)
                                try:
                                    validated = ExplainIncidentOutput.model_validate(data)
                                except ValidationError:
                                    explanation_result = explanations.generate(
                                        incident_payload,
                                        hypotheses,
                                        events_for_incident,
                                    )
                                else:
                                    payload_dict = {
                                        "explanation_id": new_id("exp"),
                                        "incident_id": incident_payload["incident_id"],
                                        "summary": validated.summary,
                                        "primary_hypothesis_text": validated.primary_hypothesis_text,
                                        "evidence_narrative": validated.evidence_narrative,
                                        "timeline_narrative": validated.timeline_narrative,
                                        "alternative_explanations": list(validated.alternative_explanations),
                                        "suggested_actions": list(validated.suggested_actions),
                                        "uncertainty_notes": list(validated.uncertainty_notes),
                                        "generation_model": runtime.config.ai.model,
                                        "guardrail_violations": [],
                                    }
                                    finalized = finalize_explanation_payload(
                                        incident_payload,
                                        hypotheses,
                                        events_for_incident,
                                        payload_dict,
                                        template=False,
                                    )
                                    explanation_result = explanation_result_from_dict(finalized)
                            if prompt_trace is not None and not stream_failed:
                                persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
                    runtime.incident_store.add_explanation(explanation_result)
                    item_after = runtime.incident_store.get_incident(incident_id)
                    if item_after is not None and item_after.state == IncidentState.INVESTIGATING:
                        runtime.incident_store.transition_state(
                            incident_id,
                            IncidentState.EXPLAINED,
                            "explanation persisted",
                        )
                    exp_dict = explanation_to_dict(explanation_result)
                    await live_hub.broadcast(
                        "explanation_ready",
                        {"incident_id": incident_id, "explanation": exp_dict},
                    )
                    await websocket.send_json({"type": "explain_done", "incident_id": incident_id})
                else:
                    await websocket.send_json({"type": "error", "error": f"unknown message type: {mtype}"})
        except WebSocketDisconnect:
            return
        finally:
            await live_hub.unregister(websocket)

    register_frontend_routes(app, ui_dist)
    return app
