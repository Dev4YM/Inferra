# Storage Architecture

## Design Goals

1. **Local-first**: All data on the local filesystem. No network storage.
2. **Bounded growth**: Storage must not grow unboundedly. All stores have retention or capacity limits.
3. **Crash-safe**: The system must not corrupt data on unclean shutdown.
4. **Read-optimized**: Writes are append-heavy, but reads are the critical path (analysis queries events by time range, service, severity).

---

## Storage Components

### 1. Event Store (SQLite)

**File**: `./data/events.db`

**Mode**: WAL (Write-Ahead Logging) for concurrent reads during writes.

#### Schema

```sql
CREATE TABLE events (
    event_id        TEXT PRIMARY KEY,
    timestamp       TEXT NOT NULL,          -- ISO 8601 with microseconds
    timestamp_source TEXT NOT NULL,         -- 'parsed' | 'collected_at' | 'inferred'
    service_id      TEXT NOT NULL,
    host_id         TEXT NOT NULL,
    severity        INTEGER NOT NULL,       -- 0=DEBUG, 1=INFO, 2=WARN, 3=ERROR, 4=CRITICAL
    event_type      INTEGER NOT NULL,       -- 0=LOG, 1=METRIC, 2=STATE_CHANGE, 3=HEALTH_CHECK
    message         TEXT NOT NULL,
    structured_data TEXT,                   -- JSON blob
    tags            TEXT,                   -- comma-separated, indexed via FTS or LIKE
    fingerprint     TEXT NOT NULL,
    source_type     TEXT NOT NULL,
    source_id       TEXT NOT NULL,
    raw_offset      INTEGER,
    collected_at    TEXT NOT NULL,
    schema_version  INTEGER NOT NULL DEFAULT 1,
    inserted_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f', 'now'))
);

CREATE INDEX idx_events_timestamp ON events(timestamp);
CREATE INDEX idx_events_service_ts ON events(service_id, timestamp);
CREATE INDEX idx_events_severity_ts ON events(severity, timestamp);
CREATE INDEX idx_events_fingerprint ON events(fingerprint);
CREATE INDEX idx_events_inserted ON events(inserted_at);
```

#### Retention

A background task runs every 60 seconds:

```sql
DELETE FROM events
WHERE inserted_at < datetime('now', '-' || :retention_hours || ' hours');
```

Default `retention_hours`: 72. Configurable in `inferra.toml`.

After deletion, `PRAGMA incremental_vacuum` reclaims disk space without blocking reads.

#### Write Batching

Events are buffered in-memory and flushed to SQLite in batches of 100 or every 500ms (whichever comes first). This amortizes transaction overhead.

```python
INSERT INTO events (...) VALUES (...), (...), (...);  -- batch insert
```

#### Read Patterns


| Query                      | Index Used               | Expected Latency        |
| -------------------------- | ------------------------ | ----------------------- |
| Events in time range       | `idx_events_timestamp`   | <10ms for 1-hour window |
| Events by service + time   | `idx_events_service_ts`  | <5ms                    |
| Events by severity + time  | `idx_events_severity_ts` | <5ms                    |
| Dedup check by fingerprint | `idx_events_fingerprint` | <1ms                    |
| Count queries              | Covering indexes         | <1ms                    |


---

### 2. Incident Store (SQLite)

**File**: `./data/incidents.db`

Separate from events to avoid contention between high-frequency event writes and lower-frequency incident updates.

#### Schema

