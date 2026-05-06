from __future__ import annotations

import math
from collections import defaultdict
from dataclasses import dataclass

import networkx as nx

from config.models import InferenceGraphConfig
from core.enums import EventType, InferenceEdgeType, Severity
from core.models import InferenceEdge, InferenceGraph, InferenceNode
from events.models import NormalizedEvent
from runtime.service_graph import ServiceGraph


def _svc_key(service_id: str) -> str:
    return service_id.strip().lower()


def temporal_plausibility(latency_seconds: float, halflife: float = 10.0) -> float:
    if latency_seconds <= 0:
        return 0.0
    return math.exp(-0.693 * latency_seconds / halflife)


def plausibility_score(latency_seconds: float, hop_count: int = 1, halflife: float = 10.0) -> float:
    if latency_seconds <= 0:
        return 0.0
    temporal = temporal_plausibility(latency_seconds, halflife=halflife)
    topological = 1.0 / (1.0 + float(hop_count))
    return temporal * topological


@dataclass(frozen=True)
class EventIndexes:
    by_service: dict[str, list[NormalizedEvent]]
    by_host: dict[str, list[NormalizedEvent]]
    sorted_by_time: tuple[NormalizedEvent, ...]
    by_tag: dict[str, tuple[str, ...]]

    def event_map(self) -> dict[str, NormalizedEvent]:
        return {e.event_id: e for e in self.sorted_by_time}


def _events_for_service_key(indexes: EventIndexes, service_key: str) -> list[NormalizedEvent]:
    for sid, evs in indexes.by_service.items():
        if _svc_key(sid) == service_key:
            return evs
    return []


def build_indexes(events: list[NormalizedEvent]) -> EventIndexes:
    by_service: dict[str, list[NormalizedEvent]] = defaultdict(list)
    by_host: dict[str, list[NormalizedEvent]] = defaultdict(list)
    by_tag_lists: dict[str, list[str]] = defaultdict(list)
    for event in events:
        by_service[event.service_id].append(event)
        by_host[event.host_id].append(event)
        for tag in sorted(event.tags):
            by_tag_lists[tag].append(event.event_id)
    for key in by_service:
        by_service[key].sort(key=lambda e: (e.timestamp, e.event_id))
    for key in by_host:
        by_host[key].sort(key=lambda e: (e.timestamp, e.event_id))
    by_tag = {k: tuple(sorted(set(v), key=str)) for k, v in sorted(by_tag_lists.items())}
    ordered = tuple(sorted(events, key=lambda e: (e.timestamp, e.event_id)))
    return EventIndexes(
        by_service={k: by_service[k] for k in sorted(by_service)},
        by_host={k: by_host[k] for k in sorted(by_host)},
        sorted_by_time=ordered,
        by_tag=by_tag,
    )


def _edge_key(edge: InferenceEdge) -> tuple[float, str, str, str]:
    return (-edge.plausibility, edge.source_event_id, edge.target_event_id, edge.edge_type.value)


def _enforce_dag(nodes: set[str], edges: list[InferenceEdge]) -> list[InferenceEdge]:
    dag = nx.DiGraph()
    dag.add_nodes_from(sorted(nodes))
    accepted: list[InferenceEdge] = []
    for edge in sorted(edges, key=_edge_key):
        dag.add_edge(edge.source_event_id, edge.target_event_id)
        if not nx.is_directed_acyclic_graph(dag):
            dag.remove_edge(edge.source_event_id, edge.target_event_id)
        else:
            accepted.append(edge)
    return accepted


def _apply_degree_caps(edges: list[InferenceEdge], max_per_node: int) -> list[InferenceEdge]:
    out_degree: dict[str, int] = defaultdict(int)
    in_degree: dict[str, int] = defaultdict(int)
    kept: list[InferenceEdge] = []
    for edge in sorted(edges, key=_edge_key):
        if out_degree[edge.source_event_id] >= max_per_node or in_degree[edge.target_event_id] >= max_per_node:
            continue
        kept.append(edge)
        out_degree[edge.source_event_id] += 1
        in_degree[edge.target_event_id] += 1
    return kept


