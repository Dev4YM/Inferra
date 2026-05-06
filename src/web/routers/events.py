"""Event and log routes: list events, fetch one, anomaly status, log search.

Includes the natural-language event search endpoint backed by AIService.
All endpoints are read-only.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import timedelta
from typing import Any

from fastapi import APIRouter, HTTPException

from ai.ollama import OllamaError
from ai.service import AIService
from analysis.anomaly import anomaly_service_status_to_json, build_anomaly_service_status
from app import InferraRuntime
from core.enums import Severity
from core.time import utc_now
from events.models import EventFilter
from events.serialization import event_to_dict
from web._shared import bounded_limit


@dataclass(frozen=True)
class EventsDeps:
    runtime: InferraRuntime
    ai_holder: list[AIService]


def build_events_router(deps: EventsDeps) -> APIRouter:
    router = APIRouter()
    runtime = deps.runtime

    @router.get("/api/events")
    async def events(limit: int = 100) -> dict[str, Any]:
        items = [event_to_dict(event) for event in runtime.event_store.latest_events(limit=bounded_limit(limit, 500))]
        return {"events": items}

    @router.get("/api/events/{event_id}")
    async def event_detail(event_id: str) -> dict[str, Any]:
        event = runtime.event_store.get_event(event_id)
        if event is None:
            raise HTTPException(status_code=404, detail="Event not found")
        return {"event": event_to_dict(event)}

    @router.get("/api/anomaly/{service}/status")
    async def anomaly_service_status(service: str, window_hours: int = 24) -> dict[str, Any]:
        if not runtime.config.anomaly_detection.enabled:
            return {"enabled": False, "service_id": service, "status": "disabled", "buckets": []}
        window = timedelta(hours=max(1, min(int(window_hours), 168)))
        events_for_service = list(runtime.event_store.query_by_service(service, window, limit=20000))
        payload = build_anomaly_service_status(
            service,
            events_for_service,
            runtime.baseline_store,
            config=runtime.config.anomaly_detection,
            now=utc_now(),
            reconcile=True,
        )
        data = anomaly_service_status_to_json(payload)
        data["enabled"] = True
        return data

    @router.get("/api/logs")
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
            limit=bounded_limit(limit, 2000),
        ):
            if source_type and event.source_ref.source_type != source_type:
                continue
            matched.append(event_to_dict(event))
        return {"logs": matched, "limit": bounded_limit(limit, 2000)}

    @router.get("/api/search/natural")
    async def natural_language_event_search(
        q: str,
        window_hours: int = 24,
        limit: int = 200,
    ) -> dict[str, Any]:
        text = q.strip()
        if not text:
            raise HTTPException(status_code=400, detail="q is required")
        try:
            payload = await deps.ai_holder[0].natural_language_search(
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

    return router
