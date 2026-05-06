"""Workspace intelligence API: discovered projects + service mappings + per-project inspection."""

from __future__ import annotations

from dataclasses import dataclass, replace
from typing import Any

from fastapi import APIRouter, Body, HTTPException, Query

from app import InferraRuntime
from config import WorkspaceServiceMapping, write_config
from runtime.workspace_map import build_workspace_map, inspect_project
from runtime.workspace_scan import discover_code_projects, projects_to_json


@dataclass(frozen=True)
class WorkspaceDeps:
    runtime: InferraRuntime
    config_path: str | None = None


def build_workspace_router(deps: WorkspaceDeps) -> APIRouter:
    router = APIRouter(prefix="/api/workspace")
    runtime = deps.runtime

    @router.get("/projects")
    async def workspace_projects(
        max_depth: int = Query(default=3, ge=1, le=10),
        max_results: int = Query(default=50, ge=1, le=500),
    ) -> dict[str, Any]:
        hits = discover_code_projects(max_depth=max_depth, max_results=max_results)
        return {"projects": projects_to_json(hits)}

    @router.get("/map")
    async def workspace_map() -> dict[str, Any]:
        services = [
            str(item.get("service_id"))
            for item in runtime.event_store.list_services()
            if item.get("service_id")
        ]
        return build_workspace_map(runtime.config, services=services)

    @router.get("/services")
    async def workspace_services() -> dict[str, Any]:
        services = [
            str(item.get("service_id"))
            for item in runtime.event_store.list_services()
            if item.get("service_id")
        ]
        mapped = build_workspace_map(runtime.config, services=services)
        return {
            "service_mappings": mapped.get("service_mappings", []),
            "unmapped_services": mapped.get("unmapped_services", []),
        }

    @router.get("/inspect")
    async def workspace_inspect(path: str = Query(..., min_length=1)) -> dict[str, Any]:
        return inspect_project(path)

    @router.post("/mappings")
    async def add_mapping(payload: dict[str, Any] = Body(...)) -> dict[str, Any]:
        service_id = str(payload.get("service_id") or "").strip()
        project_path = str(payload.get("project_path") or "").strip()
        if not service_id or not project_path:
            raise HTTPException(status_code=400, detail="service_id and project_path are required")
        confidence = float(payload.get("confidence", 1.0))
        confidence = max(0.0, min(1.0, confidence))
        notes = str(payload.get("notes") or "")
        existing = list(runtime.config.workspace.service_mappings)
        existing = [m for m in existing if not (m.service_id == service_id and m.project_path == project_path)]
        existing.append(
            WorkspaceServiceMapping(
                service_id=service_id,
                project_path=project_path,
                confidence=confidence,
                source="user",
                notes=notes,
            )
        )
        new_workspace = replace(runtime.config.workspace, service_mappings=existing)
        runtime.config = replace(runtime.config, workspace=new_workspace)
        if deps.config_path is not None:
            write_config(runtime.config, deps.config_path)
        return {
            "stored": True,
            "service_id": service_id,
            "project_path": project_path,
            "confidence": confidence,
            "persisted": deps.config_path is not None,
        }

    return router