def build_inference_graph(
    events: list[NormalizedEvent],
    service_graph: ServiceGraph,
    config: InferenceGraphConfig,
) -> InferenceGraph:
    if not events:
        return InferenceGraph()
    max_n = int(config.max_events_for_graph)
    ordered = sorted(events, key=lambda e: (e.timestamp, e.event_id))[:max_n]
    indexes = build_indexes(ordered)
    strategies = config.strategies
    candidates: list[InferenceEdge] = []

    if strategies.dependency_propagation:
        candidates.extend(_edges_dependency_propagation(indexes, service_graph))

    if strategies.same_service_escalation:
        candidates.extend(_edges_same_service_escalation(indexes))

    if strategies.resource_preceded_error:
        candidates.extend(_edges_resource_preceded_error(indexes))

    if strategies.config_preceded_error:
        candidates.extend(_edges_config_preceded_error(indexes, service_graph))

    if strategies.restart_preceded_disconnection:
        candidates.extend(_edges_restart_preceded_disconnection(indexes, service_graph))

    if strategies.shared_fate:
        candidates.extend(_edges_shared_fate(indexes))

    if strategies.timeout_chain:
        candidates.extend(_edges_timeout_chain(indexes, service_graph))

    threshold = float(config.plausibility_threshold)
    filtered = [e for e in candidates if e.plausibility >= threshold]
    node_ids = {e.event_id for e in ordered}
    dag_edges = _enforce_dag(node_ids, filtered)
    capped = _apply_degree_caps(dag_edges, int(config.max_edges_per_node))
    nodes = _build_nodes(ordered, capped)
    roots, leaves = _roots_and_leaves(node_ids, capped)
    return InferenceGraph(nodes=nodes, edges=capped, root_candidates=roots, leaf_nodes=leaves)


def _edges_dependency_propagation(indexes: EventIndexes, graph: ServiceGraph) -> list[InferenceEdge]:
    edges: list[InferenceEdge] = []
    for service_id in sorted(indexes.by_service):
        sk = _svc_key(service_id)
        error_events = [e for e in indexes.by_service[service_id] if e.severity >= Severity.ERROR]
        if not error_events:
            continue
        neighbors = graph.get_dependencies(sk) | graph.get_dependents(sk)
        for neighbor in sorted(neighbors):
            nsk = _svc_key(neighbor)
            neighbor_events = _events_for_service_key(indexes, nsk)
            neighbor_warns = [e for e in neighbor_events if e.severity >= Severity.WARN]
            hop = graph.shortest_path_length(sk, nsk, max_depth=4)
            hop_count = hop if hop is not None else 4
            for a in error_events:
                for b in neighbor_warns:
                    latency = (b.timestamp - a.timestamp).total_seconds()
                    if 0 < latency <= 60.0:
                        pl = plausibility_score(latency, hop_count=min(hop_count, 4), halflife=12.0)
                        edges.append(
                            InferenceEdge(
                                source_event_id=a.event_id,
                                target_event_id=b.event_id,
                                edge_type=InferenceEdgeType.DEPENDENCY_PROPAGATION,
                                plausibility=round(min(0.95, pl * 0.92), 4),
                                latency_ms=round(latency * 1000.0, 3),
                                evidence=f"{service_id} error preceded {neighbor} signal by {latency:.1f}s",
                                requires=[
                                    f"service graph relation {service_id}~{neighbor} is accurate",
                                    "temporal ordering reflects dependency impact",
                                ],
                            )
                        )
    return edges


def _edges_same_service_escalation(indexes: EventIndexes) -> list[InferenceEdge]:
    edges: list[InferenceEdge] = []
    for service_id in sorted(indexes.by_service):
        sorted_svc = list(indexes.by_service[service_id])
        for idx in range(len(sorted_svc) - 1):
            a, b = sorted_svc[idx], sorted_svc[idx + 1]
            if int(b.severity) > int(a.severity):
                latency = (b.timestamp - a.timestamp).total_seconds()
                if 0 < latency <= 30.0:
                    pl = 0.7 * temporal_plausibility(latency, halflife=8.0)
                    edges.append(
                        InferenceEdge(
                            source_event_id=a.event_id,
                            target_event_id=b.event_id,
                            edge_type=InferenceEdgeType.SAME_SERVICE_ESCALATION,
                            plausibility=round(pl, 4),
                            latency_ms=round(latency * 1000.0, 3),
                            evidence=f"severity escalation on {service_id}: {a.severity.name} to {b.severity.name}",
                            requires=["severity escalation is progressive rather than coincidental"],
                        )
                    )
    return edges