```sql
CREATE TABLE incidents (
    incident_id     TEXT PRIMARY KEY,
    state           TEXT NOT NULL,          -- 'open' | 'investigating' | 'explained' | 'resolved' | 'stale'
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    severity        INTEGER NOT NULL,
    primary_service TEXT,
    affected_services TEXT NOT NULL,        -- JSON array
    time_range_start TEXT NOT NULL,
    time_range_end  TEXT NOT NULL,
    event_count     INTEGER NOT NULL DEFAULT 0,
    schema_version  INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE incident_events (
    incident_id     TEXT NOT NULL REFERENCES incidents(incident_id),
    event_id        TEXT NOT NULL,
    added_at        TEXT NOT NULL,
    PRIMARY KEY (incident_id, event_id)
);

CREATE TABLE incident_clusters (
    incident_id     TEXT NOT NULL REFERENCES incidents(incident_id),
    cluster_id      TEXT NOT NULL,
    cluster_data    TEXT NOT NULL,          -- JSON: correlation edges, anomaly scores
    PRIMARY KEY (incident_id, cluster_id)
);

CREATE TABLE hypotheses (
    hypothesis_id   TEXT PRIMARY KEY,
    incident_id     TEXT NOT NULL REFERENCES incidents(incident_id),
    rank            INTEGER,
    cause_type      TEXT NOT NULL,
    description     TEXT NOT NULL,
    total_score     REAL,
    score_breakdown TEXT,                   -- JSON
    supporting_events TEXT,                 -- JSON array of event_ids
    contradicting_events TEXT,              -- JSON array of event_ids
    affected_services TEXT,                 -- JSON array
    suggested_checks TEXT,                  -- JSON array
    confidence_label TEXT,
    is_valid        INTEGER NOT NULL DEFAULT 1,
    invalidation_reasons TEXT,             -- JSON array
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE explanations (
    explanation_id  TEXT PRIMARY KEY,
    incident_id     TEXT NOT NULL REFERENCES incidents(incident_id),
    summary         TEXT NOT NULL,
    primary_text    TEXT NOT NULL,
    evidence_text   TEXT,
    timeline_text   TEXT,
    alternatives    TEXT,                   -- JSON array
    actions         TEXT,                   -- JSON array
    uncertainty     TEXT,                   -- JSON array
    model_used      TEXT NOT NULL,
    guardrail_flags TEXT,                   -- JSON array
    created_at      TEXT NOT NULL
);

CREATE INDEX idx_incidents_state ON incidents(state);
CREATE INDEX idx_incidents_updated ON incidents(updated_at);
CREATE INDEX idx_hypotheses_incident ON hypotheses(incident_id);
CREATE INDEX idx_explanations_incident ON explanations(incident_id);
```

#### Incident Reconstruction Queries

Full incident reconstruction (loading an incident with all its events, hypotheses, and explanation) uses these access patterns:

```sql
-- Primary: load incident metadata
SELECT * FROM incidents WHERE incident_id = ?;

-- Load all events for an incident (join through incident_events)
SELECT e.* FROM events e
JOIN incident_events ie ON e.event_id = ie.event_id
WHERE ie.incident_id = ?
ORDER BY e.timestamp;

-- Load hypotheses for incident
SELECT * FROM hypotheses WHERE incident_id = ? ORDER BY rank;

-- Load latest explanation
SELECT * FROM explanations WHERE incident_id = ? ORDER BY created_at DESC LIMIT 1;

-- Load inference graph snapshot
SELECT graph_data FROM inference_graph_snapshots WHERE incident_id = ?;

-- Dashboard: active incidents with top hypothesis
SELECT i.*, h.description, h.total_score, h.confidence_label
FROM incidents i
LEFT JOIN hypotheses h ON i.incident_id = h.incident_id AND h.rank = 1
WHERE i.state IN ('open', 'investigating', 'explained')
ORDER BY i.severity DESC, i.updated_at DESC;
```

**Indexes supporting these queries**:
```sql
-- incident_events: both directions
CREATE INDEX idx_ie_incident ON incident_events(incident_id);
CREATE INDEX idx_ie_event ON incident_events(event_id);

-- hypotheses: incident lookup + rank ordering
CREATE INDEX idx_hyp_incident_rank ON hypotheses(incident_id, rank);

-- incidents: state-based queries for dashboard
CREATE INDEX idx_incidents_state_severity ON incidents(state, severity DESC);
```

