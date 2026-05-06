from __future__ import annotations

import hashlib
import math
from collections import defaultdict
from itertools import combinations

import networkx as nx

from analysis.anomaly import AnomalyScorer
from analysis.models import CorrelationEdge, EventCluster
from config.models import CorrelationConfig
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


def stable_cluster_id(events: list[NormalizedEvent]) -> str:
    t0 = min(event.timestamp for event in events)
    bucket = int(t0.timestamp() // 300)
    services = ",".join(sorted({event.service_id for event in events}))
    digest = hashlib.sha256(f"{services}|{bucket}|{len(events)}".encode("utf-8")).hexdigest()[:16]
    return f"clu-{digest}"


class CorrelationEngine:
    def __init__(
        self,
        service_graph: ServiceGraph | None = None,
        config: CorrelationConfig | None = None,
    ) -> None:
        self.service_graph = service_graph or ServiceGraph()
        self._cfg = config or CorrelationConfig()
        self.anomaly_scorer = AnomalyScorer()

    def build_clusters(self, events: list[NormalizedEvent]) -> list[EventCluster]:
        candidates = sorted(
            [event for event in events if event.severity >= Severity.WARN and event.quality.overall >= 0.3],
            key=lambda event: event.timestamp,
        )
        if len(candidates) < self._cfg.cluster_min_events:
            return []

        raw_edges = (
            self._temporal_and_topology_edges(candidates)
            + self._shared_service_edges(candidates)
            + self._cooccurrence_edges(candidates)
            + self._cascade_edges(candidates)
        )
        merged = self._merge_edges(raw_edges)
        graph = nx.Graph()
        for event in candidates:
            graph.add_node(event.event_id)
        for edge in merged:
            if edge.weight < self._cfg.cluster_min_edge_weight:
                continue
            graph.add_edge(edge.source_event_id, edge.target_event_id, edge=edge, weight=edge.weight)

        by_id = {event.event_id: event for event in candidates}
        clusters: list[EventCluster] = []
        for component in nx.connected_components(graph):
            if len(component) < self._cfg.cluster_min_events:
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
            cluster_id = stable_cluster_id(component_events)
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

    def _max_time_delta(self) -> float:
        return float(self._cfg.temporal_lookback_seconds + self._cfg.temporal_lookahead_seconds)

    def _temporal_decay(self, delta_seconds: float) -> float:
        hl = float(self._cfg.temporal_half_life_seconds)
        if hl <= 0:
            return 1.0
        return math.exp(-0.693 * abs(delta_seconds) / hl)

    def _cascade_directional_weight(self, latency_seconds: float) -> float:
        window = float(self._cfg.cascade_window_seconds)
        if latency_seconds <= 0 or latency_seconds > window:
            return 0.0
        return 1.0 - (latency_seconds / window)

    def _hop_weight(self, hops: int | None) -> float:
        if hops is None or hops > self._cfg.max_hop_distance:
            return 0.0
        if self._cfg.dependency_weight_decay == "none":
            return 1.0
        if self._cfg.dependency_weight_decay == "linear":
            return max(0.0, 1.0 - hops / (self._cfg.max_hop_distance + 1))
        return 1.0 / (1.0 + float(hops))

    def _temporal_and_topology_edges(self, events: list[NormalizedEvent]) -> list[CorrelationEdge]:
        edges: list[CorrelationEdge] = []
        max_delta = self._max_time_delta()
        for left, right in combinations(events, 2):
            if left.service_id == right.service_id:
                continue
            delta = (right.timestamp - left.timestamp).total_seconds()
            if abs(delta) > max_delta:
                continue
            a, b = (left, right) if left.timestamp <= right.timestamp else (right, left)
            delta_ab = (b.timestamp - a.timestamp).total_seconds()
            decay = self._temporal_decay(delta_ab)
            shared_tags = (a.tags & b.tags) & FAILURE_TAGS
            related = b.service_id in self.service_graph.related_services(a.service_id)
            if not related and not shared_tags:
                continue
            q = min(a.quality.overall, b.quality.overall)
            if not related and shared_tags:
                weight = 0.5 * decay * q
                if weight < 0.1:
                    continue
                edges.append(
                    CorrelationEdge(
                        source_event_id=a.event_id,
                        target_event_id=b.event_id,
                        edge_type="co_occurrence",
                        weight=round(min(1.0, weight), 4),
                        evidence=f"shared failure tags within {abs(delta_ab):.1f}s",
                        reason_codes=("temporal_tag_cooccurrence",),
                    )
                )
                continue
            hops: int | None
            if b.service_id in self.service_graph.get_colocated(a.service_id):
                hops = 1
            else:
                hops = self.service_graph.shortest_path_length(
                    a.service_id, b.service_id, self._cfg.max_hop_distance
                )
            hop_mul = self._hop_weight(hops) if hops is not None else 0.0
            if hop_mul <= 0 and related:
                hop_mul = 1.0 / (1.0 + float(self._cfg.max_hop_distance))
            codes: tuple[str, ...] = ("topology_related_window",)
            if hops == 1 and b.service_id not in self.service_graph.get_colocated(a.service_id):
                codes = ("dependency_propagation",)
            elif hops is not None and hops > 1:
                codes = ("dependency_propagation", f"hops={hops}")
            if b.service_id in self.service_graph.get_colocated(a.service_id):
                codes = (*codes, "shared_fate")
            et = "dependency_propagation"
            evidence = f"related services within {abs(delta_ab):.1f}s"
            if shared_tags:
                et = "temporal_tag"
                evidence += f"; shared tags: {', '.join(sorted(shared_tags))}"
            weight = decay * hop_mul * q
            if shared_tags:
                weight = max(weight, 0.55 * q)
            if weight < 0.1:
                continue
            edges.append(
                CorrelationEdge(
                    source_event_id=a.event_id,
                    target_event_id=b.event_id,
                    edge_type=et,
                    weight=round(min(1.0, weight), 4),
                    evidence=evidence,
                    reason_codes=codes,
                )
            )
        return edges

    def _shared_service_edges(self, events: list[NormalizedEvent]) -> list[CorrelationEdge]:
        edges: list[CorrelationEdge] = []
        max_delta = self._max_time_delta()
        by_service: dict[str, list[NormalizedEvent]] = defaultdict(list)
        for event in events:
            by_service[event.service_id].append(event)
        for service_events in by_service.values():
            service_events.sort(key=lambda event: event.timestamp)
            for left, right in combinations(service_events, 2):
                delta = abs((right.timestamp - left.timestamp).total_seconds())
                if delta > max_delta:
                    continue
                a, b = (left, right) if left.timestamp <= right.timestamp else (right, left)
                decay = self._temporal_decay((b.timestamp - a.timestamp).total_seconds())
                q = min(a.quality.overall, b.quality.overall)
                weight = decay * q
                if weight < 0.1:
                    continue
                edges.append(
                    CorrelationEdge(
                        source_event_id=a.event_id,
                        target_event_id=b.event_id,
                        edge_type="shared_service",
                        weight=round(min(1.0, weight), 4),
                        evidence=f"shared_service {a.service_id} within {delta:.1f}s",
                        reason_codes=("shared_service", "temporal_proximity"),
                    )
                )
        return edges

    def _cooccurrence_edges(self, events: list[NormalizedEvent]) -> list[CorrelationEdge]:
        edges: list[CorrelationEdge] = []
        bucket_sec = self._cfg.cooccurrence_bucket_seconds
        buckets: dict[int, list[NormalizedEvent]] = defaultdict(list)
        for event in events:
            key = int(event.timestamp.timestamp()) // max(1, bucket_sec)
            buckets[key].append(event)
        for bucket_events in buckets.values():
            if len(bucket_events) < 2:
                continue
            tag_groups: dict[str, list[NormalizedEvent]] = defaultdict(list)
            for event in bucket_events:
                for tag in event.tags & FAILURE_TAGS:
                    tag_groups[tag].append(event)
            for tag, tag_events in tag_groups.items():
                if len({event.service_id for event in tag_events}) < 2:
                    continue
                for left, right in combinations(sorted(tag_events, key=lambda event: event.event_id), 2):
                    delta = abs((right.timestamp - left.timestamp).total_seconds())
                    decay = self._temporal_decay(delta)
                    q = min(left.quality.overall, right.quality.overall)
                    weight = 0.5 * decay * q
                    if weight < 0.1:
                        continue
                    a, b = (left, right) if left.timestamp <= right.timestamp else (right, left)
                    edges.append(
                        CorrelationEdge(
                            source_event_id=a.event_id,
                            target_event_id=b.event_id,
                            edge_type="co_occurrence",
                            weight=round(min(1.0, weight), 4),
                            evidence=f"co_occurrence bucket tag={tag}",
                            reason_codes=("co_occurrence", f"tag:{tag}"),
                        )
                    )
        return edges

    def _cascade_edges(self, events: list[NormalizedEvent]) -> list[CorrelationEdge]:
        edges: list[CorrelationEdge] = []
        window = float(self._cfg.cascade_window_seconds)
        errors = [event for event in events if event.severity >= Severity.ERROR]
        for upstream in errors:
            for downstream in errors:
                if upstream.event_id == downstream.event_id:
                    continue
                latency = (downstream.timestamp - upstream.timestamp).total_seconds()
                if latency <= 0 or latency > window:
                    continue
                if downstream.service_id not in self.service_graph.get_dependents(upstream.service_id):
                    continue
                w = self._cascade_directional_weight(latency) * min(upstream.quality.overall, downstream.quality.overall)
                if w < 0.1:
                    continue
                edges.append(
                    CorrelationEdge(
                        source_event_id=upstream.event_id,
                        target_event_id=downstream.event_id,
                        edge_type="cascade",
                        weight=round(min(1.0, w), 4),
                        evidence=(
                            f"cascade {upstream.service_id} -> {downstream.service_id} latency={latency:.2f}s"
                        ),
                        reason_codes=("cascade_downstream_error",),
                    )
                )
        return edges

    def _merge_edges(self, edges: list[CorrelationEdge]) -> list[CorrelationEdge]:
        best: dict[tuple[str, str], CorrelationEdge] = {}
        for edge in edges:
            key = tuple(sorted((edge.source_event_id, edge.target_event_id)))
            existing = best.get(key)
            if existing is None or edge.weight > existing.weight:
                best[key] = edge
            elif abs(edge.weight - existing.weight) <= 1e-9:
                codes = tuple(sorted(set(existing.reason_codes) | set(edge.reason_codes)))
                best[key] = CorrelationEdge(
                    source_event_id=existing.source_event_id,
                    target_event_id=existing.target_event_id,
                    edge_type=sorted([existing.edge_type, edge.edge_type])[0],
                    weight=existing.weight,
                    evidence=existing.evidence + " | " + edge.evidence,
                    reason_codes=codes,
                )
        return list(best.values())

