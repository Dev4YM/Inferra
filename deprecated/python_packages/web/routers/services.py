"""Service inspection routes: health list, single service detail, recent events."""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta
from typing import Any

from fastapi import APIRouter, HTTPException

from inferra_legacy.app import InferraRuntime
from events.serialization import event_to_dict
from web._shared import (
    active_incidents,
    bounded_limit,
    incident_to_dict,
    service_health,
    severity_counts,
)


@dataclass(frozen=True)
class ServicesDeps:
    runtime: InferraRuntime


def build_services_router(deps: ServicesDeps) -> APIRouter:
    router = APIRouter(prefix="/api/services")
    runtime = deps.runtime

    @router.get("")
    async def services() -> dict[str, Any]:
        return {"services": service_health(runtime.event_store.list_services(), active_incidents(runtime))}

    @router.get("/{service_id}")
    async def service_detail(service_id: str, limit: int = 100) -> dict[str, Any]:
        events_for_service = list(
            runtime.event_store.query_by_service(service_id, timedelta(hours=24), limit=bounded_limit(limit, 500))
        )
        active = [
            incident_to_dict(item)
            for item in active_incidents(runtime)
            if service_id in item.affected_services or service_id == item.primary_service
        ]
        services_payload = service_health(runtime.event_store.list_services(), active_incidents(runtime))
        service = next((item for item in services_payload if item["service_id"] == service_id), None)
        if service is None:
            raise HTTPException(status_code=404, detail="Service not found")
        return {
            "service": service,
            "events": [event_to_dict(event) for event in events_for_service],
            "incidents": active,
            "severity_counts": severity_counts(events_for_service),
        }

    @router.get("/{service_id}/events")
    async def service_events(service_id: str, limit: int = 100) -> dict[str, Any]:
        events_for_service = list(
            runtime.event_store.query_by_service(service_id, timedelta(hours=24), limit=bounded_limit(limit, 500))
        )
        return {"events": [event_to_dict(event) for event in events_for_service]}

    return router