def _edges_resource_preceded_error(indexes: EventIndexes) -> list[InferenceEdge]:
    edges: list[InferenceEdge] = []
    for host_id in sorted(indexes.by_host):
        host_events = indexes.by_host[host_id]
        metric_events = [
            e
            for e in host_events
            if e.event_type == EventType.METRIC or "threshold_exceeded" in e.tags or "resource_pressure" in e.tags
        ]
        error_events = [e for e in host_events if e.severity >= Severity.ERROR]
        for m in metric_events:
            for err in error_events:
                latency = (err.timestamp - m.timestamp).total_seconds()
                if 0 < latency <= 60.0:
                    pl = 0.8 * temporal_plausibility(latency, halflife=14.0)
                    edges.append(
                        InferenceEdge(
                            source_event_id=m.event_id,
                            target_event_id=err.event_id,
                            edge_type=InferenceEdgeType.RESOURCE_PRECEDED_ERROR,
                            plausibility=round(pl, 4),
                            latency_ms=round(latency * 1000.0, 3),
                            evidence=f"resource signal on host {host_id} preceded error",
                            requires=["resource signal is relevant to the downstream error"],
                        )
                    )
    return edges


def _edges_config_preceded_error(indexes: EventIndexes, graph: ServiceGraph) -> list[InferenceEdge]:
    edges: list[InferenceEdge] = []
    config_ids = list(indexes.by_tag.get("config_change", ())) + list(indexes.by_tag.get("deployment", ()))
    by_id = indexes.event_map()
    for cid in sorted(config_ids):
        ce = by_id.get(cid)
        if ce is None:
            continue
        sk = _svc_key(ce.service_id)
        reachable = {ce.service_id, *graph.get_dependents(sk)}
        for service_id in sorted(reachable):
            for err in indexes.by_service.get(service_id, []):
                if err.severity >= Severity.ERROR:
                    latency = (err.timestamp - ce.timestamp).total_seconds()
                    if 0 < latency <= 300.0:
                        pl = 0.5 * temporal_plausibility(latency, halflife=60.0)
                        edges.append(
                            InferenceEdge(
                                source_event_id=ce.event_id,
                                target_event_id=err.event_id,
                                edge_type=InferenceEdgeType.CONFIG_PRECEDED_ERROR,
                                plausibility=round(pl, 4),
                                latency_ms=round(latency * 1000.0, 3),
                                evidence="configuration or deployment change preceded errors",
                                requires=[
                                    "configuration change may be unrelated; temporal proximity is a weak signal",
                                ],
                            )
                        )
    return edges


def _edges_restart_preceded_disconnection(indexes: EventIndexes, graph: ServiceGraph) -> list[InferenceEdge]:
    edges: list[InferenceEdge] = []
    by_id = indexes.event_map()
    for rid in sorted(indexes.by_tag.get("restart", ())):
        restart_event = by_id.get(rid)
        if restart_event is None:
            continue
        sk = _svc_key(restart_event.service_id)
        dependents = graph.get_dependents(sk)
        for dep in sorted(dependents):
            dep_events = _events_for_service_key(indexes, _svc_key(dep))
            conn_errors = [
                e
                for e in dep_events
                if "connection_refused" in e.tags or "connection refused" in e.message.lower()
            ]
            for ce in conn_errors:
                latency = (ce.timestamp - restart_event.timestamp).total_seconds()
                if 0 < latency <= 30.0:
                    pl = 0.9 * temporal_plausibility(latency, halflife=6.0)
                    edges.append(
                        InferenceEdge(
                            source_event_id=restart_event.event_id,
                            target_event_id=ce.event_id,
                            edge_type=InferenceEdgeType.RESTART_PRECEDED_DISCONNECTION,
                            plausibility=round(pl, 4),
                            latency_ms=round(latency * 1000.0, 3),
                            evidence=f"restart on {restart_event.service_id} preceded connection errors on {dep}",
                            requires=["restart caused brief unavailability for dependents"],
                        )
                    )
    return edges