#### Retention

Incidents in `resolved` or `stale` state older than 7 days are archived (moved to `./data/archive/incidents_YYYYMMDD.db`) then deleted from the active store.

---

### 3. Metric Ringbuffer

**Purpose**: Store time-series metric data (event counts, error rates, latencies) for anomaly detection baselines.

**Implementation**: In-memory fixed-size arrays, persisted to disk on shutdown.

```python
class MetricRingbuffer:
    """Fixed-size circular buffer for one metric series."""

    def __init__(self, capacity: int = 720):
        # 720 slots × 5-minute buckets = 60 hours of data
        self.values: list[float | None] = [None] * capacity
        self.timestamps: list[datetime | None] = [None] * capacity
        self.head: int = 0
        self.count: int = 0

    def append(self, timestamp: datetime, value: float) -> None: ...
    def query_range(self, start: datetime, end: datetime) -> list[tuple[datetime, float]]: ...
    def last_n(self, n: int) -> list[tuple[datetime, float]]: ...
```

**Persistence**: On shutdown, each ringbuffer is serialized to `./data/metrics/{service_id}_{metric_name}.json`. On startup, these files are loaded back into memory.

**Capacity limits**:

- Max 100 metric series (across all services)
- Max 720 data points per series (60 hours at 5-minute resolution)
- Total memory: ~100 × 720 × 16 bytes ≈ 1.1MB

---

### 4. Baseline Store

**Purpose**: Store learned baselines for anomaly detection — what "normal" looks like for each service.

**File**: `./data/baselines/{service_id}.json`

```json
{
    "service_id": "api-gateway",
    "schema_version": 1,
    "updated_at": "2026-05-01T12:00:00Z",
    "hourly_profiles": {
        "error_rate": {
            "buckets": [0.01, 0.02, 0.01, ...],  // 168 values, one per hour of week
            "stddev": [0.005, 0.008, 0.004, ...],
            "sample_count": [48, 52, 45, ...]
        },
        "event_volume": {
            "buckets": [120, 130, 95, ...],
            "stddev": [15, 20, 12, ...],
            "sample_count": [48, 52, 45, ...]
        }
    },
    "known_patterns": [
        {
            "pattern": "connection refused",
            "normal_hourly_rate": 0.5,
            "stddev": 0.3
        }
    ]
}
```

**Update policy**: Baselines are updated every hour by incorporating the latest metric data using exponential moving average (EMA). Older data decays, giving recent weeks more weight.

---

### 5. Service Graph Cache (Persisted)

**Purpose**: In-memory representation of the service dependency graph with persistence across restarts.

**Persistence**: The service graph is written to `./data/service_graph.json` on every modification and on shutdown. On startup, the graph is loaded from this file, then updated with any new topology discovered from Docker or config.

```python
class ServiceGraphCache:
    """In-memory adjacency list for service dependencies, persisted to JSON."""

    graph: nx.DiGraph  # nodes = service_ids, edges = relationships
    _dirty: bool = False  # tracks whether persistence is needed
    _persist_path: Path

    def add_relation(self, source: str, target: str, relation_type: str,
                     origin: str = "config",
                     confidence: str = "high") -> None:
        """Add an edge. origin: 'config'|'docker_compose'|'log_inference'|'user_confirmed'"""
        ...
        self._dirty = True

    def get_dependencies(self, service_id: str) -> list[str]: ...
    def get_dependents(self, service_id: str) -> list[str]: ...
    def get_colocated(self, service_id: str) -> list[str]: ...
    def shortest_path(self, source: str, target: str) -> list[str] | None: ...
    def shortest_path_length(self, source: str, target: str) -> int | None: ...
    def subgraph_around(self, service_id: str, depth: int = 2) -> nx.DiGraph: ...

    def persist(self) -> None:
        """Write graph to disk if dirty."""
        if not self._dirty:
            return
        data = nx.node_link_data(self.graph)
        atomic_write(self._persist_path, json.dumps(data, indent=2))
        self._dirty = False

    def load(self) -> None:
        """Load graph from disk. Merge with config-defined topology."""
        if self._persist_path.exists():
            data = json.loads(self._persist_path.read_text())
            self.graph = nx.node_link_graph(data)
```

