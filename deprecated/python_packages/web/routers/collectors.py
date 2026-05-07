"""Collector control routes: list, start/stop all, start/stop one.

These endpoints manage Inferra's own collectors. They never touch observed
systems, only Inferra's collector lifecycle.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from fastapi import APIRouter, HTTPException, Query

from inferra_legacy.app import InferraRuntime
from web.live_hub import LiveHub


@dataclass(frozen=True)
class CollectorsDeps:
    runtime: InferraRuntime
    live_hub: LiveHub


def build_collectors_router(deps: CollectorsDeps) -> APIRouter:
    router = APIRouter(prefix="/api/collectors")
    runtime = deps.runtime
    live_hub = deps.live_hub

    def _snapshot() -> dict[str, Any]:
        return {"collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()}

    @router.get("")
    async def list_collectors() -> dict[str, Any]:
        return _snapshot()

    @router.post("/start")
    async def start_collectors() -> dict[str, Any]:
        await runtime.start_collectors()
        await live_hub.broadcast("collector_health", _snapshot())
        return {"started": True, **_snapshot()}

    @router.post("/stop")
    async def stop_collectors() -> dict[str, Any]:
        await runtime.stop_collectors()
        await live_hub.broadcast("collector_health", _snapshot())
        return {"stopped": True, **_snapshot()}

    @router.post("/one/start")
    async def start_one_collector(collector_id: str = Query(..., min_length=1)) -> dict[str, Any]:
        ok = await runtime.start_collector(collector_id.strip())
        if not ok:
            raise HTTPException(status_code=404, detail="collector not found")
        await live_hub.broadcast("collector_health", _snapshot())
        return {"started": True, "collector_id": collector_id, "collectors": runtime.collector_health()}

    @router.post("/one/stop")
    async def stop_one_collector(collector_id: str = Query(..., min_length=1)) -> dict[str, Any]:
        ok = await runtime.stop_collector(collector_id.strip())
        if not ok:
            raise HTTPException(status_code=404, detail="collector not found")
        await live_hub.broadcast("collector_health", _snapshot())
        return {"stopped": True, "collector_id": collector_id, "collectors": runtime.collector_health()}

    return router
