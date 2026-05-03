# Inference Graph Engine

## What This Is (and What It Is Not)

This engine constructs a directed acyclic graph (DAG) representing **inferred probable sequences** between events within an incident. The direction of edges is determined by temporal ordering, service topology, and pattern-matching rules.

**This is not causal inference.** True causal inference requires interventional data or known structural equations. We have neither. What we have is: "A happened before B, and A's service is upstream of B's service, so it is *plausible* that A contributed to B." That's a useful signal, but it's not proof of causation. The system names things accordingly.

Every edge in this graph is a **plausibility judgment**, not a causal fact. The operator should treat the graph as "the most plausible sequence of events given what we observed" — a starting point for investigation, not a conclusion.

---

## Distinction from Correlation Graph

| Aspect | Correlation Graph | Inference Graph |
|---|---|---|
| Edge semantics | "these events are related" | "A plausibly preceded and contributed to B" |
| Edge direction | Undirected | Directed (inferred sequence) |
| Graph type | General graph | DAG (no cycles) |
| Construction | Statistical co-occurrence | Temporal ordering + topology + rules |
| Purpose | Clustering events | Ordering events for hypothesis generation |
| Epistemic status | "correlated" | "plausibly sequential" — NOT "caused" |

---

## Graph Structure

```python
@dataclass
class InferenceGraph:
    """Directed acyclic graph of inferred event sequences within an incident."""
    nodes: dict[str, InferenceNode]      # event_id → node
    edges: list[InferenceEdge]
    root_candidates: list[str]           # event_ids with no incoming edges (potential origin points)
    leaf_nodes: list[str]                # event_ids with no outgoing edges (terminal symptoms)

@dataclass
class InferenceNode:
    event_id: str
    service_id: str
    timestamp: datetime
    severity: Severity
    summary: str
    node_type: str                       # "origin_candidate" | "intermediate" | "symptom"
    in_degree: int
    out_degree: int

@dataclass
class InferenceEdge:
    source_event_id: str                 # earlier event
    target_event_id: str                 # later event
    edge_type: InferenceEdgeType
    plausibility: float                  # 0.0–1.0 — how plausible is this sequence?
    latency_ms: float                    # time between events
    evidence: str                        # structured justification for this edge
    requires: list[str]                  # what assumptions this edge depends on
```

### Edge Types

```python
class InferenceEdgeType(Enum):
    DEPENDENCY_PROPAGATION = "dependency_propagation"
    # Service A is upstream of B; A errored before B errored
    # Assumes: service graph is correct, temporal ordering is meaningful

    SAME_SERVICE_ESCALATION = "same_service_escalation"
    # Severity increased on the same service over time
    # Assumes: escalation is progressive, not coincidental

    RESOURCE_PRECEDED_ERROR = "resource_preceded_error"
    # Resource metric crossed threshold before error events on same host
    # Assumes: resource metric is relevant to the error

    TIMEOUT_CHAIN = "timeout_chain"
    # Timeout in B preceded timeout in A, where A calls B
    # Assumes: service graph is correct

    RESTART_PRECEDED_DISCONNECTION = "restart_preceded_disconnection"
    # Restart event preceded connection errors from dependent services
    # Assumes: restart caused brief unavailability

    CONFIG_PRECEDED_ERROR = "config_preceded_error"
    # Config/deployment change preceded errors
    # Assumes: temporal proximity implies relevance (weakest assumption)

    SHARED_FATE = "shared_fate"
    # Multiple services on same host failed around the same time
    # Assumes: shared host resource contention
```

Each edge type explicitly documents its assumptions in the `requires` field. This flows through to the UI and explanation layer, so the operator can evaluate whether the assumptions hold for their system.

---

## Construction Algorithm

### Phase 1: Event Indexing (O(N) setup)

Instead of comparing every pair O(N²), build indexes that allow targeted lookups:

