"""Shared serialization and helpers used by multiple web routers.

These helpers were originally inlined in `web.api`. They are now centralized
so that each domain router can import them without duplicating presentation
logic. Nothing in here mutates state or talks to the network.
"""

from __future__ import annotations

from typing import Any

from ai.explainer import AiPromptTrace
from app import InferraRuntime
from core.enums import IncidentState, Severity
from core.ids import new_id
from core.models import (
    ExplanationResult,
    Incident,
    IncidentAiTrace,
    ScoredHypothesis,
)
from core.time import to_iso
from events.models import NormalizedEvent


def active_incidents(runtime: InferraRuntime) -> list[Incident]:
    return runtime.incident_store.list_incidents(
        state=[IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED],
        limit=200,
    )


def incident_to_dict(item: Incident) -> dict[str, Any]:
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


def hypothesis_to_dict(item: ScoredHypothesis) -> dict[str, Any]:
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


def explanation_to_dict(item: ExplanationResult) -> dict[str, Any]:
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


def bounded_limit(limit: int, maximum: int) -> int:
    return max(1, min(maximum, int(limit)))


def severity_counts(events: list[NormalizedEvent]) -> dict[str, int]:
    counts = {item.name.lower(): 0 for item in Severity}
    for event in events:
        counts[event.severity.name.lower()] += 1
    return counts


def event_rate(events: list[NormalizedEvent]) -> list[dict[str, Any]]:
    buckets: dict[str, dict[str, Any]] = {}
    for event in events:
        label = to_iso(event.timestamp.replace(second=0, microsecond=0))
        if label not in buckets:
            buckets[label] = {"timestamp": label, "total": 0, "warn": 0, "error": 0, "critical": 0}
        buckets[label]["total"] += 1
        if event.severity >= Severity.WARN:
            buckets[label][event.severity.name.lower()] += 1
    return [buckets[key] for key in sorted(buckets)][-60:]


def service_health(services: list[dict[str, Any]], incidents: list[Incident]) -> list[dict[str, Any]]:
    incident_services: dict[str, list[dict[str, Any]]] = {}
    for incident in incidents:
        payload = incident_to_dict(incident)
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


def ai_trace_event(event: NormalizedEvent, supporting: bool, contradicting: bool) -> dict[str, Any]:
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


def persist_ai_prompt_trace(runtime: InferraRuntime, incident_id: str, trace: AiPromptTrace) -> None:
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
