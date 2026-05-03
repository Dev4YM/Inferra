# Incident Lifecycle

## Purpose

An incident is the central unit of analysis in Inferra. It represents a group of correlated events that collectively indicate a system problem. This document defines how incidents are created, progress through states, merge or split, and eventually resolve.

---

## Incident States

```
                              ┌─────────────────────────────────┐
                              │                                 │
    ┌──────────┐         ┌────▼──────┐         ┌─────────────┐  │
    │          │ cluster  │           │ hypotheses│             │  │
    │  (none)  ├─────────▶│   OPEN    ├──────────▶│INVESTIGATING│──┘ new events
    │          │ detected │           │ generated │             │   (re-analyze)
    └──────────┘         └────┬──────┘         └──────┬──────┘
                              │                       │
                              │ no events for         │ explanation
                              │ stale_timeout         │ generated
                              │                       ▼
                              │              ┌─────────────┐
                              │              │  EXPLAINED   │
                              │              │              │
                              │              └──────┬──────┘
                              │                     │
                              │                     │ operator marks
                              │                     │ as resolved
                              ▼                     ▼
                        ┌──────────┐         ┌─────────────┐
                        │  STALE   │         │  RESOLVED   │
                        │          │         │             │
                        └──────────┘         └─────────────┘
```

### State Definitions

| State | Description | Entry Condition | Exit Condition |
|---|---|---|---|
| **OPEN** | Cluster detected, incident created. Events are being collected. | Correlation engine produces a new cluster | Hypotheses generated → INVESTIGATING; no new events for `stale_timeout` → STALE |
| **INVESTIGATING** | Hypotheses have been generated and scored. Active analysis. | Hypothesis engine produces ≥1 hypothesis | Explanation generated → EXPLAINED; new events arrive → stay in INVESTIGATING (re-analyze) |
| **EXPLAINED** | Full analysis complete, explanation available. | Explanation layer produces output | Operator resolves → RESOLVED; new events arrive → back to INVESTIGATING |
| **RESOLVED** | Operator has acknowledged/resolved the incident. | Operator action in UI | Archived after retention period |
| **STALE** | No new events for `stale_timeout`. Incident likely self-resolved. | No new events added for `stale_timeout` (default: 15 min) | Archived after retention period |

---

## State Transitions

```python
ALLOWED_TRANSITIONS = {
    IncidentState.OPEN: {IncidentState.INVESTIGATING, IncidentState.STALE},
    IncidentState.INVESTIGATING: {IncidentState.EXPLAINED, IncidentState.INVESTIGATING, IncidentState.STALE},
    IncidentState.EXPLAINED: {IncidentState.RESOLVED, IncidentState.INVESTIGATING},
    IncidentState.RESOLVED: set(),  # terminal state
    IncidentState.STALE: set(),     # terminal state
}

def transition(incident: Incident, new_state: IncidentState, reason: str) -> None:
    if new_state not in ALLOWED_TRANSITIONS[incident.state]:
        raise InvalidTransition(f"Cannot transition from {incident.state} to {new_state}")

    incident.state = new_state
    incident.updated_at = datetime.utcnow()
    log_transition(incident.incident_id, incident.state, new_state, reason)
```

### Transition: EXPLAINED → INVESTIGATING (Re-analysis)

When new events are added to an incident that was already explained:
1. The incident moves back to INVESTIGATING
2. The hypothesis engine re-runs with the expanded event set
3. The scoring engine re-ranks hypotheses
4. A new explanation is generated (the old one is kept in history)

This handles evolving incidents where the initial analysis was based on partial data.

---

## Incident Creation

An incident is created when the correlation engine produces a new `EventCluster` that does not match any existing active incident.

```python
def maybe_create_incident(cluster: EventCluster, active_incidents: list[Incident]) -> Incident | None:
    # Check if this cluster should be merged into an existing incident
    for incident in active_incidents:
        if should_merge(cluster, incident):
            merge_cluster_into_incident(cluster, incident)
            return None  # no new incident, existing one updated

    # New incident
    return Incident(
        incident_id=generate_incident_id(),
        state=IncidentState.OPEN,
        created_at=datetime.utcnow(),
        updated_at=datetime.utcnow(),
        clusters=[cluster.cluster_id],
        events=cluster.events,
        affected_services=cluster.affected_services,
        primary_service=identify_primary_service(cluster),
        time_range=cluster.time_range,
        severity=cluster.primary_severity,
        runtime_context=None,  # populated when analysis starts
        inference_graph=None,  # populated by inference graph engine
    )
```

### Merge Criteria

A new cluster is merged into an existing incident if:

```python
def should_merge(cluster: EventCluster, incident: Incident) -> bool:
    # Shared services
    service_overlap = cluster.affected_services & incident.affected_services
    if not service_overlap:
        return False

    # Temporal overlap or adjacency
    time_gap = max(
        (cluster.time_range[0] - incident.time_range[1]).total_seconds(),
        (incident.time_range[0] - cluster.time_range[1]).total_seconds(),
    )
    if time_gap > MERGE_TIME_THRESHOLD:  # default: 300 seconds (5 min)
        return False

    # At least one shared event
    shared_events = set(cluster.events) & set(incident.events)
    if shared_events:
        return True

    # High service overlap ratio
    overlap_ratio = len(service_overlap) / min(len(cluster.affected_services), len(incident.affected_services))
    if overlap_ratio >= 0.5 and time_gap < 60:
        return True

    return False
```

---

## Incident Splitting

