# Correlation Engine

## Purpose

The correlation engine discovers relationships between events that may represent a single underlying failure. It answers: "Which events are related to each other, and how?"

Its output is a set of `EventCluster` objects — groups of events connected by temporal, topological, or causal relationships — that the Incident Builder then promotes into formal incidents.

---

## Input

- Stream of `NormalizedEvent` objects from the event store (queried by time window)
- Service graph (from the `ServiceGraphCache`)
- Anomaly signals (from the anomaly detector)
- Runtime context (from the runtime context builder)

## Output

- `list[EventCluster]` — groups of correlated events with typed edges

---

## Correlation Strategies

The engine applies multiple correlation strategies in parallel, then merges their results.

### Strategy 1: Temporal Proximity

Events from the same or related services occurring within a configurable time window are candidates for correlation.

**Algorithm**:
```
for each new error/critical event E:
    window = [E.timestamp - lookback, E.timestamp + lookahead]
    candidates = event_store.query_time_range(window, severity >= WARN)
    for each candidate C:
        if C.service_id in related_services(E.service_id):
            emit edge(E, C, type="temporal", weight=temporal_decay(E, C))
```

**Temporal decay function**:
```python
def temporal_decay(event_a: NormalizedEvent, event_b: NormalizedEvent) -> float:
    """Weight decays with time distance. Events closer in time are more strongly correlated."""
    delta = abs((event_a.timestamp - event_b.timestamp).total_seconds())
    # Exponential decay with half-life of 10 seconds
    half_life = 10.0
    return math.exp(-0.693 * delta / half_life)
```

**Parameters**:
- `lookback`: 30 seconds (default) — how far back to look for correlated events
- `lookahead`: 10 seconds (default) — how far ahead (for cascading effects)
- `half_life`: 10 seconds — temporal decay half-life
- `min_weight`: 0.1 — edges below this weight are discarded

---

### Strategy 2: Service Graph Traversal

Events on services connected in the service dependency graph are more likely to be causally related.

**Algorithm**:
```
for each new error/critical event E:
    neighbors = service_graph.get_dependencies(E.service_id)
                + service_graph.get_dependents(E.service_id)
    for neighbor in neighbors:
        neighbor_events = event_store.query_by_service(
            neighbor, window=timedelta(seconds=60)
        )
        for NE in neighbor_events:
            if NE.severity >= WARN:
                hop_distance = service_graph.shortest_path_length(E.service_id, NE.service_id)
                weight = 1.0 / (1.0 + hop_distance)
                emit edge(E, NE, type="service_dependency", weight=weight * temporal_decay(E, NE))
```

**Dependency proximity scoring**:
- Direct dependency (1 hop): weight multiplier = 1.0
- 2 hops: weight multiplier = 0.5
- 3 hops: weight multiplier = 0.33
- >3 hops: not considered (too distant)

---

### Strategy 3: Co-Occurrence Patterns

Events with the same tag or similar structured data that appear across multiple services within a time window suggest a shared cause.

**Algorithm**:
```
for each time bucket (5-second granularity):
    events_in_bucket = event_store.query_time_range(bucket_start, bucket_end, severity >= WARN)
    tag_groups = group_by_tags(events_in_bucket)
    for tag, tag_events in tag_groups:
        if len(tag_events) > 1 and len(unique_services(tag_events)) > 1:
            for pair in combinations(tag_events, 2):
                emit edge(pair[0], pair[1], type="co_occurrence",
                         weight=0.5 * temporal_decay(pair[0], pair[1]))
```

**Relevant tags for co-occurrence**: `connection_refused`, `timeout`, `oom`, `disk_full`, `dns_failure`, `certificate_error`. Informational tags like `deployment` or `config_change` are treated as context, not co-occurrence triggers.

---

### Strategy 4: Error Cascade Detection

Detects temporal sequences where an error in one service precedes errors in dependent services — a classic failure propagation pattern.

**Algorithm**:
```
for each ERROR/CRITICAL event E on service S:
    dependents = service_graph.get_dependents(S)  # services that depend on S
    for dep in dependents:
        # Look for errors in dependents AFTER E
        downstream_errors = event_store.query_by_service(
            dep, time_range=(E.timestamp, E.timestamp + cascade_window)
        ).filter(severity >= ERROR)

        for DE in downstream_errors:
            latency = (DE.timestamp - E.timestamp).total_seconds()
            if latency > 0:  # downstream error occurred after upstream error
                weight = temporal_decay_directional(latency, max_seconds=30)
                emit edge(E, DE, type="causal", weight=weight)
```

