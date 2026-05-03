# Data Flow Contracts

This document defines the typed interfaces between every major subsystem. Each contract specifies the exact structure of data crossing a module boundary, the guarantees the producer provides, and the assumptions the consumer makes.

---

## Contract 1: Collector → Normalization Pipeline

### Structure: `RawEvent`

```python
@dataclass
class RawEvent:
    source_type: str          # "docker" | "journald" | "file" | "procfs" | "app"
    source_id: str            # unique collector instance id (e.g., "docker://container_abc")
    raw_payload: str          # the original log line or metric blob, unmodified
    collected_at: datetime    # wall-clock time when the collector captured this
    metadata: dict            # source-specific metadata:
                              #   docker: {"container_id", "image", "container_name"}
                              #   journald: {"unit", "priority", "syslog_identifier"}
                              #   file: {"path", "line_number", "inode"}
                              #   procfs: {"pid", "stat_type"}
```

### Producer Guarantees (Collector)
- `raw_payload` is the exact bytes read from the source (UTF-8 decoded, invalid bytes replaced with U+FFFD)
- `collected_at` is monotonically non-decreasing per collector instance
- `source_id` is stable across restarts for the same logical source

### Consumer Assumptions (Normalization)
- `raw_payload` may be malformed, empty, or binary-like
- `metadata` keys vary by `source_type`
- Events may arrive out of order across different collectors

---

## Contract 2: Normalization Pipeline → Storage

### Structure: `NormalizedEvent`

```python
@dataclass
class NormalizedEvent:
    event_id: str             # UUID v4, assigned during normalization
    timestamp: datetime       # parsed from raw_payload, or falls back to collected_at
    timestamp_source: str     # "parsed" | "collected_at" | "inferred"
    service_id: str           # canonicalized service identifier
    host_id: str              # hostname or container ID
    severity: Severity        # enum: DEBUG, INFO, WARN, ERROR, CRITICAL
    event_type: EventType     # enum: LOG, METRIC, STATE_CHANGE, HEALTH_CHECK
    message: str              # human-readable summary (first 1024 chars of meaningful content)
    structured_data: dict     # parsed key-value pairs extracted from the log
    tags: set[str]            # derived tags: {"restart", "oom", "connection_refused", ...}
    fingerprint: str          # deduplication fingerprint (see event_deduplication.md)
    quality: DataQuality      # data quality assessment (see event_model.md)
    source_ref: SourceRef     # back-reference to raw event source for drill-down
    schema_version: int       # currently 1; incremented on breaking schema changes
```

```python
class SourceRef:
    source_type: str
    source_id: str
    raw_offset: int | None    # byte offset in source file, if applicable
    collected_at: datetime
```

### Producer Guarantees (Normalization)
- `event_id` is globally unique
- `severity` is always populated (defaults to INFO if unparseable)
- `fingerprint` is deterministic: same raw_payload + service_id → same fingerprint
- `quality` is always populated with valid DataQuality scores (all fields in 0.0–1.0)
- `timestamp` is never None
- `schema_version` matches the current code version

### Consumer Assumptions (Storage)
- Events are deduplicated before reaching storage (fingerprint checked)
- Events passing validation satisfy all field constraints in event_model.md

---

## Contract 3: Storage → Analysis Layer

### Query Interface: `EventStore`

```python
class EventStore(Protocol):
    def query_time_range(
        self,
        start: datetime,
        end: datetime,
        filters: EventFilter | None = None
    ) -> Iterator[NormalizedEvent]:
        """Returns events in timestamp order. Lazily loaded."""
        ...

    def query_by_service(
        self,
        service_id: str,
        window: timedelta
    ) -> Iterator[NormalizedEvent]:
        """Returns events for a service within the lookback window."""
        ...

    def count_by_severity(
        self,
        service_id: str,
        severity: Severity,
        window: timedelta
    ) -> int:
        """Fast count query for anomaly detection."""
        ...

    def get_event(self, event_id: str) -> NormalizedEvent | None:
        ...
```

```python
@dataclass
class EventFilter:
    service_ids: set[str] | None = None
    host_ids: set[str] | None = None
    severities: set[Severity] | None = None
    event_types: set[EventType] | None = None
    tags: set[str] | None = None        # ANY match
    message_contains: str | None = None  # substring search
```

### Guarantees
- `query_time_range` returns events in ascending timestamp order
- Iterator is lazy: does not load all events into memory
- Queries against pruned data return empty results (no error)

---

## Contract 4: Analysis Layer → Incident Builder

### Structure: `EventCluster`

```python
@dataclass
class EventCluster:
    cluster_id: str
    events: list[str]              # event_ids, ordered by timestamp
    time_range: tuple[datetime, datetime]
    affected_services: set[str]
    primary_severity: Severity     # highest severity in cluster
    trigger_event_id: str          # the event that initiated cluster formation
    correlation_edges: list[CorrelationEdge]
    anomaly_scores: dict[str, float]  # service_id → anomaly score at cluster time
```

```python
@dataclass
class CorrelationEdge:
    source_event_id: str
    target_event_id: str
    edge_type: str           # "temporal" | "service_dependency" | "inferred_sequence" | "co_occurrence"
    weight: float            # 0.0 – 1.0, strength of correlation
    evidence: str            # brief justification: "events within 2s on dependent services"
```

### Guarantees
- Every `event_id` in `events` exists in the EventStore
- `time_range[0] <= time_range[1]`
- `trigger_event_id` is in `events`
- `correlation_edges` reference only events within this cluster

