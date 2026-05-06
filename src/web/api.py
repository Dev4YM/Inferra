from __future__ import annotations

import importlib.metadata
import os
from contextlib import asynccontextmanager
from dataclasses import replace
from datetime import timedelta
from pathlib import Path
from typing import Any

from pydantic import ValidationError

from fastapi import Body, FastAPI, HTTPException, Query, Request, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse, PlainTextResponse
from fastapi.staticfiles import StaticFiles
from starlette.middleware.cors import CORSMiddleware

from ai.explainer import AiPromptTrace, chat_answer_from_model_output
from ai import AIService
from ai.ollama import OllamaError
from ai.prompts import EXPLAIN_INCIDENT_ALLOWED_FIELDS, ExplainIncidentOutput, extract_json_object
from app import InferraRuntime
from config import InferraConfig, config_to_dict, parse_config_payload, write_config
from core.enums import IncidentState, Severity
from core.ids import new_id
from core.logging import get_logger
from core.models import ExplanationResult, Incident, IncidentAiTrace, IncidentFeedback, ResolutionInfo, ScoredHypothesis
from core.time import to_iso, utc_now
from events.models import EventFilter, NormalizedEvent
from events.serialization import event_to_dict
from explanation import TemplateExplanationEngine
from explanation.cache_key import explanation_cache_key_hashes
from explanation.finalize import explanation_result_from_dict, finalize_explanation_payload
from storage.calibration_store import update_calibration
from storage.weight_store import update_weights
from web.http_security import ContentSecurityPolicyMiddleware, LocalSecurityMiddleware
from web.live_hub import LiveHub
from web.rate_limit import HostRateLimiter

from analysis.anomaly import anomaly_service_status_to_json, build_anomaly_service_status

_log = get_logger(__name__)

_CSP_POLICY = (
    "default-src 'self'; base-uri 'self'; frame-ancestors 'none'; img-src 'self' data:; "
    "font-src 'self' data:; connect-src 'self' ws: wss:; script-src 'self'; style-src 'self' 'unsafe-inline'"
)


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
        "active_incidents": len(_active_incidents(runtime)),
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


def _client_ip(request: Request) -> str:
    return request.client.host if request.client else "unknown"


def _enforce_chat_rate(request: Request) -> None:
    if not request.app.state.rate_chat.consume(_client_ip(request)):
        raise HTTPException(status_code=429, detail="chat rate limit exceeded")


def _enforce_explain_rate(request: Request) -> None:
    if not request.app.state.rate_explain.consume(_client_ip(request)):
        raise HTTPException(status_code=429, detail="explain rate limit exceeded")


def _prometheus_text(runtime: InferraRuntime) -> str:
    active = _active_incidents(runtime)
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