```python
def build_indexes(events: list[NormalizedEvent]) -> EventIndexes:
    """Pre-index events for efficient edge generation."""
    return EventIndexes(
        by_service=group_by(events, key=lambda e: e.service_id),
        by_host=group_by(events, key=lambda e: e.host_id),
        by_time_bucket=bucket_by_time(events, bucket_seconds=2),
        by_severity=group_by(events, key=lambda e: e.severity),
        by_tag=invert_tags(events),  # tag → list[event_id]
        sorted_by_time=sorted(events, key=lambda e: e.timestamp),
    )
```

### Phase 2: Targeted Edge Generation (O(N × F) where F = fan-out per strategy)

Instead of pairwise comparison, each strategy uses the indexes to find only relevant pairs:

```python
def generate_edges(events: list[NormalizedEvent],
                    indexes: EventIndexes,
                    service_graph: ServiceGraphCache,
                    budget_ms: int = 100) -> list[InferenceEdge]:
    start = time.monotonic_ns()
    edges = []

    # Strategy 1: Dependency propagation
    # For each service with errors, check ONLY its direct dependencies and dependents
    for service_id, service_events in indexes.by_service.items():
        error_events = [e for e in service_events if e.severity >= Severity.ERROR]
        if not error_events:
            continue

        neighbors = (service_graph.get_dependencies(service_id)
                     | service_graph.get_dependents(service_id))

        for neighbor in neighbors:
            neighbor_events = indexes.by_service.get(neighbor, [])
            neighbor_errors = [e for e in neighbor_events if e.severity >= Severity.WARN]

            for a in error_events:
                for b in neighbor_errors:
                    latency = (b.timestamp - a.timestamp).total_seconds()
                    if 0 < latency <= 60:
                        hop_count = service_graph.shortest_path_length(service_id, neighbor)
                        edges.append(InferenceEdge(
                            source_event_id=a.event_id,
                            target_event_id=b.event_id,
                            edge_type=InferenceEdgeType.DEPENDENCY_PROPAGATION,
                            plausibility=plausibility_score(latency, hop_count),
                            latency_ms=latency * 1000,
                            evidence=f"{service_id} errored {latency:.1f}s before {neighbor}",
                            requires=[f"service graph edge {service_id}→{neighbor} is correct",
                                      "temporal proximity implies contribution"],
                        ))

        # Budget check: stop if we've used too much time
        if elapsed_ms(start) > budget_ms * 0.4:
            break  # leave budget for other strategies

    # Strategy 2: Same-service escalation
    # Only compare events within the SAME service, sequential by time
    for service_id, service_events in indexes.by_service.items():
        sorted_svc = sorted(service_events, key=lambda e: e.timestamp)
        for i in range(len(sorted_svc) - 1):
            a, b = sorted_svc[i], sorted_svc[i + 1]
            if b.severity > a.severity:
                latency = (b.timestamp - a.timestamp).total_seconds()
                if latency <= 30:
                    edges.append(InferenceEdge(
                        source_event_id=a.event_id,
                        target_event_id=b.event_id,
                        edge_type=InferenceEdgeType.SAME_SERVICE_ESCALATION,
                        plausibility=0.7 * temporal_plausibility(latency),
                        latency_ms=latency * 1000,
                        evidence=f"severity escalation on {service_id}: {a.severity.name} → {b.severity.name}",
                        requires=["escalation is progressive, not coincidental"],
                    ))

    # Strategy 3: Resource → Error (same host only)
    for host_id, host_events in indexes.by_host.items():
        metric_events = [e for e in host_events if e.event_type == EventType.METRIC
                         and "threshold_exceeded" in e.tags]
        error_events = [e for e in host_events if e.severity >= Severity.ERROR]

        for m in metric_events:
            for err in error_events:
                latency = (err.timestamp - m.timestamp).total_seconds()
                if 0 < latency <= 60:
                    edges.append(InferenceEdge(
                        source_event_id=m.event_id,
                        target_event_id=err.event_id,
                        edge_type=InferenceEdgeType.RESOURCE_PRECEDED_ERROR,
                        plausibility=0.8 * temporal_plausibility(latency),
                        latency_ms=latency * 1000,
                        evidence=f"resource threshold on {host_id} preceded error",
                        requires=["resource metric is related to the error",
                                  "not a coincidence on a busy host"],
                    ))

    # Strategy 4: Config/deployment → Error
    config_events = indexes.by_tag.get("config_change", []) + indexes.by_tag.get("deployment", [])
    for ce in config_events:
        config_event = get_event(ce, events)
        reachable = ({config_event.service_id}
                     | service_graph.get_dependents(config_event.service_id))
        for service_id in reachable:
            for err in indexes.by_service.get(service_id, []):
                if err.severity >= Severity.ERROR:
                    latency = (err.timestamp - config_event.timestamp).total_seconds()
                    if 0 < latency <= 300:  # config impact can be slower
                        edges.append(InferenceEdge(
                            source_event_id=config_event.event_id,
                            target_event_id=err.event_id,
                            edge_type=InferenceEdgeType.CONFIG_PRECEDED_ERROR,
                            plausibility=0.5 * temporal_plausibility(latency, halflife=60),
                            latency_ms=latency * 1000,
                            evidence=f"config change preceded error by {latency:.0f}s",
                            requires=["config change is relevant to the error",
                                      "temporal proximity implies contribution (WEAK — config changes precede many things)"],
                        ))

    # Strategy 5: Restart → connection errors
    restart_events = indexes.by_tag.get("restart", [])
    for re_id in restart_events:
        restart_event = get_event(re_id, events)
        dependents = service_graph.get_dependents(restart_event.service_id)
        for dep in dependents:
            conn_errors = [e for e in indexes.by_service.get(dep, [])
                           if "connection_refused" in e.tags]
            for ce in conn_errors:
                latency = (ce.timestamp - restart_event.timestamp).total_seconds()
                if 0 < latency <= 30:
                    edges.append(InferenceEdge(
                        source_event_id=restart_event.event_id,
                        target_event_id=ce.event_id,
                        edge_type=InferenceEdgeType.RESTART_PRECEDED_DISCONNECTION,
                        plausibility=0.9 * temporal_plausibility(latency),
                        latency_ms=latency * 1000,
                        evidence=f"{restart_event.service_id} restart → {dep} connection refused",
                        requires=["restart caused brief unavailability"],
                    ))

    return edges
```