**Directional decay**: Unlike symmetric temporal proximity, cascade detection uses a directional model — the upstream event must precede the downstream event:

```python
def temporal_decay_directional(latency_seconds: float, max_seconds: float = 30.0) -> float:
    """High weight for short latencies, zero for negative or too-long latencies."""
    if latency_seconds <= 0 or latency_seconds > max_seconds:
        return 0.0
    return 1.0 - (latency_seconds / max_seconds)
```

---

## Clustering Algorithm

After all strategies have emitted edges, the engine forms clusters using connected component analysis on the edge graph.

### Step 1: Build Weighted Edge Graph

```python
graph = nx.Graph()
for edge in all_edges:
    if graph.has_edge(edge.source, edge.target):
        # Merge: keep maximum weight, combine types
        existing = graph[edge.source][edge.target]
        existing["weight"] = max(existing["weight"], edge.weight)
        existing["types"].add(edge.edge_type)
    else:
        graph.add_edge(edge.source, edge.target,
                       weight=edge.weight, types={edge.edge_type})
```

### Step 2: Prune Weak Edges

Remove edges with weight below `cluster_min_edge_weight` (default: 0.15). These are too weakly correlated to justify clustering.

### Step 3: Extract Connected Components

```python
components = list(nx.connected_components(graph))
```

Each connected component becomes an `EventCluster`.

### Step 4: Filter Trivial Clusters

Clusters with only 1 event are not promoted to incidents (a single error with no correlated events is logged but not investigated). Minimum cluster size: 2 events.

### Step 5: Assign Cluster Properties

```python
for component in components:
    events = [event_store.get(eid) for eid in component]
    cluster = EventCluster(
        cluster_id=generate_cluster_id(),
        events=sorted(component, key=lambda e: events[e].timestamp),
        time_range=(min(e.timestamp for e in events), max(e.timestamp for e in events)),
        affected_services={e.service_id for e in events},
        primary_severity=max(e.severity for e in events),
        trigger_event_id=earliest_high_severity_event(events).event_id,
        correlation_edges=edges_within_component(component, graph),
        anomaly_scores=get_anomaly_scores(affected_services),
    )
```

---

## Execution Model

The correlation engine runs on a periodic tick:

```
Every analysis_interval (default: 5 seconds):
    1. Query events from last analysis_window (default: 60 seconds)
    2. Include events from active (unclosed) clusters for continuity
    3. Run all four correlation strategies
    4. Build edge graph
    5. Extract clusters
    6. Diff against previous clusters:
       - New clusters → emit to Incident Builder
       - Growing clusters (new events added) → update existing incident
       - Unchanged clusters → no action
       - Shrinking/split clusters → rare, handle via incident merge/split
```

**Incremental processing**: The engine does not re-correlate the entire event history on each tick. It processes only events in the current window, plus "boundary events" from active clusters that extend beyond the window. This keeps computation bounded.

---

## Cluster Merging

Two clusters are merged if:
1. They share at least one common service AND one common time bucket (5-second granularity), OR
2. A new edge connects an event in cluster A to an event in cluster B

Merging creates a new cluster containing the union of both clusters' events and edges.

---

## Performance Budget

| Operation | Budget | Notes |
|---|---|---|
| Full tick (all strategies) | <200ms | For up to 1000 events in window |
| Temporal proximity scan | <50ms | Index-backed time range query |
| Service graph traversal | <30ms | In-memory graph, max 3 hops |
| Co-occurrence grouping | <20ms | Hash-based grouping |
| Cascade detection | <50ms | Bounded by dependency fan-out |
| Clustering | <50ms | NetworkX connected components |

---

## Configuration

```toml
[correlation]
analysis_interval_seconds = 5
analysis_window_seconds = 60

# Temporal proximity
temporal_lookback_seconds = 30
temporal_lookahead_seconds = 10
temporal_half_life_seconds = 10.0

# Service graph
max_hop_distance = 3
dependency_weight_decay = "inverse"  # 1/(1+hops)

# Co-occurrence
cooccurrence_bucket_seconds = 5

# Cascade detection
cascade_window_seconds = 30

# Clustering
cluster_min_edge_weight = 0.15
cluster_min_events = 2

# Merging
merge_on_shared_service_and_time = true
```

---

## Failure Modes