def _persist_ai_prompt_trace(runtime: InferraRuntime, incident_id: str, trace: AiPromptTrace) -> None:
    record = IncidentAiTrace(
        trace_id=new_id("ait"),
        incident_id=incident_id,
        trace_kind=trace.trace_kind,
        sanitized_system_prompt=trace.sanitized_system_prompt,
        sanitized_user_prompt=trace.sanitized_user_prompt,
        allowed_fields=tuple(trace.allowed_fields),
        blocked_fields=tuple(trace.blocked_fields),
        raw_logs_sent=trace.raw_logs_sent,
        schema_version=trace.schema_version,
    )
    runtime.incident_store.add_ai_trace(record)


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
    static_dir = Path(__file__).parent / "static"
    app.mount("/static", StaticFiles(directory=static_dir), name="static")
    if mounted_http_collector is not None:
        app.include_router(mounted_http_collector.router())

    @app.get("/api/version")
    async def api_version() -> dict[str, Any]:
        try:
            version = importlib.metadata.version("inferra")
        except importlib.metadata.PackageNotFoundError:
            version = "0.0.0"
        return {"name": "inferra", "version": version, "api": "1"}

    @app.get("/api/metrics")
    async def api_metrics() -> PlainTextResponse:
        if not runtime.config.server.expose_prometheus_metrics:
            raise HTTPException(status_code=404, detail="metrics disabled")
        body = _prometheus_text(runtime)
        return PlainTextResponse(body, media_type="text/plain; version=0.0.4")

    @app.get("/api/config")
    async def get_config() -> dict[str, Any]:
        return {"config": config_to_dict(runtime.config)}

    @app.put("/api/config")
    async def put_config(payload: dict[str, Any] = Body(...)) -> dict[str, Any]:
        from core.errors import ConfigError

        try:
            new_cfg = parse_config_payload(payload.get("config") or payload)
        except ConfigError as exc:
            raise HTTPException(status_code=422, detail=str(exc)) from exc
        old_root = runtime.event_store.path.resolve().parent
        new_root = Path(new_cfg.storage.data_dir).resolve()
        if old_root != new_root:
            raise HTTPException(status_code=409, detail="storage.data_dir cannot be changed at runtime")
        runtime.config = new_cfg
        ai_holder[0] = AIService(runtime.config)
        app.state.rate_chat = HostRateLimiter(runtime.config.server.rate_limit_chat_tokens_per_minute)
        app.state.rate_explain = HostRateLimiter(runtime.config.server.rate_limit_explain_tokens_per_minute)
        app.state.ws_rate_chat = HostRateLimiter(runtime.config.server.rate_limit_chat_tokens_per_minute)
        app.state.ws_rate_explain = HostRateLimiter(runtime.config.server.rate_limit_explain_tokens_per_minute)
        if config_path is not None:
            write_config(runtime.config, config_path)
        return {"config": config_to_dict(runtime.config), "applied": True}

    @app.get("/")
    async def index():
        return FileResponse(static_dir / "index.html")

    @app.get("/api/health")
    async def health() -> dict[str, Any]:
        return await _runtime_health_bundle(runtime, ai_holder)

    @app.get("/api/dashboard")
    async def dashboard() -> dict[str, Any]:
        events = runtime.event_store.latest_events(limit=500)
        incidents = _active_incidents(runtime)
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
            "incidents": [_incident_to_dict(item) for item in incidents[:10]],
            "services": _service_health(services, incidents),
            "event_rate": _event_rate(events),
            "severity_counts": _severity_counts(events),
        }

    @app.get("/api/collectors")
    async def collectors() -> dict[str, Any]:
        return {"collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()}

    @app.post("/api/collectors/start")
    async def start_collectors() -> dict[str, Any]:
        await runtime.start_collectors()
        await live_hub.broadcast(
            "collector_health",
            {"collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()},
        )
        return {"started": True, "collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()}

    @app.post("/api/collectors/stop")
    async def stop_collectors() -> dict[str, Any]:
        await runtime.stop_collectors()
        await live_hub.broadcast(
            "collector_health",
            {"collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()},
        )
        return {"stopped": True, "collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()}

    @app.post("/api/collectors/one/start")
    async def start_one_collector(collector_id: str = Query(..., min_length=1)) -> dict[str, Any]:
        ok = await runtime.start_collector(collector_id.strip())
        if not ok:
            raise HTTPException(status_code=404, detail="collector not found")
        await live_hub.broadcast(
            "collector_health",
            {"collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()},
        )
        return {"started": True, "collector_id": collector_id, "collectors": runtime.collector_health()}

    @app.post("/api/collectors/one/stop")
    async def stop_one_collector(collector_id: str = Query(..., min_length=1)) -> dict[str, Any]:
        ok = await runtime.stop_collector(collector_id.strip())
        if not ok:
            raise HTTPException(status_code=404, detail="collector not found")
        await live_hub.broadcast(
            "collector_health",
            {"collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()},
        )
        return {"stopped": True, "collector_id": collector_id, "collectors": runtime.collector_health()}

    @app.get("/api/ai/status")
    async def ai_status() -> dict[str, Any]:
        return await ai_holder[0].status()

    @app.get("/api/ai/models")
    async def ai_models() -> dict[str, Any]:
        installed: list[str] = []
        error = None
        if runtime.config.ai.enabled:
            try:
                installed = await ai_holder[0].installed_models()
            except Exception as exc:  # provider errors should not make the UI unusable
                error = str(exc)
        return {"registry": ai_holder[0].registry(), "installed": installed, "error": error}

    @app.websocket("/api/ai/pull")
    async def ai_pull(websocket: WebSocket) -> None:
        await websocket.accept()
        try:
            payload = await websocket.receive_json()
            model = str(payload.get("model") or runtime.config.ai.model).strip()
            if not model:
                await websocket.send_json({"type": "error", "error": "'model' is required"})
                return
            async for progress in ai_holder[0].pull_model_stream(model):
                await websocket.send_json(
                    {
                        "type": "progress",
                        "status": progress.status,
                        "digest": progress.digest,
                        "total": progress.total,
                        "completed": progress.completed,
                        "percent": progress.percent,
                    }
                )
            await websocket.send_json({"type": "done", "model": model})
        except WebSocketDisconnect:
            return
        except Exception as exc:
            await websocket.send_json({"type": "error", "error": str(exc)})

    @app.post("/api/ai/config")
    async def update_ai_config(payload: dict[str, Any] = Body(...)) -> dict[str, Any]:
        ai = runtime.config.ai
        provider = payload.get("provider", ai.provider)
        if provider != "ollama":
            raise HTTPException(status_code=400, detail="Only the ollama provider is supported")
        base_url = str(payload.get("base_url", ai.base_url)).strip().rstrip("/")
        model = str(payload.get("model", ai.model)).strip()
        token_env = str(payload.get("token_env", ai.token_env)).strip()
        if not base_url:
            raise HTTPException(status_code=400, detail="'base_url' is required")
        if not model:
            raise HTTPException(status_code=400, detail="'model' is required")
        updated_ai = replace(
            ai,
            enabled=bool(payload.get("enabled", ai.enabled)),
            provider=provider,
            base_url=base_url,
            model=model,
            token_env=token_env,
            allow_remote=bool(payload.get("allow_remote", ai.allow_remote)),
        )
        runtime.config = replace(runtime.config, ai=updated_ai)
        ai_holder[0] = AIService(runtime.config)
        if config_path is not None:
            write_config(runtime.config, config_path)
        return await ai_holder[0].status()

    @app.get("/api/events")
    async def events(limit: int = 100) -> dict[str, Any]:
        items = [event_to_dict(event) for event in runtime.event_store.latest_events(limit=_bounded_limit(limit, 500))]
        return {"events": items}

    @app.get("/api/events/{event_id}")
    async def event_detail(event_id: str) -> dict[str, Any]:
        event = runtime.event_store.get_event(event_id)
        if event is None:
            raise HTTPException(status_code=404, detail="Event not found")
        return {"event": event_to_dict(event)}

    @app.get("/api/anomaly/{service}/status")
    async def anomaly_service_status(service: str, window_hours: int = 24) -> dict[str, Any]:
        if not runtime.config.anomaly_detection.enabled:
            return {"enabled": False, "service_id": service, "status": "disabled", "buckets": []}
        window = timedelta(hours=max(1, min(int(window_hours), 168)))
        events = list(runtime.event_store.query_by_service(service, window, limit=20000))
        payload = build_anomaly_service_status(
            service,
            events,
            runtime.baseline_store,
            config=runtime.config.anomaly_detection,
            now=utc_now(),
            reconcile=True,
        )
        data = anomaly_service_status_to_json(payload)
        data["enabled"] = True
        return data

    @app.get("/api/logs")
    async def logs(
        limit: int = 100,
        service: str | None = None,
        severity: int | None = None,
        search: str | None = None,
        source_type: str | None = None,
    ) -> dict[str, Any]:
        window_end = utc_now()
        window_start = window_end - timedelta(hours=24)
        severities = None
        if severity is not None:
            severities = {item for item in Severity if int(item) >= max(0, min(4, severity))}
        filters = EventFilter(
            service_ids={service} if service else None,
            severities=severities,
            message_contains=search.strip() if search and search.strip() else None,
        )
        matched = []
        for event in runtime.event_store.query_time_range(
            window_start,
            window_end,
            filters=filters,
            limit=_bounded_limit(limit, 2000),
        ):
            if source_type and event.source_ref.source_type != source_type:
                continue
            matched.append(event_to_dict(event))
        return {"logs": matched, "limit": _bounded_limit(limit, 2000)}

    @app.get("/api/incidents")
    async def incidents() -> dict[str, Any]:
        return {"incidents": [_incident_to_dict(item) for item in _active_incidents(runtime)]}

    @app.get("/api/incidents/{incident_id}")
    async def incident(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
        events_for_incident = [event for event in events_for_incident if event is not None]
        return {
            "incident": _incident_to_dict(item),
            "events": [event_to_dict(event) for event in events_for_incident],
            "clusters": runtime.incident_store.get_clusters(incident_id),
            "hypotheses": [
                _hypothesis_to_dict(hypothesis)
                for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
            ],
        }

    @app.get("/api/incidents/{incident_id}/ai-trace")
    async def incident_ai_trace(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
        events_for_incident = [event for event in events_for_incident if event is not None]
        hypotheses = [
            _hypothesis_to_dict(hypothesis)
            for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
        ]
        top_hypothesis = hypotheses[0] if hypotheses else None
        supporting_ids = set(top_hypothesis.get("supporting_events", []) if top_hypothesis else [])
        contradicting_ids = set(top_hypothesis.get("contradicting_events", []) if top_hypothesis else [])
        trace_events = [
            _ai_trace_event(event, event.event_id in supporting_ids, event.event_id in contradicting_ids)
            for event in events_for_incident[:30]
        ]
        explanation = runtime.incident_store.get_latest_explanation(incident_id)
        stored_trace = runtime.incident_store.get_latest_ai_trace(incident_id, "explain")
        allowed_labels = list(stored_trace.allowed_fields) if stored_trace else list(EXPLAIN_INCIDENT_ALLOWED_FIELDS)
        blocked_observed = list(stored_trace.blocked_fields) if stored_trace else []
        static_blocked = [
            "raw secrets",
            "environment variable values",
            "IP addresses",
            "full file paths",
            "request bodies",
            "raw structured-data values that may contain sensitive data",
        ]
        return {
            "incident": _incident_to_dict(item),
            "provider": await ai_holder[0].status(),
            "prompt_contract": {
                "allowed": allowed_labels,
                "blocked": sorted(set(static_blocked + blocked_observed)),
            },
            "prompt_audit": {
                "sanitized_system_prompt": stored_trace.sanitized_system_prompt if stored_trace else "",
                "sanitized_user_prompt": stored_trace.sanitized_user_prompt if stored_trace else "",
                "allowed_fields": allowed_labels,
                "blocked_fields": blocked_observed,
                "raw_logs_sent": False,
            },
            "top_hypothesis": top_hypothesis,
            "included_events": trace_events,
            "redaction": {
                "raw_logs_sent": False,
                "structured_values_limited": True,
                "max_events": runtime.config.ai.max_context_events,
            },
            "last_explanation": _explanation_to_dict(explanation) if explanation is not None else None,
        }

    @app.get("/api/ai/trace/{incident_id}")
    async def ai_trace_public_path(incident_id: str) -> dict[str, Any]:
        return await incident_ai_trace(incident_id)

    @app.get("/api/incidents/{incident_id}/events")
    async def incident_events(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
        return {"events": [event_to_dict(event) for event in events_for_incident if event is not None]}

    @app.get("/api/incidents/{incident_id}/hypotheses")
    async def incident_hypotheses(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        return {
            "hypotheses": [
                _hypothesis_to_dict(item)
                for item in runtime.incident_store.get_hypotheses(incident_id)
            ]
        }

    @app.post("/api/incidents/{incident_id}/feedback")
    async def incident_feedback(incident_id: str, payload: dict[str, Any] = Body(...)) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        hypotheses = runtime.incident_store.get_hypotheses(incident_id)
        if not hypotheses:
            raise HTTPException(status_code=400, detail="No hypotheses recorded for this incident")
        was_correct = bool(payload.get("was_correct", True))
        correct_id = payload.get("correct_hypothesis_id")
        if was_correct and not correct_id:
            correct_id = hypotheses[0].hypothesis_id
        if not was_correct and not correct_id:
            feedback_type = "none_correct"
        else:
            feedback_type = "confirmed"
        feedback = IncidentFeedback(
            incident_id=incident_id,
            resolved_at=utc_now(),
            correct_hypothesis_id=str(correct_id) if correct_id else None,
            feedback_type=feedback_type,
            operator_notes=str(payload.get("notes") or ""),
        )
        if runtime.config.scoring.tuning.enabled:
            weight_state = runtime.weight_store.load()
            update_weights(
                weight_state,
                feedback,
                hypotheses,
                tuning=runtime.config.scoring.tuning,
            )
            runtime.weight_store.save(weight_state)
        if feedback_type != "none_correct":
            cal_model = runtime.calibration_store.load()
            update_calibration(
                cal_model,
                feedback,
                hypotheses,
                min_samples=int(runtime.config.calibration.min_samples_per_bucket),
            )
            runtime.calibration_store.save(cal_model)
        item_after = runtime.incident_store.get_incident(incident_id)
        if item_after is not None:
            await live_hub.broadcast("incident_updated", _incident_to_dict(item_after))
        return {"stored": True, "feedback_type": feedback_type, "correct_hypothesis_id": correct_id}

    @app.get("/api/incidents/{incident_id}/clusters")
    async def incident_clusters(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        return {"clusters": runtime.incident_store.get_clusters(incident_id)}

    @app.get("/api/incidents/{incident_id}/explanation")
    async def incident_explanation(incident_id: str, request: Request) -> dict[str, Any]:
        _enforce_explain_rate(request)
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
        events_for_incident = [event for event in events_for_incident if event is not None]
        hypotheses = [
            _hypothesis_to_dict(hypothesis)
            for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
        ]
        incident_payload = _incident_to_dict(item)
        hyp_h, evt_h = explanation_cache_key_hashes(hypotheses, events_for_incident)
        cached = runtime.incident_store.get_cached_explanation(incident_id, hyp_h, evt_h)
        if cached is not None:
            if item.state == IncidentState.INVESTIGATING:
                runtime.incident_store.transition_state(
                    incident_id,
                    IncidentState.EXPLAINED,
                    "explanation persisted",
                )
            exp_dict = _explanation_to_dict(cached)
            await live_hub.broadcast(
                "explanation_ready",
                {"incident_id": incident_id, "explanation": exp_dict},
            )
            return {"explanation": exp_dict}
        explanation_job = await ai_holder[0].explain(incident_payload, hypotheses, events_for_incident)
        if explanation_job is None:
            explanation_result = explanations.generate(incident_payload, hypotheses, events_for_incident)
        else:
            explanation_payload, prompt_trace = explanation_job
            explanation_result = explanation_result_from_dict(explanation_payload)
            if prompt_trace is not None:
                _persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
        runtime.incident_store.add_explanation(explanation_result)
        item_after = runtime.incident_store.get_incident(incident_id)
        if item_after is not None and item_after.state == IncidentState.INVESTIGATING:
            runtime.incident_store.transition_state(
                incident_id,
                IncidentState.EXPLAINED,
                "explanation persisted",
            )
        exp_dict = _explanation_to_dict(explanation_result)
        await live_hub.broadcast("explanation_ready", {"incident_id": incident_id, "explanation": exp_dict})
        return {"explanation": exp_dict}

    @app.post("/api/incidents/{incident_id}/chat")
    async def incident_chat(
        incident_id: str,
        request: Request,
        payload: dict[str, Any] = Body(...),
    ) -> dict[str, Any]:
        _enforce_chat_rate(request)
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        question = payload.get("question")
        if not question:
            raise HTTPException(status_code=400, detail="'question' is required")
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
        events_for_incident = [event for event in events_for_incident if event is not None]
        hypotheses = [
            _hypothesis_to_dict(hypothesis)
            for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
        ]
        prior_messages = runtime.incident_store.list_chat_messages(incident_id)
        history_rows = [{"role": message.role, "content": message.content} for message in prior_messages]
        runtime.incident_store.append_chat_message(incident_id, "user", str(question))
        chat_payload, prompt_trace = await ai_holder[0].chat(
            str(question),
            _incident_to_dict(item),
            hypotheses,
            events_for_incident,
            history=history_rows,
        )
        runtime.incident_store.append_chat_message(incident_id, "assistant", str(chat_payload.get("answer") or ""))
        if prompt_trace is not None:
            _persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
        return {key: value for key, value in chat_payload.items() if key != "_trace"}

    @app.websocket("/api/incidents/{incident_id}/chat/stream")
    async def incident_chat_stream(incident_id: str, websocket: WebSocket) -> None:
        await websocket.accept()
        try:
            payload = await websocket.receive_json()
            item = runtime.incident_store.get_incident(incident_id)
            if item is None:
                await websocket.send_json({"type": "error", "error": "Incident not found"})
                return
            question = str(payload.get("question") or "").strip()
            if not question:
                await websocket.send_json({"type": "error", "error": "'question' is required"})
                return
            events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
            events_for_incident = [event for event in events_for_incident if event is not None]
            hypotheses = [
                _hypothesis_to_dict(hypothesis)
                for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
            ]
            prior_messages = runtime.incident_store.list_chat_messages(incident_id)
            history_rows = [{"role": message.role, "content": message.content} for message in prior_messages]
            runtime.incident_store.append_chat_message(incident_id, "user", question)
            stream_iter, prompt_trace = ai_holder[0].incident_chat_stream(
                question,
                _incident_to_dict(item),
                hypotheses,
                events_for_incident,
                history=history_rows,
            )
            aggregated: list[str] = []
            async for chunk in stream_iter:
                aggregated.append(chunk.content)
                await websocket.send_json({"type": "token", "content": chunk.content, "done": chunk.done})
            full_text = "".join(aggregated)
            answer_text = chat_answer_from_model_output(full_text, hypotheses)
            runtime.incident_store.append_chat_message(incident_id, "assistant", answer_text)
            if prompt_trace is not None:
                _persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
            await websocket.send_json({"type": "done"})
        except WebSocketDisconnect:
            return
        except Exception as exc:
            await websocket.send_json({"type": "error", "error": str(exc)})

    @app.get("/api/incidents/{incident_id}/chat/messages")
    async def incident_chat_messages(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        messages = runtime.incident_store.list_chat_messages(incident_id)
        return {
            "messages": [
                {
                    "message_id": message.message_id,
                    "role": message.role,
                    "content": message.content,
                    "created_at": message.created_at,
                    "schema_version": message.schema_version,
                }
                for message in messages
            ]
        }

    @app.get("/api/search/natural")
    async def natural_language_event_search(
        q: str,
        window_hours: int = 24,
        limit: int = 200,
    ) -> dict[str, Any]:
        text = q.strip()
        if not text:
            raise HTTPException(status_code=400, detail="q is required")
        try:
            payload = await ai_holder[0].natural_language_search(
                text,
                runtime.event_store,
                window_hours=window_hours,
                limit=limit,
            )
        except OllamaError as exc:
            raise HTTPException(status_code=503, detail=str(exc)) from exc
        threshold = float(runtime.config.ai.nl_search_min_confidence)
        if payload["confidence"] < threshold:
            raise HTTPException(
                status_code=422,
                detail={
                    "reason": "low_confidence",
                    "confidence": payload["confidence"],
                    "suggestions": payload["suggestions"],
                    "filter": payload["filter"],
                },
            )
        return payload

    @app.post("/api/incidents/{incident_id}/resolve")
    async def resolve_incident(
        incident_id: str,
        payload: dict[str, Any] | None = Body(default=None),
    ) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        feedback = payload or {}
        runtime.incident_store.resolve_incident(
            incident_id,
            ResolutionInfo(
                resolved_by=str(feedback.get("resolved_by", "operator")),
                correct_hypothesis_id=feedback.get("correct_hypothesis_id"),
                feedback_type=str(feedback.get("feedback_type", "confirmed")),
                notes=feedback.get("notes"),
                resolved_at=utc_now(),
            ),
        )
        await live_hub.broadcast(
            "incident_resolved",
            {"incident_id": incident_id, "reason": "operator_resolve"},
        )
        return {"resolved": True, "feedback": feedback}

    @app.get("/api/services")
    async def services() -> dict[str, Any]:
        return {"services": _service_health(runtime.event_store.list_services(), _active_incidents(runtime))}

    @app.get("/api/services/{service_id}")
    async def service_detail(service_id: str, limit: int = 100) -> dict[str, Any]:
        events_for_service = list(
            runtime.event_store.query_by_service(service_id, timedelta(hours=24), limit=_bounded_limit(limit, 500))
        )
        active_incidents = [
            _incident_to_dict(item)
            for item in _active_incidents(runtime)
            if service_id in item.affected_services or service_id == item.primary_service
        ]
        services = _service_health(runtime.event_store.list_services(), _active_incidents(runtime))
        service = next((item for item in services if item["service_id"] == service_id), None)
        if service is None:
            raise HTTPException(status_code=404, detail="Service not found")
        return {
            "service": service,
            "events": [event_to_dict(event) for event in events_for_service],
            "incidents": active_incidents,
            "severity_counts": _severity_counts(events_for_service),
        }

    @app.get("/api/services/{service_id}/events")
    async def service_events(service_id: str, limit: int = 100) -> dict[str, Any]:
        events_for_service = list(
            runtime.event_store.query_by_service(service_id, timedelta(hours=24), limit=_bounded_limit(limit, 500))
        )
        return {"events": [event_to_dict(event) for event in events_for_service]}

    @app.get("/api/topology")
    async def topology() -> dict[str, Any]:
        return {"edges": runtime.service_graph.edges()}

    @app.post("/api/topology/edges")
    async def add_topology_edge(payload: dict[str, Any] = Body(...)) -> dict[str, Any]:
        source = payload.get("source")
        target = payload.get("target")
        if not source or not target:
            raise HTTPException(status_code=400, detail="'source' and 'target' are required")
        relation_type = payload.get("relation_type", "depends_on")
        runtime.add_topology_relation(source, target, relation_type)
        return {"added": True, "edge": {"source": source, "target": target, "relation_type": relation_type}}

    @app.get("/api/incidents/{incident_id}/state-log")
    async def incident_state_log(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        entries = runtime.incident_store.list_state_log(incident_id)
        return {
            "incident_id": incident_id,
            "entries": [
                {
                    "log_id": entry.log_id,
                    "old_state": entry.old_state,
                    "new_state": entry.new_state,
                    "changed_at": to_iso(entry.changed_at),
                    "reason": entry.reason,
                }
                for entry in entries
            ],
        }

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
                        _hypothesis_to_dict(hypothesis)
                        for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
                    ]
                    prior_messages = runtime.incident_store.list_chat_messages(incident_id)
                    history_rows = [{"role": message.role, "content": message.content} for message in prior_messages]
                    runtime.incident_store.append_chat_message(incident_id, "user", question)
                    stream_iter, prompt_trace = ai_holder[0].incident_chat_stream(
                        question,
                        _incident_to_dict(item),
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
                        _persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
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
                        _hypothesis_to_dict(hypothesis)
                        for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
                    ]
                    incident_payload = _incident_to_dict(item)
                    hyp_h, evt_h = explanation_cache_key_hashes(hypotheses, events_for_incident)
                    cached = runtime.incident_store.get_cached_explanation(incident_id, hyp_h, evt_h)
                    if cached is not None:
                        if item.state == IncidentState.INVESTIGATING:
                            runtime.incident_store.transition_state(
                                incident_id,
                                IncidentState.EXPLAINED,
                                "explanation persisted",
                            )
                        exp_dict = _explanation_to_dict(cached)
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
                                _persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
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
                                _persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
                    runtime.incident_store.add_explanation(explanation_result)
                    item_after = runtime.incident_store.get_incident(incident_id)
                    if item_after is not None and item_after.state == IncidentState.INVESTIGATING:
                        runtime.incident_store.transition_state(
                            incident_id,
                            IncidentState.EXPLAINED,
                            "explanation persisted",
                        )
                    exp_dict = _explanation_to_dict(explanation_result)
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

    return app


def _active_incidents(runtime: InferraRuntime) -> list[Incident]:
    return runtime.incident_store.list_incidents(
        state=[IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED],
        limit=200,
    )


def _incident_to_dict(item: Incident) -> dict[str, Any]:
    updated_at = item.updated_at or item.created_at
    return {
        "incident_id": item.incident_id,
        "state": item.state.value,
        "created_at": to_iso(item.created_at),
        "updated_at": to_iso(updated_at),
        "severity": int(item.severity),
        "primary_service": item.primary_service,
        "affected_services": sorted(item.affected_services),
        "time_range_start": to_iso(item.time_range[0]),
        "time_range_end": to_iso(item.time_range[1]),
        "event_count": len(item.events),
    }


def _hypothesis_to_dict(item: ScoredHypothesis) -> dict[str, Any]:
    return {
        "hypothesis_id": item.hypothesis_id,
        "rank": item.rank,
        "cause_type": item.cause_type.value,
        "description": item.description,
        "total_score": item.total_score,
        "score_breakdown": {
            "temporal_alignment": item.score_breakdown.temporal_alignment,
            "correlation_strength": item.score_breakdown.correlation_strength,
            "frequency_weight": item.score_breakdown.frequency_weight,
            "dependency_proximity": item.score_breakdown.dependency_proximity,
            "evidence_coverage": item.score_breakdown.evidence_coverage,
            "anomaly_severity": item.score_breakdown.anomaly_severity,
        },
        "supporting_events": list(item.supporting_events),
        "contradicting_events": list(item.contradicting_events),
        "affected_services": sorted(item.affected_services),
        "suggested_checks": list(item.suggested_checks),
        "confidence_label": item.confidence_label,
        "is_valid": item.is_valid,
        "invalidation_reasons": list(item.invalidation_reasons),
    }


def _explanation_to_dict(item: ExplanationResult) -> dict[str, Any]:
    return {
        "explanation_id": item.explanation_id,
        "incident_id": item.incident_id,
        "summary": item.summary,
        "primary_hypothesis_text": item.primary_hypothesis_text,
        "evidence_narrative": item.evidence_narrative,
        "timeline_narrative": item.timeline_narrative,
        "alternative_explanations": list(item.alternative_explanations),
        "suggested_actions": list(item.suggested_actions),
        "uncertainty_notes": list(item.uncertainty_notes),
        "generation_model": item.generation_model,
        "guardrail_violations": list(item.guardrail_violations),
        "hypotheses_hash": item.hypotheses_hash,
        "events_hash_head": item.events_hash_head,
        "schema_version": item.schema_version,
        "quality": item.quality,
    }


def _explanation_from_dict(payload: dict[str, Any]) -> ExplanationResult:
    return ExplanationResult(
        incident_id=str(payload["incident_id"]),
        summary=str(payload["summary"]),
        primary_hypothesis_text=str(payload["primary_hypothesis_text"]),
        evidence_narrative=str(payload.get("evidence_narrative", "")),
        timeline_narrative=str(payload.get("timeline_narrative", "")),
        alternative_explanations=list(payload.get("alternative_explanations") or []),
        suggested_actions=list(payload.get("suggested_actions") or []),
        uncertainty_notes=list(payload.get("uncertainty_notes") or []),
        generation_model=str(payload.get("generation_model", "template_fallback")),
        guardrail_violations=list(payload.get("guardrail_violations") or []),
        explanation_id=str(payload.get("explanation_id") or ""),
        hypotheses_hash=str(payload.get("hypotheses_hash") or ""),
        events_hash_head=str(payload.get("events_hash_head") or ""),
        schema_version=int(payload.get("schema_version") or 1),
        quality=str(payload.get("quality") or "ok"),
    )


def _bounded_limit(limit: int, maximum: int) -> int:
    return max(1, min(maximum, int(limit)))


def _severity_counts(events: list[NormalizedEvent]) -> dict[str, int]:
    counts = {item.name.lower(): 0 for item in Severity}
    for event in events:
        counts[event.severity.name.lower()] += 1
    return counts


def _event_rate(events: list[NormalizedEvent]) -> list[dict[str, Any]]:
    buckets: dict[str, dict[str, Any]] = {}
    for event in events:
        label = to_iso(event.timestamp.replace(second=0, microsecond=0))
        if label not in buckets:
            buckets[label] = {"timestamp": label, "total": 0, "warn": 0, "error": 0, "critical": 0}
        buckets[label]["total"] += 1
        if event.severity >= Severity.WARN:
            buckets[label][event.severity.name.lower()] += 1
    return [buckets[key] for key in sorted(buckets)][-60:]


def _service_health(services: list[dict[str, Any]], incidents: list[Incident]) -> list[dict[str, Any]]:
    incident_services: dict[str, list[dict[str, Any]]] = {}
    for incident in incidents:
        payload = _incident_to_dict(incident)
        incident_service_ids = set(incident.affected_services) | (
            {incident.primary_service} if incident.primary_service else set()
        )
        for service in incident_service_ids:
            incident_services.setdefault(service, []).append(payload)

    enriched = []
    for service in services:
        event_count = int(service.get("event_count", 0))
        error_count = int(service.get("error_count", 0))
        related_incidents = incident_services.get(str(service["service_id"]), [])
        error_ratio = error_count / event_count if event_count else 0.0
        if related_incidents and max(item["severity"] for item in related_incidents) >= int(Severity.ERROR):
            status = "critical"
        elif related_incidents or error_ratio >= 0.25:
            status = "degraded"
        elif error_count:
            status = "elevated"
        else:
            status = "healthy"
        enriched.append(
            {
                **service,
                "status": status,
                "error_ratio": round(error_ratio, 3),
                "active_incidents": related_incidents,
            }
        )
    return enriched


def _ai_trace_event(event: NormalizedEvent, supporting: bool, contradicting: bool) -> dict[str, Any]:
    return {
        "event_id": event.event_id,
        "timestamp": to_iso(event.timestamp),
        "service_id": event.service_id,
        "severity": event.severity.name.lower(),
        "summary": event.message[:240],
        "tags": sorted(event.tags),
        "quality": event.quality.overall,
        "supporting": supporting,
        "contradicting": contradicting,
        "source_type": event.source_ref.source_type,
    }

