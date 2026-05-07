"""AI provider routes: status, models, pull stream, config, trace.

These manage the AI provider configuration and registry. The investigation
endpoints live in `web.routers.investigate`. Doctor lives there too.
The `pull` endpoint is a websocket that streams ollama pull progress.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

from fastapi import APIRouter, Body, HTTPException, WebSocket, WebSocketDisconnect

from ai.prompts import EXPLAIN_INCIDENT_ALLOWED_FIELDS
from ai.service import AIService
from inferra_legacy.app import InferraRuntime
from config import write_config
from dataclasses import replace
from web._shared import (
    ai_trace_event,
    explanation_to_dict,
    hypothesis_to_dict,
    incident_to_dict,
)


@dataclass(frozen=True)
class AiDeps:
    runtime: InferraRuntime
    ai_holder: list[AIService]
    config_path: str | Path | None = None


def build_ai_router(deps: AiDeps) -> APIRouter:
    router = APIRouter()
    runtime = deps.runtime
    ai_holder = deps.ai_holder

    @router.get("/api/ai/status")
    async def ai_status() -> dict[str, Any]:
        return await ai_holder[0].status()

    @router.get("/api/ai/models")
    async def ai_models() -> dict[str, Any]:
        installed: list[str] = []
        error = None
        if runtime.config.ai.enabled:
            try:
                installed = await ai_holder[0].installed_models()
            except Exception as exc:  # provider errors should not make the UI unusable
                error = str(exc)
        return {"registry": ai_holder[0].registry(), "installed": installed, "error": error}

    @router.websocket("/api/ai/pull")
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

    @router.post("/api/ai/config")
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
        if deps.config_path is not None:
            write_config(runtime.config, deps.config_path)
        return await ai_holder[0].status()

    @router.get("/api/ai/trace/{incident_id}")
    async def ai_trace_public_path(incident_id: str) -> dict[str, Any]:
        return await _build_incident_ai_trace(runtime, ai_holder, incident_id)

    @router.get("/api/incidents/{incident_id}/ai-trace")
    async def incident_ai_trace(incident_id: str) -> dict[str, Any]:
        return await _build_incident_ai_trace(runtime, ai_holder, incident_id)

    return router


async def _build_incident_ai_trace(
    runtime: InferraRuntime,
    ai_holder: list[AIService],
    incident_id: str,
) -> dict[str, Any]:
    item = runtime.incident_store.get_incident(incident_id)
    if item is None:
        raise HTTPException(status_code=404, detail="Incident not found")
    events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
    events_for_incident = [event for event in events_for_incident if event is not None]
    hypotheses = [
        hypothesis_to_dict(hypothesis) for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
    ]
    top_hypothesis = hypotheses[0] if hypotheses else None
    supporting_ids = set(top_hypothesis.get("supporting_events", []) if top_hypothesis else [])
    contradicting_ids = set(top_hypothesis.get("contradicting_events", []) if top_hypothesis else [])
    trace_events = [
        ai_trace_event(event, event.event_id in supporting_ids, event.event_id in contradicting_ids)
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
        "incident": incident_to_dict(item),
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
        "last_explanation": explanation_to_dict(explanation) if explanation is not None else None,
    }