Rarely, an incident may need to be split — when analysis reveals that two unrelated failures were incorrectly clustered together.

**Trigger**: The hypothesis engine generates hypotheses with completely disjoint affected service sets, AND the inference graph has disconnected components.

```python
def should_split(incident: Incident, hypotheses: list[Hypothesis],
                   inference_graph: InferenceGraph | None) -> bool:
    if inference_graph is None:
        return False

    components = list(nx.weakly_connected_components(inference_graph.to_networkx()))
    if len(components) <= 1:
        return False

    # Check if hypothesis service sets are disjoint
    service_sets = [set(h.affected_services) for h in hypotheses]
    for i, s1 in enumerate(service_sets):
        for s2 in service_sets[i+1:]:
            if not s1 & s2:
                return True

    return False
```

**Split operation**:
1. Identify disconnected components in the inference graph
2. Create new incidents for each component
3. Move events to the appropriate new incident
4. Mark the original incident as `RESOLVED` with reason `"split into {new_ids}"`

Splitting is conservative — it only happens when there's strong evidence of independence.

---

## Staleness Detection

A background task checks for stale incidents every 60 seconds:

```python
async def check_staleness(incidents: list[Incident]) -> None:
    cutoff = datetime.utcnow() - timedelta(seconds=STALE_TIMEOUT)

    for incident in incidents:
        if incident.state in (IncidentState.OPEN, IncidentState.INVESTIGATING, IncidentState.EXPLAINED):
            if incident.updated_at < cutoff:
                last_event_time = get_last_event_time(incident)
                if last_event_time and last_event_time < cutoff:
                    transition(incident, IncidentState.STALE,
                              f"No new events for {STALE_TIMEOUT}s")
```

**Stale timeout**: Default 15 minutes. Configurable.

Stale incidents are not deleted — they are available for review and contribute to calibration data. They are archived after 7 days.

---

## Incident Deduplication

To prevent duplicate incidents for the same failure:

1. **Fingerprint-based**: Each incident has a fingerprint computed from `frozenset(affected_services) + primary_severity + time_bucket(created_at, resolution=5min)`.
2. If a new incident's fingerprint matches an active incident's fingerprint, it is merged instead of created.
3. This handles the case where the correlation engine produces slightly different clusters for the same ongoing failure on consecutive ticks.

---

## Event Addition

Events are added to incidents by the correlation engine on each analysis tick:

```python
def add_events_to_incident(incident: Incident, new_event_ids: list[str]) -> bool:
    """Add events to incident. Returns True if any new events were added."""
    existing = set(incident.events)
    added = [eid for eid in new_event_ids if eid not in existing]

    if not added:
        return False

    incident.events.extend(added)
    incident.updated_at = datetime.utcnow()

    # Expand time range if needed
    for eid in added:
        event = event_store.get_event(eid)
        if event:
            incident.time_range = (
                min(incident.time_range[0], event.timestamp),
                max(incident.time_range[1], event.timestamp),
            )

    # Expand affected services
    for eid in added:
        event = event_store.get_event(eid)
        if event:
            incident.affected_services.add(event.service_id)

    # Trigger re-analysis if incident was already explained
    if incident.state == IncidentState.EXPLAINED:
        transition(incident, IncidentState.INVESTIGATING, "New events added")

    return True
```

---

## Resolution

Resolution is an operator action:

```python
@dataclass
class ResolutionInfo:
    resolved_by: str           # "operator" | "auto_stale"
    correct_hypothesis_id: str | None
    feedback_type: str         # "confirmed" | "none_correct" | "skipped"
    notes: str = ""
    resolved_at: datetime = field(default_factory=datetime.utcnow)

def resolve_incident(incident: Incident, resolution: ResolutionInfo) -> None:
    transition(incident, IncidentState.RESOLVED, f"Resolved by {resolution.resolved_by}")

    # Store feedback for calibration
    if resolution.feedback_type != "skipped":
        feedback = IncidentFeedback(
            incident_id=incident.incident_id,
            resolved_at=resolution.resolved_at,
            correct_hypothesis_id=resolution.correct_hypothesis_id,
            feedback_type=resolution.feedback_type,
            operator_notes=resolution.notes,
        )
        calibration_model.update(feedback)
```

---

## Archival

Resolved and stale incidents older than the archive threshold (default: 7 days) are moved to archive storage:

1. Incident record + hypotheses + explanations → `./data/archive/incidents_YYYYMMDD.db`
2. Event references are preserved (events may still be in the event store if within retention)
3. Active incident store entry is deleted

Archived incidents are read-only. They contribute to trend analysis and calibration but are not visible in the active incident list.

---

## Configuration

```toml
[incident_lifecycle]
stale_timeout_seconds = 900          # 15 minutes
merge_time_threshold_seconds = 300   # 5 minutes
archive_after_days = 7
enable_auto_split = true
staleness_check_interval_seconds = 60

[incident_lifecycle.limits]
max_events_per_incident = 10000
max_active_incidents = 200
max_clusters_per_incident = 20
```

---

## Metrics

| Metric | Description |
|---|---|
| `incidents_created_total` | Total incidents created |
| `incidents_resolved_total` | Total incidents resolved (by type: operator, auto_stale) |
| `incidents_merged_total` | Total merge operations |
| `incidents_split_total` | Total split operations |
| `incident_active_count` | Currently active (non-terminal) incidents |
| `incident_duration_seconds` | Histogram of incident duration (created → resolved/stale) |
| `incident_events_count` | Histogram of event count per incident |
