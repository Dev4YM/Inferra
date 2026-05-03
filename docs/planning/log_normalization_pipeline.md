# Log Normalization Pipeline

## Purpose

Transform heterogeneous raw inputs (Docker logs, journald entries, application log files, procfs readings) into a uniform `NormalizedEvent` structure. This pipeline is the single point where raw, unstructured data becomes structured, validated, and queryable.

---

## Pipeline Stages

```
RawEvent
  │
  ▼
┌─────────────────────────────────────┐
│ Stage 1: FORMAT DETECTION           │
│ Identify the log format             │
│ (JSON, syslog, key=value, free-text)│
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│ Stage 2: PARSING                    │
│ Extract timestamp, severity,        │
│ message body, structured fields     │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│ Stage 3: IDENTITY RESOLUTION        │
│ Resolve service_id, host_id         │
│ from source metadata + content      │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│ Stage 4: ENRICHMENT                 │
│ Apply tag rules, extract semantic   │
│ signals, classify event_type        │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│ Stage 5: FINGERPRINTING             │
│ Compute deduplication fingerprint   │
└──────────────┬──────────────────────┘
               │
               ▼
┌─────────────────────────────────────┐
│ Stage 6: VALIDATION                 │
│ Enforce schema constraints          │
└──────────────┬──────────────────────┘
               │
               ▼
NormalizedEvent (or dropped with reason)
```

---

## Stage 1: Format Detection

The pipeline determines the format of `raw_payload` using a decision tree:

```
1. Is raw_payload valid JSON?
   → YES: format = "json"
   → NO: continue

2. Does raw_payload match syslog pattern?
   (RFC 3164: <priority>timestamp hostname app[pid]: message)
   (RFC 5424: <priority>version timestamp hostname app-name procid msgid structured-data msg)
   → YES: format = "syslog_3164" or "syslog_5424"
   → NO: continue

3. Does raw_payload contain 3+ key=value or key="value" pairs?
   → YES: format = "kv"
   → NO: continue

4. Does raw_payload match a known framework format?
   (Log4j: timestamp [thread] LEVEL class - message)
   (Python: timestamp - name - LEVEL - message)
   (Go: timestamp LEVEL message key=value...)
   → YES: format = "framework_{name}"
   → NO: format = "freetext"
```

Format detection is deterministic and does not depend on source_type (though source_type may provide hints that break ties).

**Performance**: Format detection must complete in <1ms. The JSON check uses a fast prefix scan (first non-whitespace character is `{` or `[`) before attempting full parse.

---

## Stage 2: Parsing

Each format has a dedicated parser.

### JSON Parser
```python
def parse_json(payload: str) -> ParseResult:
    data = json.loads(payload)
    return ParseResult(
        timestamp=extract_timestamp(data),    # checks: "timestamp", "time", "ts", "@timestamp", "date"
        severity=extract_severity(data),      # checks: "level", "severity", "loglevel", "priority"
        message=extract_message(data),        # checks: "message", "msg", "text", "log"
        structured_data=remaining_fields(data),
    )
```

### Syslog Parser
Regex-based extraction for both RFC 3164 and RFC 5424:

```python
SYSLOG_3164 = re.compile(
    r'^<(\d+)>'                          # priority
    r'(\w{3}\s+\d+\s+\d{2}:\d{2}:\d{2})' # timestamp
    r'\s+(\S+)'                          # hostname
    r'\s+(\S+?)(?:\[(\d+)\])?'           # app[pid]
    r':\s*(.*)',                          # message
    re.DOTALL
)
```

### Key-Value Parser
Extracts `key=value` and `key="quoted value"` pairs, with the remainder becoming the `message`.

### Framework Parsers
Configurable regex patterns loaded from `inferra.toml`:

```toml
[[log_formats]]
name = "python_logging"
pattern = '^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3})\s+-\s+(?P<logger>\S+)\s+-\s+(?P<level>\w+)\s+-\s+(?P<message>.*)'

[[log_formats]]
name = "go_structured"
pattern = '^(?P<timestamp>\S+)\s+(?P<level>\w+)\s+(?P<message>[^\t]+)(?P<kvpairs>.*)'
```

### Freetext Parser
No structure extracted beyond basic heuristics:
- First recognizable timestamp pattern is extracted
- Severity inferred from keyword presence
- Entire payload becomes the message

### ParseResult

```python
@dataclass
class ParseResult:
    timestamp: datetime | None
    severity: Severity | None
    message: str
    structured_data: dict
    parse_format: str          # which parser was used
    parse_confidence: float    # 0.0-1.0, how confident the parser is in the extraction
```

---

## Stage 3: Identity Resolution

Resolves `service_id` and `host_id` from multiple signals:

### service_id Resolution

Priority order:
1. **Explicit in structured_data**: `structured_data.get("service")` or `structured_data.get("app_name")`
2. **Source metadata**: Docker `container_name` → strip suffixes (`_1`, `-0`, `-replica-N`) → lowercase
3. **Journald unit**: `_SYSTEMD_UNIT` → strip `.service` suffix
4. **Config mapping**: `inferra.toml` maps file paths or patterns to service names:
   ```toml
   [[service_mappings]]
   match = "/var/log/nginx/*.log"
   service_id = "nginx"
   
   [[service_mappings]]
   match_container = "^myapp-.*"
   service_id = "myapp"
   ```
5. **Fallback**: `"unknown-{source_type}"` (e.g., `"unknown-docker"`)

### host_id Resolution

Priority order:
1. Docker: short container ID (first 12 hex chars of container_id)
2. Journald: `_HOSTNAME` field
3. Procfs: system hostname
4. File: system hostname
5. Explicit: `structured_data.get("host")` or `structured_data.get("hostname")`

