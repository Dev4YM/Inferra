from __future__ import annotations

import importlib.metadata
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from fastapi import APIRouter, Body, HTTPException, Request
from fastapi.responses import PlainTextResponse

from ai import AIService
from inferra_legacy.app import InferraRuntime
from config import config_to_dict, parse_config_payload, write_config
from core.errors import ConfigError
from runtime.context import build_runtime_context_snapshot, runtime_context_to_correlation_dict
from runtime.workspace_scan import discover_code_projects, projects_to_json
from web.rate_limit import HostRateLimiter

__all__ = ["SystemRouteDeps", "build_system_router"]

DashboardPayloadBuilder = Callable[[InferraRuntime, list[AIService]], Awaitable[dict[str, Any]]]
HealthBundleBuilder = Callable[[InferraRuntime, list[AIService]], Awaitable[dict[str, Any]]]
PrometheusRenderer = Callable[[InferraRuntime], str]


@dataclass(frozen=True)
class SystemRouteDeps:
    runtime: InferraRuntime
    ai_holder: list[AIService]
    config_path: str | Path | None
    dashboard_payload: DashboardPayloadBuilder
    health_bundle: HealthBundleBuilder
    prometheus_text: PrometheusRenderer


def build_system_router(deps: SystemRouteDeps) -> APIRouter:
    router = APIRouter(prefix="/api")
    runtime = deps.runtime
    ai_holder = deps.ai_holder

    @router.get("/version")
    async def api_version() -> dict[str, Any]:
        try:
            version = importlib.metadata.version("inferra")
        except importlib.metadata.PackageNotFoundError:
            version = "0.0.0"
        return {"name": "inferra", "version": version, "api": "1"}

    @router.get("/metrics")
    async def api_metrics() -> PlainTextResponse:
        if not runtime.config.server.expose_prometheus_metrics:
            raise HTTPException(status_code=404, detail="metrics disabled")
        body = deps.prometheus_text(runtime)
        return PlainTextResponse(body, media_type="text/plain; version=0.0.4")

    @router.get("/config")
    async def get_config() -> dict[str, Any]:
        return {"config": config_to_dict(runtime.config)}

    @router.put("/config")
    async def put_config(request: Request, payload: dict[str, Any] = Body(...)) -> dict[str, Any]:
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
        srv = runtime.config.server
        request.app.state.rate_chat = HostRateLimiter(srv.rate_limit_chat_tokens_per_minute)
        request.app.state.rate_explain = HostRateLimiter(srv.rate_limit_explain_tokens_per_minute)
        request.app.state.ws_rate_chat = HostRateLimiter(srv.rate_limit_chat_tokens_per_minute)
        request.app.state.ws_rate_explain = HostRateLimiter(srv.rate_limit_explain_tokens_per_minute)
        if deps.config_path is not None:
            write_config(runtime.config, deps.config_path)
        return {"config": config_to_dict(runtime.config), "applied": True}

    @router.get("/health")
    async def health() -> dict[str, Any]:
        return await deps.health_bundle(runtime, ai_holder)

    @router.get("/dashboard")
    async def dashboard() -> dict[str, Any]:
        return await deps.dashboard_payload(runtime, ai_holder)

    @router.get("/runtime/context")
    async def runtime_context() -> dict[str, Any]:
        snap = await build_runtime_context_snapshot(process_limit=80)
        return runtime_context_to_correlation_dict(snap)

    @router.get("/overview")
    async def overview() -> dict[str, Any]:
        dash = await deps.dashboard_payload(runtime, ai_holder)
        snap = await build_runtime_context_snapshot(process_limit=60)
        rt = runtime_context_to_correlation_dict(snap)
        projects = projects_to_json(discover_code_projects(max_depth=2, max_results=25))
        health_row = dash["health"]
        degraded = bool(health_row.get("degraded"))
        incidents_n = int(health_row.get("active_incidents", 0))
        top = dash["incidents"][0] if dash["incidents"] else None
        summary_parts: list[str] = []
        if incidents_n:
            summary_parts.append(f"{incidents_n} active incident(s).")
        else:
            summary_parts.append("No active incidents.")
        if degraded:
            summary_parts.append("System is degraded; check collectors or storage.")
        if health_row.get("ai_enabled") and not health_row.get("ai_available"):
            summary_parts.append("AI observer is unavailable.")
        if top:
            summary_parts.append(f"Top incident: {top.get('primary_service', '?')} (severity {top.get('severity')}).")
        quick = {
            "headline": " ".join(summary_parts),
            "risk_level": "high" if degraded or incidents_n else "low",
            "containers_running": len(rt["containers"]),
            "process_sample_size": len(rt["processes"]),
            "code_projects_found": len(projects),
            "mode": runtime.config.experience.mode,
            "ai_role": runtime.config.experience.ai_role,
        }
        return {
            "quick_analysis": quick,
            "dashboard": dash,
            "runtime": rt,
            "workspace_projects": projects,
            "experience": {
                "mode": runtime.config.experience.mode,
                "ai_role": runtime.config.experience.ai_role,
                "suggest_safe_actions": runtime.config.experience.suggest_safe_actions,
                "execute_actions": runtime.config.experience.execute_actions,
                "show_raw_evidence_by_default": runtime.config.experience.show_raw_evidence_by_default,
            },
        }

    return router
