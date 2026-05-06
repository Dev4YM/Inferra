from __future__ import annotations

import hashlib
from collections import defaultdict
from collections.abc import Callable
from dataclasses import fields
from datetime import datetime, timedelta
from typing import Any

from analysis.anomaly import AnomalyScorer, reconcile_baseline_from_events
from analysis.correlation import CorrelationEngine, stable_cluster_id
from analysis.models import EventCluster
from config.models import AnomalyDetectionConfig, InferraConfig
from core.enums import CauseType, IncidentState, Severity
from core.logging import get_logger
from core.models import Incident, ScoredHypothesis, ScoreBreakdown
from core.time import to_iso, utc_now
from events.models import EventFilter, NormalizedEvent
from reasoning.engine import HypothesisEngine
from runtime.service_graph import ServiceGraph
from storage import BaselineStore, CalibrationStore, EventStore, IncidentStore, WeightStore

_log = get_logger(__name__)


def _time_gap_seconds(
    range_a: tuple[Any, Any],
    range_b: tuple[Any, Any],
) -> float:
    if range_a[1] < range_b[0]:
        return (range_b[0] - range_a[1]).total_seconds()
    if range_b[1] < range_a[0]:
        return (range_a[0] - range_b[1]).total_seconds()
    return 0.0


def should_merge_cluster(
    cluster: EventCluster,
    incident: Incident,
    merge_threshold_seconds: int,
) -> bool:
    overlap = cluster.affected_services & incident.affected_services
    if not overlap:
        return False
    gap = _time_gap_seconds(cluster.time_range, incident.time_range)
    if gap > merge_threshold_seconds:
        return False
    if set(cluster.events) & set(incident.events):
        return True
    small = min(len(cluster.affected_services), len(incident.affected_services))
    overlap_ratio = len(overlap) / max(small, 1)
    return bool(overlap_ratio >= 0.5 and gap < 60.0)


