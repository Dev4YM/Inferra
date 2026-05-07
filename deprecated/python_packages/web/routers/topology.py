"""Topology routes: list edges, add a manual dependency edge.

Topology represents *Inferra's* observed-service dependency graph. Edits do not
touch observed systems; they only annotate the dependency model used by
correlation reasoning.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from fastapi import APIRouter, Body, HTTPException

from inferra_legacy.app import InferraRuntime


@dataclass(frozen=True)
class TopologyDeps:
    runtime: InferraRuntime


def build_topology_router(deps: TopologyDeps) -> APIRouter:
    router = APIRouter(prefix="/api/topology")
    runtime = deps.runtime

    @router.get("")
    async def topology() -> dict[str, Any]:
        return {"edges": runtime.service_graph.edges()}

    @router.post("/edges")
    async def add_topology_edge(payload: dict[str, Any] = Body(...)) -> dict[str, Any]:
        source = payload.get("source")
        target = payload.get("target")
        if not source or not target:
            raise HTTPException(status_code=400, detail="'source' and 'target' are required")
        relation_type = payload.get("relation_type", "depends_on")
        runtime.add_topology_relation(source, target, relation_type)
        return {
            "added": True,
            "edge": {"source": source, "target": target, "relation_type": relation_type},
        }

    return router
