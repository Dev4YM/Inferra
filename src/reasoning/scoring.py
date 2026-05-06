from __future__ import annotations

import math
from datetime import datetime
from typing import Any

from core.time import utc_now

from analysis.models import EventCluster
from config.models import InferraConfig
from core.models import Incident, ScoreBreakdown
from events.models import NormalizedEvent
from runtime.service_graph import ServiceGraph


def compute_score_breakdown(
    hypothesis: dict[str, Any],
    events_by_id: dict[str, NormalizedEvent],
    *,
    cluster: EventCluster | None,
    incident: Incident | None,
    incident_event_ids: list[str] | None,
    service_graph: ServiceGraph,
    anomaly_by_service: dict[str, float],
    anomaly_event_scores: dict[str, float],
) -> ScoreBreakdown:
    supporting_ids = list(hypothesis.get("supporting_events") or [])
    supporting = [events_by_id[event_id] for event_id in supporting_ids if event_id in events_by_id]
    root_id = hypothesis.get("root_cause_event_id")
    temporal = _temporal_alignment(root_id, supporting, events_by_id)
    correlation = _correlation_strength(hypothesis, cluster)
    frequency = _frequency_weight(supporting_ids, events_by_id)
    dep = _dependency_proximity_score(hypothesis.get("affected_services") or [], service_graph)
    coverage = _evidence_coverage(supporting_ids, incident, incident_event_ids)
    anomaly = _anomaly_severity(hypothesis.get("affected_services") or [], anomaly_by_service, supporting_ids, anomaly_event_scores, events_by_id)
    return ScoreBreakdown(
        temporal_alignment=round(temporal, 4),
        correlation_strength=round(correlation, 4),
        frequency_weight=round(frequency, 4),
        dependency_proximity=round(dep, 4),
        evidence_coverage=round(coverage, 4),
        anomaly_severity=round(anomaly, 4),
    )


def weighted_total(breakdown: ScoreBreakdown, weights: dict[str, float]) -> float:
    parts = (
        weights["temporal_alignment"] * breakdown.temporal_alignment,
        weights["correlation_strength"] * breakdown.correlation_strength,
        weights["frequency_weight"] * breakdown.frequency_weight,
        weights["dependency_proximity"] * breakdown.dependency_proximity,
        weights["evidence_coverage"] * breakdown.evidence_coverage,
        weights["anomaly_severity"] * breakdown.anomaly_severity,
    )
    return round(max(0.0, min(1.0, sum(parts))), 4)


def merge_config_weights(state_weights: dict[str, float], config: InferraConfig) -> dict[str, float]:
    keys = (
        "temporal_alignment",
        "correlation_strength",
        "frequency_weight",
        "dependency_proximity",
        "evidence_coverage",
        "anomaly_severity",
    )
    return {key: float(state_weights.get(key, getattr(config.scoring, key))) for key in keys}


def rank_hypotheses(
    hypotheses: list[dict[str, Any]],
    tiebreak_order: list[str],
    events_by_id: dict[str, NormalizedEvent],
) -> list[dict[str, Any]]:
    def sort_key(item: dict[str, Any]) -> tuple:
        breakdown = item.get("score_breakdown") or {}
        tie_parts: list[Any] = []
        for rule in tiebreak_order:
            if rule == "evidence_coverage":
                tie_parts.append(-float(breakdown.get("evidence_coverage", 0.0)))
            elif rule == "contradicting_events_asc":
                tie_parts.append(len(item.get("contradicting_events") or []))
            elif rule == "root_cause_timestamp_asc":
                rid = item.get("root_cause_event_id")
                ev = events_by_id.get(rid) if rid else None
                if ev is not None:
                    ts = ev.timestamp
                else:
                    sample = next(iter(events_by_id.values()), None)
                    tz = sample.timestamp.tzinfo if sample is not None else utc_now().tzinfo
                    ts = datetime(2099, 1, 1, tzinfo=tz)
                tie_parts.append(ts)
            elif rule == "temporal_alignment":
                tie_parts.append(-float(breakdown.get("temporal_alignment", 0.0)))
            elif rule == "correlation_strength":
                tie_parts.append(-float(breakdown.get("correlation_strength", 0.0)))
            elif rule == "frequency_weight":
                tie_parts.append(-float(breakdown.get("frequency_weight", 0.0)))
            elif rule == "dependency_proximity":
                tie_parts.append(-float(breakdown.get("dependency_proximity", 0.0)))
            elif rule == "anomaly_severity":
                tie_parts.append(-float(breakdown.get("anomaly_severity", 0.0)))
            elif rule == "hypothesis_id_asc":
                tie_parts.append(str(item.get("hypothesis_id", "")))
        tie_parts.append(str(item.get("hypothesis_id", "")))
        return (-float(item.get("total_score") or 0.0), tuple(tie_parts))

    return sorted(hypotheses, key=sort_key)


