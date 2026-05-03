# Event Model

## Overview

The event is the atomic unit of data in Inferra. Every signal from every source — log lines, metric samples, state changes, health check results — is normalized into a single `NormalizedEvent` structure before it enters the system.

This document defines the canonical event schema, its field semantics, validation rules, and versioning strategy.

---

## NormalizedEvent Schema

```python
@dataclass(frozen=True)
class NormalizedEvent:
    # Identity
    event_id: str               # UUID v4. Assigned at normalization time. Globally unique.

    # Temporal
    timestamp: datetime          # When the event occurred (parsed from source or inferred)
    timestamp_source: str        # "parsed" | "collected_at" | "inferred"

    # Origin
    service_id: str              # Canonical service identifier (e.g., "api-gateway", "postgres")
    host_id: str                 # Hostname, container ID, or VM identifier
    source_ref: SourceRef        # Back-reference to raw event for drill-down

    # Classification
    severity: Severity           # DEBUG=0, INFO=1, WARN=2, ERROR=3, CRITICAL=4
    event_type: EventType        # LOG, METRIC, STATE_CHANGE, HEALTH_CHECK

    # Content
    message: str                 # Human-readable summary, max 1024 characters
    structured_data: dict        # Key-value pairs extracted from the log line
    tags: frozenset[str]         # Semantic tags derived during normalization

    # Deduplication
    fingerprint: str             # SHA-256 of (service_id + message_template + severity)

    # Data Quality
    quality: DataQuality         # How much we trust this event's fields

    # Metadata
    schema_version: int          # Currently 1
```

---

## Field Specifications

### event_id
- Format: UUID v4 (RFC 4122)
- Generation: `uuid.uuid4()` during normalization
- Uniqueness: Guaranteed by UUID space. No collision handling needed.
- Immutable after creation.

### timestamp
- Format: `datetime` with microsecond precision, UTC-normalized
- Source priority:
  1. Parsed from the raw log line (e.g., syslog timestamp, JSON `timestamp` field)
  2. Falls back to `collected_at` from the RawEvent
  3. If neither is available, set to current time with `timestamp_source = "inferred"`
- Events with future timestamps (>60s ahead of ingestion time) are stored with the original timestamp but flagged with tag `clock_skew`.
- Events older than the retention window are rejected during normalization.

### service_id
- Canonical lowercase string identifier.
- Derivation rules (in order):
  1. Docker: `container_name` with common suffixes stripped (`_1`, `-replica-0`, etc.) and lowercased
  2. Journald: `syslog_identifier` or `_SYSTEMD_UNIT` (without `.service` suffix)
  3. File-based: derived from the log file path via configurable mapping in `inferra.toml`
  4. Explicit: set via structured log field `service` or `app_name`
- Max length: 128 characters
- Allowed characters: `[a-z0-9\-_.]`

### host_id
- For containers: the short container ID (first 12 hex chars)
- For hosts: the hostname
- For processes: `hostname:pid`
- Max length: 256 characters

### severity
```python
class Severity(IntEnum):
    DEBUG = 0
    INFO = 1
    WARN = 2
    ERROR = 3
    CRITICAL = 4
```

Mapping from source-specific severity levels:

| Source | DEBUG | INFO | WARN | ERROR | CRITICAL |
|---|---|---|---|---|---|
| Syslog priority | 7 | 6,5 | 4 | 3 | 2,1,0 |
| Docker log level | debug | info | warn/warning | error/err | fatal/panic/crit |
| Python logging | DEBUG | INFO | WARNING | ERROR | CRITICAL |
| Numeric (generic) | 0–9 | 10–19 | 20–29 | 30–39 | 40+ |

If severity cannot be determined from the source, it is inferred from content:
- Message contains `error`, `fail`, `exception`, `traceback` → ERROR
- Message contains `warn` → WARN
- Message contains `fatal`, `panic`, `oom`, `killed` → CRITICAL
- Default: INFO

### event_type
```python
class EventType(IntEnum):
    LOG = 0            # Free-text or structured log line
    METRIC = 1         # Numeric measurement (CPU, memory, latency, count)
    STATE_CHANGE = 2   # Container started/stopped, service up/down, config reloaded
    HEALTH_CHECK = 3   # Explicit health check result (pass/fail)
```