---

## Stage 4: Enrichment

### Tag Derivation

Tags are derived by scanning the message and structured_data against a rule set:

```python
TAG_RULES: list[TagRule] = [
    TagRule(tag="oom",                 patterns=["out of memory", "oom", "cannot allocate", "memory exhausted"]),
    TagRule(tag="restart",             patterns=["restarting", "restart", "started container", "process started"]),
    TagRule(tag="connection_refused",  patterns=["connection refused", "econnrefused", "connect: connection refused"]),
    TagRule(tag="timeout",             patterns=["timed out", "timeout", "deadline exceeded", "context deadline"]),
    TagRule(tag="crash",               patterns=["segfault", "panic", "fatal", "core dumped", "unhandled exception"]),
    TagRule(tag="disk_full",           patterns=["no space left", "disk full", "enospc"]),
    TagRule(tag="permission_denied",   patterns=["permission denied", "access denied", "eacces", "forbidden"]),
    TagRule(tag="dns_failure",         patterns=["dns", "nxdomain", "name resolution", "getaddrinfo"]),
    TagRule(tag="certificate_error",   patterns=["certificate", "ssl", "tls", "x509"]),
    TagRule(tag="rate_limited",        patterns=["rate limit", "throttl", "too many requests", "429"]),
    TagRule(tag="config_change",       patterns=["configuration", "config reload", "settings changed"]),
    TagRule(tag="deployment",          patterns=["deploy", "release", "rolling update", "image pulled"]),
]
```

Pattern matching is case-insensitive. Multiple tags can be assigned to one event.

User-defined tag rules extend this list via `inferra.toml`:
```toml
[[tag_rules]]
tag = "payment_failure"
patterns = ["payment declined", "stripe error", "billing failed"]
```

### Event Type Classification

```python
def classify_event_type(raw_event: RawEvent, parse_result: ParseResult) -> EventType:
    if raw_event.source_type == "procfs":
        return EventType.METRIC
    if any(tag in tags for tag in ["restart", "deployment", "config_change"]):
        return EventType.STATE_CHANGE
    if "health" in parse_result.message.lower() and ("pass" in msg or "fail" in msg):
        return EventType.HEALTH_CHECK
    return EventType.LOG
```

---

## Stage 5: Fingerprinting

The fingerprint enables deduplication by abstracting away variable parts of the message:

```python
def templatize_message(message: str) -> str:
    """Replace variable tokens with placeholders to create a stable template."""
    result = message
    result = re.sub(r'\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}[\.\d]*[Z]?', '<TS>', result)
    result = re.sub(r'\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}(:\d+)?', '<IP>', result)
    result = re.sub(r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}', '<UUID>', result, flags=re.I)
    result = re.sub(r'\b[0-9a-f]{8,}\b', '<HEX>', result, flags=re.I)
    result = re.sub(r'"[^"]*"', '<STR>', result)
    result = re.sub(r"'[^']*'", '<STR>', result)
    result = re.sub(r'(/[\w\-./]+)+', '<PATH>', result)
    result = re.sub(r'\b\d+\.?\d*\b', '<NUM>', result)
    return result

def compute_fingerprint(service_id: str, message: str, severity: Severity) -> str:
    template = templatize_message(message)
    raw = f"{service_id}|{template}|{severity.value}"
    return hashlib.sha256(raw.encode()).hexdigest()[:32]
```

Fingerprint determinism is critical: same inputs always produce the same fingerprint.

---

## Stage 6: Validation

All fields are validated against the rules defined in `event_model.md`. A validation failure is one of:

- **Fatal**: event is dropped (e.g., unparseable event_id, schema version mismatch)
- **Recoverable**: field is defaulted or corrected (e.g., missing severity → INFO, timestamp too old → use collected_at)

Validation metrics tracked:
- `events_validated_total` (counter)
- `events_dropped_total` (counter, by reason)
- `events_corrected_total` (counter, by field)

---

## Error Handling

| Error | Handling | Impact |
|---|---|---|
| JSON parse failure on JSON-detected payload | Fall through to KV or freetext parser | Slightly less structure extracted |
| Regex timeout (>10ms) | Abort parse, use freetext fallback | Reduced extraction quality |
| Unknown severity string | Default to INFO, log warning | Minor misclassification possible |
| Timestamp parse failure | Use collected_at from RawEvent | Temporal accuracy may be slightly reduced |
| service_id unresolvable | Map to "unknown-{source_type}" | Service-level analysis degraded |
| Structured data exceeds 32KB | Truncate to empty dict, set `_truncated` flag | Loss of structured fields |

---

## Performance Requirements

| Metric | Target | Measurement |
|---|---|---|
| Per-event normalization latency | <5ms at p99 | Timer around full pipeline |
| Format detection | <1ms | |
| Parsing | <2ms | Includes regex execution |
| Enrichment | <1ms | Tag rule scan |
| Fingerprinting | <0.5ms | SHA-256 is fast |
| Validation | <0.5ms | Field checks |

At 100 events/second, the normalization pipeline consumes <0.5 seconds of CPU time per second, leaving headroom for analysis.

---

## Configuration

```toml
[normalization]
max_message_length = 1024
max_structured_data_bytes = 32768
timestamp_future_tolerance_seconds = 60
fingerprint_hash = "sha256"
fingerprint_length = 32

# Custom log format patterns
[[log_formats]]
name = "custom_app"
pattern = '^(?P<timestamp>\S+) \[(?P<level>\w+)\] (?P<message>.*)'
timestamp_format = "%Y-%m-%dT%H:%M:%S%.fZ"

# Custom tag rules
[[tag_rules]]
tag = "database_lock"
patterns = ["lock timeout", "deadlock detected", "lock wait"]
```