---

## Contract 5: Incident Builder → Reasoning Layer

### Structure: `Incident`

```python
@dataclass
class Incident:
    incident_id: str
    state: IncidentState          # OPEN, INVESTIGATING, EXPLAINED, RESOLVED, STALE
    created_at: datetime
    updated_at: datetime
    clusters: list[str]           # cluster_ids that form this incident
    events: list[str]             # all event_ids across clusters
    affected_services: set[str]
    primary_service: str | None   # service with highest anomaly score
    time_range: tuple[datetime, datetime]
    severity: Severity            # inherited from highest-severity event
    runtime_context: RuntimeContext | None
    inference_graph: InferenceGraph | None
```

### Guarantees
- `incident_id` is stable across updates (same incident, new data → same id)
- `events` list grows monotonically (events are never removed from an incident)
- `state` transitions follow the lifecycle state machine (see incident_lifecycle.md)

---

## Contract 6: Reasoning Layer → Presentation Layer

### Structure: `ScoredHypothesisSet`

```python
@dataclass
class ScoredHypothesisSet:
    incident_id: str
    hypotheses: list[ScoredHypothesis]  # sorted by total_score descending
    contradictions: list[Contradiction]
    analysis_timestamp: datetime
    analysis_duration_ms: int

@dataclass
class ScoredHypothesis:
    hypothesis_id: str
    rank: int                          # 1-indexed
    cause_type: CauseType              # enum from failure_taxonomy.md
    description: str                   # structured description (not LLM-generated)
    total_score: float                 # 0.0 – 1.0, normalized
    score_breakdown: ScoreBreakdown
    supporting_events: list[str]       # event_ids
    contradicting_events: list[str]    # event_ids
    affected_services: list[str]
    suggested_checks: list[str]        # concrete diagnostic commands
    confidence_label: str              # "high" | "medium" | "low" (from calibration)
    is_valid: bool                     # passed validation checks
    invalidation_reasons: list[str]    # if not valid, why

@dataclass
class ScoreBreakdown:
    temporal_alignment: float
    correlation_strength: float
    frequency_weight: float
    dependency_proximity: float
    evidence_coverage: float
    anomaly_severity: float
    # each in 0.0 – 1.0 range before weighting

@dataclass
class Contradiction:
    hypothesis_id: str
    contradicting_event_id: str
    contradiction_type: str        # "timeline_violation" | "state_inconsistency" | "scope_mismatch"
    explanation: str               # structured explanation of the contradiction
```

### Guarantees
- `hypotheses` list is non-empty for any incident in INVESTIGATING state
- `rank` values are contiguous starting from 1
- `total_score` is in [0.0, 1.0]
- Every `event_id` referenced exists in the event store
- `score_breakdown` components sum (weighted) to `total_score`

---

## Contract 7: Presentation Layer → LLM (Explanation Request)

### Structure: `ExplanationRequest`

```python
@dataclass
class ExplanationRequest:
    incident_id: str
    top_hypotheses: list[ScoredHypothesis]   # top 3–5
    timeline: list[TimelineEntry]
    service_topology: list[ServiceRelation]
    runtime_context_summary: dict
    contradictions: list[Contradiction]
    output_format: str                        # "full" | "summary" | "timeline_narrative"
```

```python
@dataclass
class TimelineEntry:
    timestamp: datetime
    service_id: str
    severity: Severity
    summary: str           # 1-line summary of the event
    is_key_event: bool     # flagged by the hypothesis as important evidence

@dataclass
class ServiceRelation:
    source: str
    target: str
    relation_type: str     # "calls" | "depends_on" | "shares_host"
```

### Guarantees
- All service names are canonicalized (no raw hostnames or IPs)
- No raw log content is included (only summaries)
- Sensitive fields are stripped before this structure is created
- Timeline is sorted ascending

---

## Contract 8: LLM → Presentation Layer (Explanation Response)

### Structure: `ExplanationResult`

```python
@dataclass
class ExplanationResult:
    incident_id: str
    summary: str                    # 2-3 sentence overview
    primary_hypothesis_text: str    # human-readable explanation of top hypothesis
    evidence_narrative: str         # paragraph linking evidence to conclusion
    timeline_narrative: str         # chronological story of what happened
    alternative_explanations: list[str]  # brief descriptions of other hypotheses
    suggested_actions: list[str]    # operator-facing next steps
    uncertainty_notes: list[str]    # where the system is unsure
    generation_model: str           # which LLM or template was used
    guardrail_violations: list[str] # any claims that were stripped or modified
```

### Guarantees
- `summary` is <500 characters
- `guardrail_violations` is populated if post-processing modified the LLM output
- If LLM was unavailable, `generation_model` is "template_fallback" and outputs are structured but less natural

---

## Data Flow Invariants

1. **No backward data flow**: Data moves from Collection → Normalization → Storage → Analysis → Reasoning → Presentation. No subsystem writes back to a subsystem earlier in the pipeline.

2. **Event immutability**: Once a `NormalizedEvent` is written to storage, it is never modified. Incidents and hypotheses are append-only (new versions, not mutations).

3. **Reference integrity**: Any `event_id` referenced by any downstream structure MUST exist in the event store at the time of reference. Pruning old events requires first resolving or archiving incidents that reference them.

4. **Schema versioning**: All persisted structures include a `schema_version` field. Readers must handle version mismatches by either migrating or rejecting with a clear error.