### Complexity Analysis

| Strategy | Complexity | Worst Case |
|---|---|---|
| Dependency propagation | O(S × D × E_s × E_d) | S=services, D=avg deps, E=errors per service |
| Same-service escalation | O(N) linear scan | One pass per service, sequential pairs only |
| Resource → Error | O(H × M × E_h) | H=hosts, M=metric events, E=errors per host |
| Config → Error | O(C × R × E_r) | C=config events, R=reachable services, E=errors |
| Restart → connection | O(R × D × E_d) | R=restarts, D=dependents, E=conn errors |

For a failure storm with 1000 events across 20 services: each strategy is O(hundreds), not O(millions). The total is bounded by the time budget (100ms default), with strategies interrupted if the budget is exceeded.

**Compared to old O(N²)**: An incident with 1000 events previously generated up to 500,000 candidate pairs. The indexed approach generates ~2,000–5,000 targeted candidates. That's a 100× reduction.

---

### Phase 3: DAG Enforcement

Same as before — greedy insertion by plausibility, skip edges that create cycles:

```python
def enforce_dag(nodes: list[str], edges: list[InferenceEdge]) -> list[InferenceEdge]:
    dag = nx.DiGraph()
    dag.add_nodes_from(nodes)
    accepted = []

    for edge in sorted(edges, key=lambda e: e.plausibility, reverse=True):
        dag.add_edge(edge.source_event_id, edge.target_event_id)
        if not nx.is_directed_acyclic_graph(dag):
            dag.remove_edge(edge.source_event_id, edge.target_event_id)
        else:
            accepted.append(edge)

    return accepted
```

### Phase 4: Root and Leaf Identification