| Failure | Impact | Mitigation |
|---|---|---|
| Service graph is empty (no topology) | Only temporal and co-occurrence strategies work | Strategies degrade gracefully; graph-based edges simply absent |
| Analysis tick takes >200ms | Correlation lags behind real-time | Skip oldest events, log warning, increase interval |
| Too many clusters (>100 per tick) | Incident builder overwhelmed | Merge aggressively, raise cluster_min_events |
| False correlations (unrelated events clustered) | Bad incidents generated | Raise cluster_min_edge_weight, add manual cluster split in UI |

---

## False Correlation Prevention

The correlation engine's biggest risk is clustering unrelated events together. A bad cluster means a bad incident, bad hypotheses, and wasted operator time. The following mechanisms prevent false correlations:

### Edge Qualification Rules

Not every temporal proximity or shared tag produces an edge. Edges must pass qualification:

```python
def qualifies_for_edge(event_a: NormalizedEvent, event_b: NormalizedEvent,
                         edge_type: str, weight: float) -> bool:
    """Gate function: prevents low-quality or nonsensical edges from entering the graph."""

    # Rule 1: Minimum data quality
    # Both events must have sufficient parse quality. Garbage-parsed events
    # should not drive correlation decisions.
    min_quality = min(event_a.quality.overall, event_b.quality.overall)
    if min_quality < 0.3:
        return False

    # Rule 2: Minimum weight after quality adjustment
    adjusted_weight = weight * min_quality
    if adjusted_weight < 0.1:
        return False

    # Rule 3: Service identity must be real (not "unknown")
    if event_a.service_id == "unknown" or event_b.service_id == "unknown":
        if edge_type != "co_occurrence":
            return False  # dependency/cascade edges need real service IDs

    # Rule 4: Severity floor for dependency/cascade edges
    # Don't create cascade edges from INFO/DEBUG events
    if edge_type in ("service_dependency", "causal"):
        if event_a.severity < Severity.WARN and event_b.severity < Severity.WARN:
            return False

    # Rule 5: Self-loop prevention
    if event_a.event_id == event_b.event_id:
        return False

    return True
```

### Noise Suppression in Clustering

```python
def suppress_noise_clusters(clusters: list[EventCluster]) -> list[EventCluster]:
    """Post-clustering filter to remove clusters that are likely noise."""
    valid = []
    for cluster in clusters:
        # Reject clusters where ALL events are INFO/DEBUG
        max_severity = max(e.severity for e in cluster.events)
        if max_severity < Severity.WARN:
            continue

        # Reject clusters with very low average edge weight
        if cluster.correlation_edges:
            avg_weight = sum(e.weight for e in cluster.correlation_edges) / len(cluster.correlation_edges)
            if avg_weight < 0.2:
                continue

        # Reject clusters that span >5 minutes with <3 events
        # (probably coincidental temporal proximity)
        span = (cluster.time_range[1] - cluster.time_range[0]).total_seconds()
        if span > 300 and len(cluster.events) < 3:
            continue

        valid.append(cluster)
    return valid
```

### What "Related Services" Means (Precisely)

The temporal proximity strategy uses `related_services(E.service_id)` — here's the exact definition:

```python
def related_services(service_id: str, service_graph: ServiceGraphCache) -> set[str]:
    """Services that are topologically related and therefore plausible correlation candidates."""
    related = {service_id}  # always include self

    # Direct dependencies (services this service calls)
    related |= set(service_graph.get_dependencies(service_id))

    # Direct dependents (services that call this service)
    related |= set(service_graph.get_dependents(service_id))

    # Shared-host services (co-located on same machine/VM)
    related |= set(service_graph.get_colocated(service_id))

    # 2-hop: dependencies of dependencies (for cascade detection)
    for dep in list(service_graph.get_dependencies(service_id)):
        related |= set(service_graph.get_dependencies(dep))

    return related
```

If the service graph is empty (no topology configured), `related_services` returns only `{service_id}` — meaning only same-service events get temporal edges. This is conservative but correct: without topology information, cross-service correlation is too likely to produce false positives.

---

## Assumptions

1. The service graph is accurate enough for dependency-based correlation. If it's wrong, cascade detection produces false positives. (Mitigated: config-first topology — see `runtime_context_builder.md`)
2. Clock skew between services is <5 seconds. Larger skew breaks temporal ordering assumptions.
3. Events arrive within 30 seconds of their actual timestamp. Extreme delivery lag reduces correlation accuracy.
4. The 60-second analysis window is sufficient to capture most failure cascades. Multi-minute cascades may be split across clusters (incident merging handles this).
5. Events with `quality.overall < 0.3` are too unreliable for correlation and are excluded from edge generation.