### message
- Maximum 1024 characters. Truncated with `…` if longer.
- For LOG events: the log message with timestamps and severity prefixes stripped.
- For METRIC events: `"{metric_name} = {value} {unit}"` (e.g., `"cpu_usage = 94.2 percent"`)
- For STATE_CHANGE events: `"{entity} {transition}"` (e.g., `"container/abc started"`)
- For HEALTH_CHECK events: `"{target} health check {result}"` (e.g., `"postgres health check failed: connection refused"`)

### structured_data
A flat dictionary of key-value pairs extracted from structured log formats (JSON logs, key=value pairs). Nested structures are flattened with dot notation: `{"request.method": "GET", "request.path": "/api/users"}`.

Reserved keys (set by the system, not from source):
- `_raw_length`: byte length of original raw payload
- `_parse_method`: which parser extracted this data ("json", "kv", "regex", "grok")
- `_truncated`: true if the original event was truncated

### tags
A frozen set of string tags derived during normalization. Tags are lowercase, alphanumeric with underscores.

**System-assigned tags** (derived from content analysis):
- `oom` — out of memory detected
- `restart` — service/container restart detected
- `connection_refused` — connection refused pattern
- `timeout` — timeout pattern
- `crash` — crash/segfault/panic detected
- `disk_full` — disk space exhaustion
- `permission_denied` — permission/access denied
- `dns_failure` — DNS resolution failure
- `certificate_error` — TLS/SSL certificate issue
- `rate_limited` — rate limiting triggered
- `clock_skew` — timestamp inconsistency detected
- `config_change` — configuration change detected
- `deployment` — deployment/release event detected

**User-defined tags**: Configurable via `inferra.toml` pattern-to-tag mappings.

### quality

A structured assessment of how much the system trusts this event's parsed fields. Computed during normalization. Flows through to scoring, where low-quality events contribute less weight to hypotheses.

```python
@dataclass(frozen=True)
class DataQuality:
    overall: float             # 0.0–1.0 composite score
    timestamp_confidence: float  # 1.0 = parsed from source; 0.5 = inferred from collected_at; 0.3 = wall-clock fallback
    parse_confidence: float    # 1.0 = structured JSON; 0.8 = known regex matched; 0.5 = partial match; 0.3 = unstructured freetext
    identity_confidence: float # 1.0 = explicit service_id from config/label; 0.7 = derived from container name; 0.4 = guessed from path
    completeness: float        # fraction of key fields actually populated (vs defaults)
```

**Computation**:
```python
def compute_quality(event: NormalizedEvent, parse_meta: ParseMetadata) -> DataQuality:
    ts_conf = 1.0 if event.timestamp_source == "parsed" else (
              0.5 if event.timestamp_source == "collected_at" else 0.3)

    parse_conf = {"json": 1.0, "syslog": 0.9, "kv": 0.8, "regex": 0.7,
                  "grok": 0.7, "freetext": 0.3}.get(parse_meta.method, 0.3)

    identity_conf = 1.0 if parse_meta.service_source == "config" else (
                    0.8 if parse_meta.service_source == "docker_label" else (
                    0.7 if parse_meta.service_source == "container_name" else (
                    0.5 if parse_meta.service_source == "journald_unit" else 0.4)))

    populated = sum(1 for v in [event.service_id != "unknown",
                                 event.host_id != "unknown",
                                 event.severity != Severity.INFO or parse_meta.severity_explicit,
                                 len(event.structured_data) > 0,
                                 len(event.tags) > 0] if v) / 5.0

    overall = 0.3 * ts_conf + 0.3 * parse_conf + 0.2 * identity_conf + 0.2 * populated

    return DataQuality(
        overall=overall,
        timestamp_confidence=ts_conf,
        parse_confidence=parse_conf,
        identity_confidence=identity_conf,
        completeness=populated,
    )
```

**Impact on downstream systems**:
- **Scoring**: Events with `quality.overall < 0.5` contribute at a 50% discount to hypothesis evidence counts and frequency scores. This prevents garbage-in-garbage-out where poorly parsed events inflate hypothesis confidence.
- **Correlation**: Edge weights are multiplied by `min(event_a.quality.overall, event_b.quality.overall)`. Low-quality events produce weaker correlation edges.
- **UI**: Events with `quality.overall < 0.5` show a warning badge. Events with `quality.overall < 0.3` are grayed out in timeline views.