def _temporal_alignment(
    root_id: str | None,
    supporting: list[NormalizedEvent],
    events_by_id: dict[str, NormalizedEvent],
) -> float:
    if not root_id or root_id not in events_by_id or not supporting:
        return 0.5
    root = events_by_id[root_id]
    correct = sum(1 for event in supporting if event.timestamp >= root.timestamp or event.event_id == root_id)
    order_ratio = correct / max(len(supporting), 1)
    timestamps = sorted(event.timestamp for event in supporting)
    span = (timestamps[-1] - timestamps[0]).total_seconds()
    if span <= 0:
        tightness = 1.0
    else:
        tightness = math.exp(-0.693 * span / 60.0)
    return 0.6 * order_ratio + 0.4 * tightness


def _correlation_strength(hypothesis: dict[str, Any], cluster: EventCluster | None) -> float:
    if cluster is None:
        return 0.0
    hyp_events = set(hypothesis.get("supporting_events") or [])
    relevant = [
        edge
        for edge in cluster.correlation_edges
        if edge.source_event_id in hyp_events and edge.target_event_id in hyp_events
    ]
    if not relevant:
        return 0.0
    avg_weight = sum(edge.weight for edge in relevant) / len(relevant)
    edge_types = {edge.edge_type for edge in relevant}
    diversity_bonus = min(0.2, 0.1 * max(0, len(edge_types) - 1))
    return min(1.0, avg_weight + diversity_bonus)


def _frequency_weight(supporting_ids: list[str], events_by_id: dict[str, NormalizedEvent]) -> float:
    total = 0.0
    for event_id in supporting_ids:
        event = events_by_id.get(event_id)
        if event is None:
            continue
        raw = event.structured_data.get("_dedup_count", 1)
        try:
            total += float(raw)
        except (TypeError, ValueError):
            total += 1.0
    if total <= 1.0:
        return 0.0
    return min(1.0, math.log(total) / math.log(200.0))


def _dependency_proximity_score(services: list[str], service_graph: ServiceGraph) -> float:
    if len(services) < 2:
        return 0.5
    distances: list[int] = []
    for index, source in enumerate(services):
        for target in services[index + 1 :]:
            distance = service_graph.shortest_path_length(source, target, max_depth=4)
            if distance is not None:
                distances.append(distance)
    if not distances:
        return 0.25
    nearest = min(distances)
    return max(0.35, 1.0 - (nearest - 1) * 0.2)


def _evidence_coverage(
    supporting_ids: list[str],
    incident: Incident | None,
    incident_event_ids: list[str] | None,
) -> float:
    ids = set(supporting_ids)
    total_ids: set[str] = set()
    if incident is not None:
        total_ids = set(incident.events)
    elif incident_event_ids is not None:
        total_ids = set(incident_event_ids)
    if not total_ids:
        return 1.0 if ids else 0.0
    coverage = len(ids & total_ids) / len(total_ids)
    if coverage < 0.05:
        return coverage * 2.0
    return min(1.0, coverage)


def _anomaly_severity(
    affected_services: list[str],
    anomaly_by_service: dict[str, float],
    supporting_ids: list[str],
    anomaly_event_scores: dict[str, float],
    events_by_id: dict[str, NormalizedEvent],
) -> float:
    service_scores = [float(anomaly_by_service.get(sid, 0.0)) for sid in affected_services]
    svc_max = max(service_scores) if service_scores else 0.0
    ev_scores = [float(anomaly_event_scores.get(event_id, 0.0)) for event_id in supporting_ids if event_id in events_by_id]
    ev_max = max(ev_scores) if ev_scores else 0.0
    return max(svc_max, ev_max)
