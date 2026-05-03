from __future__ import annotations

import json
from contextlib import asynccontextmanager
from dataclasses import replace
from datetime import timedelta
from pathlib import Path
from typing import Any

from fastapi import FastAPI, HTTPException
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles

from ai import AIService
from app import InferraRuntime
from config import InferraConfig, write_config
from core.enums import EventType, IncidentState, Severity
from core.time import to_iso, utc_now
from events.models import EventFilter, NormalizedEvent
from events.serialization import event_to_dict
from explanation import TemplateExplanationEngine


def create_app(config: InferraConfig | None = None, config_path: str | Path | None = None) -> FastAPI:
    runtime = InferraRuntime(config)
    explanations = TemplateExplanationEngine()
    ai_service = AIService(runtime.config)

    @asynccontextmanager
    async def lifespan(app: FastAPI):
        await runtime.start()
        app.state.runtime = runtime
        yield
        await runtime.stop()

    app = FastAPI(title="Inferra", version="0.1.0", lifespan=lifespan)
    static_dir = Path(__file__).parent / "static"
    app.mount("/static", StaticFiles(directory=static_dir), name="static")

    @app.get("/")
    async def index():
        return FileResponse(static_dir / "index.html")

    @app.get("/api/health")
    async def health() -> dict[str, Any]:
        active_incidents = runtime.incident_store.list_active()
        collector_health = runtime.collector_health()
        return {
            "status": "observing",
            "events_db": str(runtime.event_store.path),
            "incidents_db": str(runtime.incident_store.path),
            "active_incidents": len(active_incidents),
            "queue_depth": runtime.raw_queue.qsize(),
            "collectors": len(collector_health),
            "collector_errors": sum(int(item.get("error_count", 0)) for item in collector_health),
        }

    @app.get("/api/dashboard")
    async def dashboard() -> dict[str, Any]:
        events = runtime.event_store.latest_events(limit=500)
        incidents = runtime.incident_store.list_active()
        services = runtime.event_store.list_services()
        return {
            "health": {
                "status": "observing",
                "active_incidents": len(incidents),
                "queue_depth": runtime.raw_queue.qsize(),
                "collector_errors": sum(int(item.get("error_count", 0)) for item in runtime.collector_health()),
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
        return {"started": True, "collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()}

    @app.post("/api/collectors/stop")
    async def stop_collectors() -> dict[str, Any]:
        await runtime.stop_collectors()
        return {"stopped": True, "collectors": runtime.collector_health(), "queue_depth": runtime.raw_queue.qsize()}

    @app.get("/api/ai/status")
    async def ai_status() -> dict[str, Any]:
        return ai_service.status()

    @app.get("/api/ai/models")
    async def ai_models() -> dict[str, Any]:
        installed: list[str] = []
        error = None
        if runtime.config.ai.enabled:
            try:
                installed = ai_service.installed_models()
            except Exception as exc:  # provider errors should not make the UI unusable
                error = str(exc)
        return {"registry": ai_service.registry(), "installed": installed, "error": error}

    @app.post("/api/ai/config")
    async def update_ai_config(payload: dict[str, Any]) -> dict[str, Any]:
        nonlocal ai_service
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
            allow_remote=not _is_local_base_url(base_url),
        )
        runtime.config = replace(runtime.config, ai=updated_ai)
        ai_service = AIService(runtime.config)
        if config_path is not None:
            write_config(runtime.config, config_path)
        return ai_service.status()

    @app.post("/api/ingest")
    async def ingest(payload: dict[str, Any]) -> dict[str, Any]:
        message = payload.get("message")
        if not message:
            raise HTTPException(status_code=400, detail="'message' is required")
        raw_payload = json.dumps(
            {
                "timestamp": payload.get("timestamp"),
                "service": payload.get("service", "app"),
                "level": payload.get("level", "info"),
                "message": message,
                "context": payload.get("context", {}),
            }
        )
        event_id = await runtime.ingest_payload(
            raw_payload,
            source_type="app",
            source_id="app://http",
            metadata={"service_id": payload.get("service", "app")},
        )
        return {"stored": event_id is not None, "event_id": event_id}

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
            limit=_bounded_limit(limit, 500),
        ):
            if source_type and event.source_ref.source_type != source_type:
                continue
            matched.append(event_to_dict(event))
        return {"logs": matched, "limit": _bounded_limit(limit, 500)}

    @app.get("/api/incidents")
    async def incidents() -> dict[str, Any]:
        return {"incidents": [_incident_to_dict(item) for item in runtime.incident_store.list_active()]}

    @app.get("/api/incidents/{incident_id}")
    async def incident(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        event_ids = runtime.incident_store.event_ids_for_incident(incident_id)
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in event_ids]
        events_for_incident = [event for event in events_for_incident if event is not None]
        return {
            "incident": _incident_to_dict(item),
            "events": [event_to_dict(event) for event in events_for_incident],
            "clusters": runtime.incident_store.clusters_for_incident(incident_id),
            "hypotheses": runtime.incident_store.hypotheses_for_incident(incident_id),
        }

    @app.get("/api/incidents/{incident_id}/ai-trace")
    async def incident_ai_trace(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        event_ids = runtime.incident_store.event_ids_for_incident(incident_id)
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in event_ids]
        events_for_incident = [event for event in events_for_incident if event is not None]
        hypotheses = runtime.incident_store.hypotheses_for_incident(incident_id)
        top_hypothesis = hypotheses[0] if hypotheses else None
        supporting_ids = set(top_hypothesis.get("supporting_events", []) if top_hypothesis else [])
        contradicting_ids = set(top_hypothesis.get("contradicting_events", []) if top_hypothesis else [])
        trace_events = [
            _ai_trace_event(event, event.event_id in supporting_ids, event.event_id in contradicting_ids)
            for event in events_for_incident[:30]
        ]
        explanation = runtime.incident_store.latest_explanation(incident_id)
        return {
            "incident": _incident_to_dict(item),
            "provider": ai_service.status(),
            "prompt_contract": {
                "allowed": [
                    "incident id",
                    "time range",
                    "canonical service ids",
                    "severity names",
                    "sanitized event summaries",
                    "ranked hypotheses",
                    "score breakdowns",
                    "suggested checks",
                ],
                "blocked": [
                    "raw secrets",
                    "environment variable values",
                    "IP addresses",
                    "full file paths",
                    "request bodies",
                    "raw structured-data values that may contain sensitive data",
                ],
            },
            "top_hypothesis": top_hypothesis,
            "included_events": trace_events,
            "redaction": {
                "raw_logs_sent": False,
                "structured_values_limited": True,
                "max_events": 30,
            },
            "last_explanation": explanation,
        }

    @app.get("/api/incidents/{incident_id}/events")
    async def incident_events(incident_id: str) -> dict[str, Any]:
        event_ids = runtime.incident_store.event_ids_for_incident(incident_id)
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in event_ids]
        return {"events": [event_to_dict(event) for event in events_for_incident if event is not None]}

    @app.get("/api/incidents/{incident_id}/hypotheses")
    async def incident_hypotheses(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        return {"hypotheses": runtime.incident_store.hypotheses_for_incident(incident_id)}

    @app.get("/api/incidents/{incident_id}/clusters")
    async def incident_clusters(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        return {"clusters": runtime.incident_store.clusters_for_incident(incident_id)}

    @app.get("/api/incidents/{incident_id}/explanation")
    async def incident_explanation(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        cached = runtime.incident_store.latest_explanation(incident_id)
        if cached is not None:
            return {"explanation": cached}
        event_ids = runtime.incident_store.event_ids_for_incident(incident_id)
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in event_ids]
        events_for_incident = [event for event in events_for_incident if event is not None]
        hypotheses = runtime.incident_store.hypotheses_for_incident(incident_id)
        incident_payload = _incident_to_dict(item)
        explanation = ai_service.explain(incident_payload, hypotheses, events_for_incident)
        if explanation is None:
            explanation = explanations.generate(incident_id, hypotheses, events_for_incident)
        runtime.incident_store.save_explanation(incident_id, explanation)
        return {"explanation": explanation}

    @app.post("/api/incidents/{incident_id}/chat")
    async def incident_chat(incident_id: str, payload: dict[str, Any]) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        question = payload.get("question")
        if not question:
            raise HTTPException(status_code=400, detail="'question' is required")
        event_ids = runtime.incident_store.event_ids_for_incident(incident_id)
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in event_ids]
        events_for_incident = [event for event in events_for_incident if event is not None]
        hypotheses = runtime.incident_store.hypotheses_for_incident(incident_id)
        return ai_service.chat(question, _incident_to_dict(item), hypotheses, events_for_incident)

    @app.post("/api/incidents/{incident_id}/resolve")
    async def resolve_incident(incident_id: str, payload: dict[str, Any] | None = None) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        ok = runtime.incident_store.update_incident_state(incident_id, IncidentState.RESOLVED)
        return {"resolved": ok, "feedback": payload or {}}

    @app.get("/api/services")
    async def services() -> dict[str, Any]:
        return {"services": _service_health(runtime.event_store.list_services(), runtime.incident_store.list_active())}

    @app.get("/api/services/{service_id}")
    async def service_detail(service_id: str, limit: int = 100) -> dict[str, Any]:
        events_for_service = list(
            runtime.event_store.query_by_service(service_id, timedelta(hours=24), limit=_bounded_limit(limit, 500))
        )
        active_incidents = [
            _incident_to_dict(item)
            for item in runtime.incident_store.list_active()
            if service_id in item.affected_services or service_id == item.primary_service
        ]
        services = _service_health(runtime.event_store.list_services(), runtime.incident_store.list_active())
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
    async def add_topology_edge(payload: dict[str, Any]) -> dict[str, Any]:
        source = payload.get("source")
        target = payload.get("target")
        if not source or not target:
            raise HTTPException(status_code=400, detail="'source' and 'target' are required")
        relation_type = payload.get("relation_type", "depends_on")
        runtime.add_topology_relation(source, target, relation_type)
        return {"added": True, "edge": {"source": source, "target": target, "relation_type": relation_type}}

    return app


def _incident_to_dict(item: Any) -> dict[str, Any]:
    return {
        "incident_id": item.incident_id,
        "state": item.state.value,
        "created_at": to_iso(item.created_at),
        "updated_at": to_iso(item.updated_at),
        "severity": int(item.severity),
        "primary_service": item.primary_service,
        "affected_services": list(item.affected_services),
        "time_range_start": to_iso(item.time_range_start),
        "time_range_end": to_iso(item.time_range_end),
        "event_count": item.event_count,
    }


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


def _service_health(services: list[dict[str, Any]], incidents: list[Any]) -> list[dict[str, Any]]:
    incident_services: dict[str, list[dict[str, Any]]] = {}
    for incident in incidents:
        payload = _incident_to_dict(incident)
        for service in set(incident.affected_services) | ({incident.primary_service} if incident.primary_service else set()):
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


def _is_local_base_url(base_url: str) -> bool:
    lowered = base_url.lower()
    return "127.0.0.1" in lowered or "localhost" in lowered