def _edges_shared_fate(indexes: EventIndexes) -> list[InferenceEdge]:
    edges: list[InferenceEdge] = []
    for host_id in sorted(indexes.by_host):
        errs = [e for e in indexes.by_host[host_id] if e.severity >= Severity.WARN]
        if len({e.service_id for e in errs}) < 2:
            continue
        errs = sorted(errs, key=lambda e: (e.timestamp, e.event_id))
        for idx in range(len(errs) - 1):
            a, b = errs[idx], errs[idx + 1]
            if a.service_id == b.service_id:
                continue
            latency = (b.timestamp - a.timestamp).total_seconds()
            if 0 < latency <= 20.0:
                pl = 0.55 * temporal_plausibility(latency, halflife=5.0)
                edges.append(
                    InferenceEdge(
                        source_event_id=a.event_id,
                        target_event_id=b.event_id,
                        edge_type=InferenceEdgeType.SHARED_FATE,
                        plausibility=round(pl, 4),
                        latency_ms=round(latency * 1000.0, 3),
                        evidence=f"multiple services on host {host_id} degraded in a short window",
                        requires=["shared host contention or correlated failures"],
                    )
                )
    return edges


def _edges_timeout_chain(indexes: EventIndexes, graph: ServiceGraph) -> list[InferenceEdge]:
    edges: list[InferenceEdge] = []
    timeout_events = [
        e
        for e in indexes.sorted_by_time
        if "timeout" in e.tags or "timeout" in e.message.lower() or "timed out" in e.message.lower()
    ]
    for a in timeout_events:
        sk = _svc_key(a.service_id)
        for dep in sorted(graph.get_dependencies(sk)):
            dep_events = _events_for_service_key(indexes, _svc_key(dep))
            for b in dep_events:
                if b.event_id == a.event_id:
                    continue
                if not ("timeout" in b.tags or "timeout" in b.message.lower()):
                    continue
                latency = (a.timestamp - b.timestamp).total_seconds()
                if 0 < latency <= 45.0:
                    hop = graph.shortest_path_length(_svc_key(dep), sk, max_depth=4) or 2
                    pl = plausibility_score(latency, hop_count=min(hop, 4), halflife=10.0)
                    edges.append(
                        InferenceEdge(
                            source_event_id=b.event_id,
                            target_event_id=a.event_id,
                            edge_type=InferenceEdgeType.TIMEOUT_CHAIN,
                            plausibility=round(min(0.92, pl * 0.88), 4),
                            latency_ms=round(latency * 1000.0, 3),
                            evidence=f"timeout chain from {dep} toward {a.service_id}",
                            requires=["call graph direction matches timeout propagation"],
                        )
                    )
    return edges


def _build_nodes(events: list[NormalizedEvent], edges: list[InferenceEdge]) -> dict[str, InferenceNode]:
    involved = {e.event_id for e in events}
    for edge in edges:
        involved.add(edge.source_event_id)
        involved.add(edge.target_event_id)
    out_degree: dict[str, int] = defaultdict(int)
    in_degree: dict[str, int] = defaultdict(int)
    for edge in edges:
        out_degree[edge.source_event_id] += 1
        in_degree[edge.target_event_id] += 1
    by_id = {e.event_id: e for e in events}
    nodes: dict[str, InferenceNode] = {}
    for eid in sorted(involved):
        ev = by_id.get(eid)
        if ev is None:
            continue
        if in_degree[eid] == 0:
            nt = "origin_candidate"
        elif out_degree[eid] == 0:
            nt = "symptom"
        else:
            nt = "intermediate"
        summary = ev.message[:160] if ev.message else ""
        nodes[eid] = InferenceNode(
            event_id=eid,
            service_id=ev.service_id,
            timestamp=ev.timestamp,
            severity=ev.severity,
            summary=summary,
            node_type=nt,
            in_degree=int(in_degree[eid]),
            out_degree=int(out_degree[eid]),
        )
    return nodes


def _roots_and_leaves(node_ids: set[str], edges: list[InferenceEdge]) -> tuple[list[str], list[str]]:
    out_degree: dict[str, int] = defaultdict(int)
    in_degree: dict[str, int] = defaultdict(int)
    for n in node_ids:
        out_degree[n] = 0
        in_degree[n] = 0
    for edge in edges:
        out_degree[edge.source_event_id] += 1
        in_degree[edge.target_event_id] += 1
    roots = sorted([n for n in node_ids if in_degree[n] == 0 and out_degree[n] > 0])
    leaves = sorted([n for n in node_ids if out_degree[n] == 0 and in_degree[n] > 0])
    if not roots:
        roots = sorted([n for n in node_ids if in_degree[n] == 0])
    if not leaves:
        leaves = sorted([n for n in node_ids if out_degree[n] == 0])
    return roots, leaves
