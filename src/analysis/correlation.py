from __future__ import annotations

import hashlib
import math
from datetime import datetime
from itertools import combinations

import networkx as nx

from analysis.anomaly import AnomalyScorer
from analysis.models import CorrelationEdge, EventCluster
from core.enums import Severity
from events.models import NormalizedEvent
from runtime.service_graph import ServiceGraph


FAILURE_TAGS = {
    "connection_refused",
    "timeout",
    "oom",
    "disk_full",
    "dns_failure",
    "certificate_error",
    "crash",
    "resource_pressure",
    "restart",
}


class CorrelationEngine:
    def __init__(
        self,
        service_graph: ServiceGraph | None = None,
        temporal_window_seconds: int = 30,
        cluster_min_edge_weight: float = 0.15,
        cluster_min_events: int = 2,
    ) -> None:
        self.service_graph = service_graph or ServiceGraph()
        self.temporal_window_seconds = temporal_window_seconds
        self.cluster_min_edge_weight = cluster_min_edge_weight
        self.cluster_min_events = cluster_min_events
        self.anomaly_scorer = AnomalyScorer()

    def build_clusters(self, events: list[NormalizedEvent]) -> list[EventCluster]:
        candidates = sorted(
            [event for event in events if event.severity >= Severity.WARN and event.quality.overall >= 0.3],
            key=lambda event: event.timestamp,
        )
        if len(candidates) < self.cluster_min_events:
            return []

        edges = self._build_edges(candidates)
        graph = nx.Graph()
        for event in candidates:
            graph.add_node(event.event_id)
        for edge in edges:
            if edge.weight >= self.cluster_min_edge_weight:
                graph.add_edge(edge.source_event_id, edge.target_event_id, edge=edge, weight=edge.weight)

        by_id = {event.event_id: event for event in candidates}
        clusters: list[EventCluster] = []
        for component in nx.connected_components(graph):
            if len(component) < self.cluster_min_events:
                continue
            component_events = sorted((by_id[event_id] for event_id in component), key=lambda event: event.timestamp)
            component_edges = [
                graph.edges[a, b]["edge"]
                for a, b in graph.subgraph(component).edges()
                if "edge" in graph.edges[a, b]
            ]
            if not component_edges:
                continue
            primary_severity = max(event.severity for event in component_events)
            if primary_severity < Severity.WARN:
                continue
            cluster_id = self._cluster_id(component_events)
            clusters.append(
                EventCluster(
                    cluster_id=cluster_id,
                    events=[event.event_id for event in component_events],
                    time_range=(component_events[0].timestamp, component_events[-1].timestamp),
                    affected_services={event.service_id for event in component_events},
                    primary_severity=primary_severity,
                    trigger_event_id=component_events[0].event_id,
                    correlation_edges=component_edges,
                    anomaly_scores=self.anomaly_scorer.service_scores(component_events),
                )
            )
        return clusters

    def _build_edges(self, events: list[NormalizedEvent]) -> list[CorrelationEdge]:
        edges: list[CorrelationEdge] = []
        for left, right in combinations(events, 2):
            delta = abs((right.timestamp - left.timestamp).total_seconds())
            if delta > self.temporal_window_seconds:
                continue
            related = right.service_id in self.service_graph.related_services(left.service_id)
            shared_tags = (left.tags & right.tags) & FAILURE_TAGS
            if not related and not shared_tags:
                continue
            weight = self._temporal_decay(delta)
            edge_type = "temporal" if related else "co_occurrence"
            evidence = f"events within {delta:.1f}s"
            if shared_tags:
                weight = max(weight, 0.55)
                edge_type = "co_occurrence" if not related else "temporal_tag"
                evidence += f"; shared tags: {', '.join(sorted(shared_tags))}"
            if related and left.service_id != right.service_id:
                edge_type = "service_dependency"
                evidence += "; services are related by topology"
            weight *= min(left.quality.overall, right.quality.overall)
            if weight >= 0.1:
                edges.append(
                    CorrelationEdge(
                        source_event_id=left.event_id,
                        target_event_id=right.event_id,
                        edge_type=edge_type,
                        weight=round(min(1.0, weight), 4),
                        evidence=evidence,
                    )
                )
        return edges

    def _temporal_decay(self, delta_seconds: float) -> float:
        half_life = 10.0
        return math.exp(-0.693 * delta_seconds / half_life)

    def _cluster_id(self, events: list[NormalizedEvent]) -> str:
        bucket = int(events[0].timestamp.timestamp() // 300)
        services = ",".join(sorted({event.service_id for event in events}))
        digest = hashlib.sha256(f"{services}|{bucket}|{len(events)}".encode("utf-8")).hexdigest()[:16]
        return f"clu-{digest}"