```python
root_candidates = [n for n in dag.nodes if dag.in_degree(n) == 0]
leaf_nodes = [n for n in dag.nodes if dag.out_degree(n) == 0]
```

Root candidates are **origin point candidates** — the earliest plausible starting points of the failure sequence. They are not proven root causes.

---

## Plausibility Scoring

```python
def plausibility_score(latency_seconds: float, hop_count: int = 1,
                        halflife: float = 10.0) -> float:
    """How plausible is this inferred sequence?
    Shorter latency + fewer hops = more plausible.
    """
    if latency_seconds <= 0:
        return 0.0
    temporal = math.exp(-0.693 * latency_seconds / halflife)
    topological = 1.0 / (1.0 + hop_count)
    return temporal * topological

def temporal_plausibility(latency_seconds: float, halflife: float = 10.0) -> float:
    """Plausibility based on time gap alone."""
    if latency_seconds <= 0:
        return 0.0
    return math.exp(-0.693 * latency_seconds / halflife)
```

---

## Graph Queries

```python
class InferenceGraph:
    def paths_from_origin(self, origin_event_id: str) -> list[list[str]]:
        """All directed paths from an origin candidate to any leaf."""
        ...

    def ancestors(self, event_id: str) -> set[str]:
        """All events that plausibly preceded this event."""
        ...

    def descendants(self, event_id: str) -> set[str]:
        """All events that plausibly followed from this event."""
        ...

    def strongest_path(self, source: str, target: str) -> tuple[list[str], float]:
        """Path with highest minimum-edge plausibility."""
        ...

    def origin_impact_score(self, origin_event_id: str) -> float:
        """What fraction of symptoms are reachable from this origin?"""
        reachable = len(self.descendants(origin_event_id) & set(self.leaf_nodes))
        return reachable / max(len(self.leaf_nodes), 1)

    def assumption_set(self, path: list[str]) -> list[str]:
        """All assumptions required for a given path to be valid."""
        assumptions = []
        for i in range(len(path) - 1):
            edge = self.get_edge(path[i], path[i+1])
            assumptions.extend(edge.requires)
        return assumptions
```

The `assumption_set` method is new: it lets the hypothesis engine and explanation layer communicate *what must be true* for a given inference path to hold. This is the honest alternative to claiming causation.

---

## Configuration

```toml
[inference_graph]
budget_ms = 100                      # time budget for edge generation
max_events_for_graph = 500           # pre-filter if incident exceeds this
plausibility_threshold = 0.15        # edges below this are not included
max_edges_per_node = 10              # prevent hub nodes from dominating

[inference_graph.strategies]
dependency_propagation = true
same_service_escalation = true
resource_preceded_error = true
config_preceded_error = true
restart_preceded_disconnection = true
shared_fate = true
```

---

## Failure Modes

| Failure | Impact | Mitigation |
|---|---|---|
| Service graph wrong | False dependency edges | Edge documents its assumption; operator can verify |
| Clock skew reverses ordering | Wrong edge direction | Flag clock_skew events; reduce plausibility |
| Budget exceeded (complex incident) | Some strategies skipped | Strategies run in priority order; most important first |
| All edges pruned | Empty graph | Fall back to correlation-only; hypotheses use temporal ordering |
| Config event correlates with unrelated errors | False config_preceded_error edge | Low base plausibility (0.5); explicit "WEAK" assumption label |

---

## What This Gets Right and What It Doesn't

**Gets right**: The graph provides a structured, inspectable ordering of events that is more useful than a flat log dump. An operator looking at "A → B → C" with documented assumptions can quickly evaluate whether the sequence makes sense.

**Gets wrong**: Any sequence that doesn't follow temporal ordering (effect precedes cause due to async processing, queued messages, or batch jobs). Any dependency not in the service graph. Any failure mode where the root cause doesn't produce log output (silent failures). The graph will still produce a plausible-looking sequence — it just won't be the right one.

The system's value is in structured presentation of plausible sequences, not in proving causation.