def _incident_id_from_services(services: list[str], anchor: Any) -> str:
    bucket = int(anchor.timestamp() // 300)
    joined = ",".join(services)
    digest = hashlib.sha256(f"{joined}|{bucket}".encode("utf-8")).hexdigest()[:16]
    return f"inc-{digest}"


def primary_service_for_events(events: list[NormalizedEvent], service_graph: ServiceGraph) -> str | None:
    if not events:
        return None
    by_svc: dict[str, list[NormalizedEvent]] = defaultdict(list)
    for event in events:
        by_svc[event.service_id].append(event)
    best_sid: str | None = None
    best_key: tuple[float, float, float, str] | None = None
    for sid in sorted(by_svc.keys()):
        evs = by_svc[sid]
        max_sev = max(int(e.severity) for e in evs)
        deg = len(service_graph.get_dependencies(sid)) + len(service_graph.get_dependents(sid))
        centrality = 1.0 / (1.0 + float(deg))
        weighted = float(max_sev) * centrality
        earliest = min(e.timestamp for e in evs).timestamp()
        key = (weighted, float(max_sev), -earliest, sid)
        if best_key is None or key > best_key:
            best_key = key
            best_sid = sid
    return best_sid


def _subcluster(
    parent: EventCluster,
    event_ids: list[str],
    events_by_id: dict[str, NormalizedEvent],
) -> EventCluster | None:
    event_set = set(event_ids)
    if len(event_set) < 2:
        return None
    edges = [
        edge
        for edge in parent.correlation_edges
        if edge.source_event_id in event_set and edge.target_event_id in event_set
    ]
    evs = sorted((events_by_id[e] for e in event_ids if e in events_by_id), key=lambda e: e.timestamp)
    if len(evs) < 2:
        return None
    scorer = AnomalyScorer()
    return EventCluster(
        cluster_id=stable_cluster_id(evs),
        events=[e.event_id for e in evs],
        time_range=(evs[0].timestamp, evs[-1].timestamp),
        affected_services={e.service_id for e in evs},
        primary_severity=max(e.severity for e in evs),
        trigger_event_id=evs[0].event_id,
        correlation_edges=edges,
        anomaly_scores=scorer.service_scores(evs),
    )


def expand_clusters_for_limits(
    cluster: EventCluster,
    events_by_id: dict[str, NormalizedEvent],
    max_events_per_incident: int,
    enable_auto_split: bool,
) -> list[EventCluster]:
    if not enable_auto_split:
        return [cluster]
    large_events = len(cluster.events) > max_events_per_incident
    large_services = len(cluster.affected_services) >= 8 and len(cluster.events) >= 4
    if not large_events and not large_services:
        return [cluster]
    objs = sorted(
        (events_by_id[e] for e in cluster.events if e in events_by_id),
        key=lambda e: e.timestamp,
    )
    if len(objs) < 3:
        return [cluster]
    gaps = [
        (idx, (objs[idx + 1].timestamp - objs[idx].timestamp).total_seconds()) for idx in range(len(objs) - 1)
    ]
    max_idx, max_gap = max(gaps, key=lambda item: item[1])
    median = sorted(g[1] for g in gaps)[len(gaps) // 2]
    if max_gap < max(30.0, 2.0 * max(median, 1e-6)):
        return [cluster]
    left_ids = [e.event_id for e in objs[: max_idx + 1]]
    right_ids = [e.event_id for e in objs[max_idx + 1 :]]
    left_c = _subcluster(cluster, left_ids, events_by_id)
    right_c = _subcluster(cluster, right_ids, events_by_id)
    if left_c is None or right_c is None:
        return [cluster]
    return [left_c, right_c]


class IncidentLifecycleManager:
    def __init__(
        self,
        event_store: EventStore,
        incident_store: IncidentStore,
        service_graph: ServiceGraph | None = None,
        config: InferraConfig | None = None,
        baseline_store: BaselineStore | None = None,
        anomaly_detection: AnomalyDetectionConfig | None = None,
        weight_store: WeightStore | None = None,
        calibration_store: CalibrationStore | None = None,
        live_notify: Callable[[str, dict[str, Any]], None] | None = None,
    ) -> None:
        self.event_store = event_store
        self.incident_store = incident_store
        self._config = config or InferraConfig()
        self.service_graph = service_graph or ServiceGraph()
        self._live_notify = live_notify
        self.correlation = CorrelationEngine(self.service_graph, self._config.correlation)
        self.hypotheses = HypothesisEngine(
            self.service_graph,
            self._config,
            weight_store=weight_store,
            calibration_store=calibration_store,
        )
        self._baseline_store = baseline_store
        self._anomaly_detection = anomaly_detection

    def _emit_live(self, kind: str, payload: dict[str, Any]) -> None:
        notify = self._live_notify
        if notify is None:
            return
        notify(kind, payload)

    def analyze_recent(self, window_seconds: int | None = None) -> int:
        window_seconds = window_seconds or int(self._config.correlation.analysis_window_seconds)
        end = utc_now()
        start = end - timedelta(seconds=window_seconds)
        filters = EventFilter(severities={Severity.WARN, Severity.ERROR, Severity.CRITICAL})
        events = list(self.event_store.query_time_range(start, end, filters=filters, limit=500))
        events_by_id = {event.event_id: event for event in events}
        raw_clusters = self.correlation.build_clusters(events)
        clusters: list[EventCluster] = []
        for c in sorted(raw_clusters, key=lambda item: item.cluster_id):
            clusters.extend(
                expand_clusters_for_limits(
                    c,
                    events_by_id,
                    self._config.incident_lifecycle.limits.max_events_per_incident,
                    self._config.incident_lifecycle.enable_auto_split,
                )
            )
        if not clusters:
            self._reconcile_anomaly_baselines(start, end)
            self._apply_staleness(end)
            self._promote_explained_from_cache()
            return 0
        active = self.incident_store.list_incidents(
            state=[IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED],
            limit=self._config.incident_lifecycle.limits.max_active_incidents,
        )
        active_sorted = sorted(active, key=lambda item: item.incident_id)
        updated = 0
        for cluster in sorted(clusters, key=lambda item: item.cluster_id):
            cluster_events = [events_by_id[event_id] for event_id in cluster.events if event_id in events_by_id]
            if len(cluster_events) < self._config.correlation.cluster_min_events:
                continue
            target: Incident | None = None
            for incident in active_sorted:
                if should_merge_cluster(
                    cluster,
                    incident,
                    self._config.incident_lifecycle.merge_time_threshold_seconds,
                ):
                    target = incident
                    break
            if target is not None:
                self._merge_cluster_into_incident(target, cluster, cluster_events, events_by_id)
                updated += 1
            else:
                if self._ingest_new_cluster(cluster, cluster_events, events_by_id):
                    updated += 1
        self._reconcile_anomaly_baselines(start, end)
        self._apply_staleness(end)
        self._promote_explained_from_cache()
        return updated

    def _incident_live_payload(self, incident: Incident) -> dict[str, Any]:
        return {
            "incident_id": incident.incident_id,
            "state": incident.state.value,
            "primary_service": incident.primary_service or "",
            "event_count": len(incident.events),
        }

    def _merge_cluster_into_incident(
        self,
        incident: Incident,
        cluster: EventCluster,
        cluster_events: list[NormalizedEvent],
        events_by_id: dict[str, NormalizedEvent],
    ) -> None:
        current = self.incident_store.get_incident(incident.incident_id) or incident
        merged_events = list(dict.fromkeys([*current.events, *cluster.events]))
        merged_clusters = list(dict.fromkeys([*current.clusters, cluster.cluster_id]))
        all_events = [events_by_id[e] for e in merged_events if e in events_by_id]
        affected = set(current.affected_services) | set(cluster.affected_services)
        time_range = (
            min(current.time_range[0], cluster.time_range[0]),
            max(current.time_range[1], cluster.time_range[1]),
        )
        severity = (
            current.severity if int(current.severity) >= int(cluster.primary_severity) else cluster.primary_severity
        )
        primary = primary_service_for_events(all_events, self.service_graph) if all_events else current.primary_service
        now = utc_now()
        state = current.state
        if current.state == IncidentState.EXPLAINED and set(cluster.events) - set(current.events):
            self.incident_store.transition_state(
                current.incident_id,
                IncidentState.INVESTIGATING,
                "new correlated events merged",
            )
            refreshed = self.incident_store.get_incident(current.incident_id)
            state = refreshed.state if refreshed is not None else IncidentState.INVESTIGATING
        hypotheses_payload = self.hypotheses.generate(
            current.incident_id,
            all_events,
            cluster=cluster,
            incident_event_ids=merged_events,
        )
        graph = self.hypotheses.last_inference_graph
        rebuilt = Incident(
            incident_id=current.incident_id,
            state=state,
            created_at=current.created_at,
            updated_at=now,
            clusters=merged_clusters,
            events=merged_events,
            affected_services=affected,
            primary_service=primary,
            time_range=time_range,
            severity=severity,
            runtime_context=current.runtime_context,
            inference_graph=graph if graph is not None else current.inference_graph,
        )
        self.incident_store.update_incident(rebuilt)
        self.incident_store.save_cluster(current.incident_id, cluster)
        hypotheses = [self._to_hypothesis(item) for item in hypotheses_payload]
        self.incident_store.add_hypotheses(current.incident_id, hypotheses)
        self._emit_live("incident_updated", self._incident_live_payload(rebuilt))

    def _ingest_new_cluster(
        self,
        cluster: EventCluster,
        cluster_events: list[NormalizedEvent],
        events_by_id: dict[str, NormalizedEvent],
    ) -> bool:
        services_sorted = sorted(cluster.affected_services)
        incident_id = _incident_id_from_services(services_sorted, cluster.time_range[0])
        now = utc_now()
        existing = self.incident_store.get_incident(incident_id)
        primary = primary_service_for_events(cluster_events, self.service_graph)
        state = IncidentState.OPEN if existing is None else existing.state
        if existing is not None and existing.state == IncidentState.EXPLAINED and set(cluster.events) - set(
            existing.events
        ):
            self.incident_store.transition_state(
                incident_id,
                IncidentState.INVESTIGATING,
                "expanded cluster after fingerprint match",
            )
            state = IncidentState.INVESTIGATING
        event_ids = list(dict.fromkeys([*(existing.events if existing is not None else []), *cluster.events]))
        all_events = [events_by_id[e] for e in event_ids if e in events_by_id]
        hypotheses_payload = self.hypotheses.generate(
            incident_id,
            all_events,
            cluster=cluster,
            incident_event_ids=event_ids,
        )
        graph = self.hypotheses.last_inference_graph
        incident = Incident(
            incident_id=incident_id,
            state=state,
            created_at=existing.created_at if existing is not None else now,
            updated_at=utc_now(),
            clusters=list(dict.fromkeys([*(existing.clusters if existing is not None else []), cluster.cluster_id])),
            events=event_ids,
            affected_services=set(cluster.affected_services),
            primary_service=primary,
            time_range=cluster.time_range,
            severity=cluster.primary_severity,
            inference_graph=graph if graph is not None else (existing.inference_graph if existing is not None else None),
        )
        if existing is None:
            self.incident_store.create_incident(incident)
            self.incident_store.record_state_log(incident_id, "none", IncidentState.OPEN.value, "incident opened")
            self._emit_live("incident_created", self._incident_live_payload(incident))
        else:
            self.incident_store.update_incident(incident)
            self._emit_live("incident_updated", self._incident_live_payload(incident))
        self.incident_store.save_cluster(incident_id, cluster)
        hypotheses = [self._to_hypothesis(item) for item in hypotheses_payload]
        self.incident_store.add_hypotheses(incident_id, hypotheses)
        refreshed = self.incident_store.get_incident(incident_id)
        if refreshed is not None and refreshed.state == IncidentState.OPEN:
            self.incident_store.transition_state(
                incident_id,
                IncidentState.INVESTIGATING,
                "hypotheses recorded",
            )
            after = self.incident_store.get_incident(incident_id)
            if after is not None:
                self._emit_live("incident_updated", self._incident_live_payload(after))
        return True

    def _apply_staleness(self, now: datetime) -> None:
        timeout = float(self._config.incident_lifecycle.stale_timeout_seconds)
        active = self.incident_store.list_incidents(
            state=[IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED],
            limit=self._config.incident_lifecycle.limits.max_active_incidents,
        )
        for incident in active:
            last_ts = self._last_event_timestamp(incident)
            if last_ts is None:
                continue
            if (now - last_ts).total_seconds() < timeout:
                continue
            if incident.state == IncidentState.RESOLVED:
                continue
            self.incident_store.transition_state(
                incident.incident_id,
                IncidentState.RESOLVED,
                "stale_timeout_seconds",
            )
            self._emit_live(
                "incident_resolved",
                {"incident_id": incident.incident_id, "reason": "stale_timeout_seconds"},
            )

    def _last_event_timestamp(self, incident: Incident) -> datetime | None:
        best = None
        for event_id in incident.events:
            event = self.event_store.get_event(event_id)
            if event is None:
                continue
            if best is None or event.timestamp > best:
                best = event.timestamp
        return best

    def _promote_explained_from_cache(self) -> None:
        active = self.incident_store.list_incidents(
            state=[IncidentState.INVESTIGATING],
            limit=self._config.incident_lifecycle.limits.max_active_incidents,
        )
        for incident in active:
            if self.incident_store.get_latest_explanation(incident.incident_id) is None:
                continue
            self.incident_store.transition_state(
                incident.incident_id,
                IncidentState.EXPLAINED,
                "explanation present",
            )
            refreshed = self.incident_store.get_incident(incident.incident_id)
            if refreshed is not None:
                self._emit_live("incident_updated", self._incident_live_payload(refreshed))

    def _reconcile_anomaly_baselines(self, start: Any, end: Any) -> None:
        if self._baseline_store is None or self._anomaly_detection is None or not self._anomaly_detection.enabled:
            return
        stored = list(self.event_store.query_time_range(start, end, filters=None, limit=8000))
        by_service: dict[str, list[NormalizedEvent]] = {}
        for event in stored:
            by_service.setdefault(event.service_id, []).append(event)
        now = utc_now()
        for service_id, service_events in sorted(by_service.items()):
            reconcile_baseline_from_events(
                self._baseline_store,
                service_id,
                service_events,
                config=self._anomaly_detection,
                now=now,
            )
        self._emit_live(
            "baseline_status",
            {"services_touched": sorted(by_service.keys()), "timestamp": to_iso(now)},
        )

    def _to_hypothesis(self, payload: dict[str, Any]) -> ScoredHypothesis:
        score_fields = {field.name for field in fields(ScoreBreakdown)}
        score_data = {
            key: float(value)
            for key, value in dict(payload.get("score_breakdown") or {}).items()
            if key in score_fields
        }
        return ScoredHypothesis(
            hypothesis_id=str(payload["hypothesis_id"]),
            rank=int(payload.get("rank") or 0),
            cause_type=CauseType(str(payload["cause_type"])),
            description=str(payload["description"]),
            total_score=float(payload.get("total_score") or 0.0),
            score_breakdown=ScoreBreakdown(**score_data),
            supporting_events=list(payload.get("supporting_events") or []),
            contradicting_events=list(payload.get("contradicting_events") or []),
            affected_services=sorted(payload.get("affected_services") or []),
            suggested_checks=list(payload.get("suggested_checks") or []),
            confidence_label=str(payload.get("confidence_label") or "low"),
            is_valid=bool(payload.get("is_valid", True)),
            invalidation_reasons=list(payload.get("invalidation_reasons") or []),
        )