---

### fingerprint
Used for deduplication. Computed as:

```python
def compute_fingerprint(service_id: str, message: str, severity: Severity) -> str:
    template = templatize_message(message)  # replace variable parts with placeholders
    raw = f"{service_id}|{template}|{severity.value}"
    return hashlib.sha256(raw.encode()).hexdigest()[:32]
```

The `templatize_message` function replaces:
- IP addresses → `<IP>`
- Timestamps → `<TS>`
- UUIDs → `<UUID>`
- Hex strings (>8 chars) → `<HEX>`
- Numbers → `<NUM>`
- Quoted strings → `<STR>`
- File paths → `<PATH>`

Two events with the same fingerprint are considered duplicates within a deduplication window (see `event_deduplication.md`).

---

## SourceRef

```python
@dataclass(frozen=True)
class SourceRef:
    source_type: str       # "docker" | "journald" | "file" | "procfs" | "app"
    source_id: str         # unique identifier of the collector instance
    raw_offset: int | None # byte offset in source, for seeking back to raw data
    collected_at: datetime # when the collector captured the raw event
```

Enables drill-down from a normalized event back to the raw source for manual investigation.

---

## Validation Rules

Every NormalizedEvent is validated before storage. An event failing validation is dropped and counted in a `validation_failures` metric.

| Field | Rule | On Failure |
|---|---|---|
| `event_id` | Valid UUID v4 format | Drop event (indicates normalization bug) |
| `timestamp` | Not None, not before 2020-01-01, not more than 60s in future | Fallback to `collected_at`, tag `clock_skew` |
| `service_id` | Non-empty, matches `[a-z0-9\-_.]+`, ≤128 chars | Map to `"unknown"` |
| `host_id` | Non-empty, ≤256 chars | Map to `"unknown"` |
| `severity` | Valid Severity enum value | Default to INFO |
| `event_type` | Valid EventType enum value | Default to LOG |
| `message` | Non-empty, ≤1024 chars | Truncate or set to `"<empty>"` |
| `structured_data` | Valid JSON-serializable dict, ≤32KB serialized | Truncate to empty dict |
| `tags` | Each tag matches `[a-z0-9_]+` | Strip invalid tags |
| `fingerprint` | 32 hex chars | Recompute from available fields |
| `schema_version` | Matches current version | Reject if future, migrate if past |

---

## Schema Versioning

The `schema_version` field enables forward-compatible evolution:

- **Version 1** (current): The schema defined in this document.
- When a breaking change is needed, `schema_version` is incremented.
- The storage layer maintains a migration registry. On startup, it checks the stored schema version and applies migrations if needed.
- Events with a schema version higher than the current code version are rejected with an error (suggests the data was created by a newer version of Inferra).

Migration strategy:
1. Add new fields with defaults (non-breaking, no version bump needed).
2. Rename fields → version bump + migration that rewrites stored data.
3. Remove fields → version bump + migration that drops columns.

---

## Event Lifecycle

```
Raw log line
    │
    ▼
RawEvent (from collector)
    │
    ├─ Parse timestamp, severity, message
    ├─ Resolve service_id, host_id
    ├─ Extract structured_data
    ├─ Apply tag rules
    ├─ Compute fingerprint
    │
    ▼
NormalizedEvent
    │
    ├─ Validate all fields
    ├─ Check deduplication window
    ├─ Apply noise filter
    │
    ▼
Stored in EventStore (if passes all checks)
    │
    ├─ Available for analysis queries
    ├─ Referenced by incidents
    │
    ▼
Pruned after retention window expires
```

---

## Event Volume Assumptions

| Scenario | Events/sec | Events/hour | Notes |
|---|---|---|---|
| Small dev setup (2-3 containers) | 1–5 | 3,600–18,000 | Typical local development |
| Medium setup (10-20 services) | 10–50 | 36,000–180,000 | Microservices on one machine |
| Large local setup (50+ services) | 50–100 | 180,000–360,000 | Near Inferra's designed limit |
| Failure storm | 100–500 burst | N/A | Backpressure buffer absorbs; sampling if sustained |

At 50 events/sec with 500-byte average event size:
- 72-hour retention: ~12.4M events, ~6.2GB raw storage
- With SQLite overhead: ~8–10GB total