**Persistence format** (`./data/service_graph.json`):
```json
{
    "directed": true,
    "nodes": [
        {"id": "api-gateway"},
        {"id": "postgres"},
        {"id": "user-service"}
    ],
    "links": [
        {"source": "api-gateway", "target": "postgres",
         "relation_type": "depends_on", "origin": "config", "confidence": "high"},
        {"source": "api-gateway", "target": "user-service",
         "relation_type": "calls", "origin": "docker_compose", "confidence": "medium"}
    ]
}
```

**Startup sequence**:
1. Load `service_graph.json` if it exists
2. Apply topology from `inferra.toml` (override/add edges)
3. Discover Docker Compose edges if Docker is available (add as `confidence: "medium"`)
4. Mark graph as clean (`_dirty = False`)

**Capacity**: Max 500 nodes, 5000 edges. Enforced at insertion time; oldest edges evicted if limit reached.

---

### 6. Inference Graph Snapshots

**Purpose**: Persist the last inference graph per active incident so that analysis can resume after restart without recomputation.

**Storage**: Within `incidents.db`:

```sql
CREATE TABLE inference_graph_snapshots (
    incident_id     TEXT PRIMARY KEY REFERENCES incidents(incident_id),
    graph_data      TEXT NOT NULL,    -- JSON: serialized InferenceGraph
    created_at      TEXT NOT NULL,
    event_count     INTEGER NOT NULL  -- number of events when graph was built
);
```

The snapshot is updated whenever the inference graph is rebuilt for an incident. On restart, active incidents load their snapshots and skip graph reconstruction until new events arrive.

**Eviction**: Snapshots are deleted when their incident is resolved or archived.

---

## Disk Layout

```
./data/
├── events.db                  # SQLite: normalized events
├── events.db-wal              # SQLite WAL file
├── incidents.db               # SQLite: incidents, hypotheses, explanations
├── incidents.db-wal
├── metrics/
│   ├── api-gateway_error_rate.json
│   ├── api-gateway_event_volume.json
│   ├── postgres_error_rate.json
│   └── ...
├── baselines/
│   ├── api-gateway.json
│   ├── postgres.json
│   └── ...
├── archive/
│   ├── incidents_20260425.db
│   └── ...
└── config_cache/
    └── service_graph_hints.json    # optional: user-provided dependency hints
```

---

## Failure Modes


| Failure                   | Detection                  | Recovery                                                      |
| ------------------------- | -------------------------- | ------------------------------------------------------------- |
| Disk full                 | SQLite write error caught  | Switch to in-memory mode, alert user, stop retention pruning  |
| SQLite corruption         | Integrity check on startup | Rename corrupt file, recreate empty DB, log data loss warning |
| WAL file growth           | Monitor WAL size           | Force checkpoint if WAL exceeds 100MB                         |
| Baseline file corrupt     | JSON parse error on load   | Delete corrupt file, rebuild from metric ringbuffer           |
| Archive directory missing | OS error on archive write  | Create directory, retry                                       |


---

## Performance Characteristics


| Operation                     | Latency          | Notes                               |
| ----------------------------- | ---------------- | ----------------------------------- |
| Single event insert (batched) | <0.1ms amortized | Batch of 100 committed together     |
| Time range query (1 hour)     | <10ms            | Covering index                      |
| Count by severity + service   | <1ms             | Index scan                          |
| Incident update               | <5ms             | Small table                         |
| Full-text search in messages  | 10–50ms          | LIKE query; FTS5 extension optional |
| Baseline load (all services)  | <100ms           | JSON file reads                     |
| Retention prune               | <500ms           | Index-guided DELETE                 |


