# Collector Architecture

## Purpose

Collectors are the input boundary of Inferra. Each collector is an async task that watches a specific source (Docker daemon, journald, log files, procfs) and emits `RawEvent` objects into the normalization pipeline.

Collectors are pluggable: the system ships with built-in collectors for common sources, and users can add custom collectors via configuration.

---

## Design Principles

1. **Isolation**: Each collector runs independently. A crash in one collector does not affect others.
2. **Backpressure**: If the normalization pipeline cannot keep up, collectors slow down rather than dropping events silently.
3. **Resumability**: File-based collectors track their read position and resume from where they left off after restart.
4. **Low overhead**: Collectors use non-blocking I/O and avoid polling where possible (prefer inotify/docker events API).

---

## Collector Interface

```python
class Collector(Protocol):
    """Base protocol for all collectors."""

    @property
    def collector_id(self) -> str:
        """Unique identifier for this collector instance."""
        ...

    @property
    def source_type(self) -> str:
        """One of: 'docker', 'journald', 'file', 'procfs', 'app'."""
        ...

    async def start(self, sink: asyncio.Queue[RawEvent]) -> None:
        """Begin collecting. Push RawEvents into sink. Runs until cancelled."""
        ...

    async def stop(self) -> None:
        """Graceful shutdown. Drain in-flight events, persist checkpoint."""
        ...

    def health_check(self) -> CollectorHealth:
        """Return current health status."""
        ...

@dataclass
class CollectorHealth:
    collector_id: str
    is_running: bool
    events_emitted: int        # total since start
    events_per_second: float   # last 60 seconds
    last_event_at: datetime | None
    error_count: int           # errors since start
    last_error: str | None
    lag_seconds: float | None  # how far behind real-time (for file collectors)
```

---

## Built-In Collectors

### 1. Docker Collector

**Source**: Docker Engine API via Unix socket (`/var/run/docker.sock`) or TCP.

**Mechanism**:
- Uses Docker's `/containers/{id}/logs` endpoint with `follow=true` and `timestamps=true`
- Maintains one log stream per running container
- Listens to Docker events API (`/events`) for container start/stop/die to dynamically add/remove log streams

**Event metadata**:
```python
metadata = {
    "container_id": "abc123def456",
    "container_name": "api-gateway",
    "image": "myapp/api:v2.1",
    "labels": {"com.docker.compose.service": "api"},
}
```

**Container discovery**:
1. On startup: list all running containers, start a log stream for each
2. On `start` event: add new log stream
3. On `stop`/`die` event: emit a STATE_CHANGE event, close log stream
4. On `oom` event: emit a CRITICAL event with `oom` tag

**Configuration**:
```toml
[collectors.docker]
enabled = true
socket = "/var/run/docker.sock"
# Filter containers (optional). If empty, all containers are watched.
include_labels = ["inferra.watch=true"]
exclude_names = ["inferra-*"]  # don't watch ourselves
include_all = true  # if true, ignore include_labels and watch everything
```

**Failure handling**:
- Docker socket unavailable: retry with exponential backoff (1s → 60s max)
- Log stream disconnected: reconnect from last known timestamp
- Container logs too fast: backpressure from the sink queue naturally throttles

---

### 2. Journald Collector

**Source**: systemd journal via `systemd.journal.Reader` (Python `systemd` package).

**Mechanism**:
- Opens the journal with `SD_JOURNAL_LOCAL_ONLY` flag
- Seeks to the current tail (or checkpoint position on restart)
- Polls for new entries with configurable timeout

**Event metadata**:
```python
metadata = {
    "unit": "nginx.service",
    "priority": 3,
    "syslog_identifier": "nginx",
    "pid": 1234,
    "uid": 0,
}
```

**Configuration**:
```toml
[collectors.journald]
enabled = true
units = []           # empty = all units
exclude_units = ["systemd-resolved.service", "systemd-timesyncd.service"]
min_priority = 6     # 0=emerg ... 7=debug. 6 = include info and above
```

**Checkpoint**: Stores journal cursor in `./data/checkpoints/journald.cursor`. On restart, seeks to this cursor.

---

### 3. File Collector

**Source**: Log files on the filesystem, watched via `inotify` (Linux) or polling (fallback).

**Mechanism**:
- Watches configured file paths or glob patterns
- Tracks file position (byte offset) per inode
- Handles log rotation: detects inode change or file truncation, resets position

**Event metadata**:
```python
metadata = {
    "path": "/var/log/nginx/access.log",
    "line_number": 4523,
    "inode": 12345678,
}
```

**Log rotation handling**:
1. File truncated (size decreased): reset to beginning
2. File replaced (inode changed): close old, open new, start from beginning
3. File renamed (common for logrotate): continue reading old fd until EOF, then switch to new file at path

**Configuration**:
```toml
[[collectors.file]]
path = "/var/log/nginx/error.log"
service_id = "nginx"  # explicit service mapping

[[collectors.file]]
glob = "/var/log/myapp/*.log"
service_id_from_filename = true  # derive service_id from filename

[[collectors.file]]
path = "/opt/app/logs/combined.log"
multiline_pattern = '^\d{4}-\d{2}-\d{2}'  # new entry starts with date
```

**Checkpoint**: Stores `{inode: offset}` mapping in `./data/checkpoints/file_positions.json`.

**Multiline handling**: Some log entries span multiple lines (stack traces, JSON blobs). The file collector supports multiline aggregation via a `multiline_pattern` that identifies the start of a new entry. Lines not matching the pattern are appended to the previous entry.

---

### 4. Procfs/Sys Collector

**Source**: `/proc` and `/sys` filesystems (Linux).

**Mechanism**: Periodic polling (not event-driven; procfs has no watch mechanism).

**Collected metrics**:

| Metric | Source | Frequency |
|---|---|---|
| CPU usage (per-process) | `/proc/{pid}/stat` | 10s |
| Memory usage (per-process) | `/proc/{pid}/status` | 10s |
| System load average | `/proc/loadavg` | 10s |
| Disk usage | `statvfs()` on configured paths | 60s |
| Open file descriptors | `/proc/{pid}/fd` count | 30s |
| Network connections | `/proc/net/tcp` | 30s |
| OOM killer activity | `/proc/vmstat` (oom_kill counter) | 10s |

**Process discovery**: Watches processes matching configured patterns:
```toml
[collectors.procfs]
enabled = true
poll_interval_seconds = 10
watch_processes = ["nginx", "postgres", "python", "node", "java"]
watch_pids = []  # explicit PIDs, optional
disk_paths = ["/", "/var/log", "/data"]
```

**Event generation**: Metric readings become events of type `METRIC`. Threshold crossings generate `WARN` or `ERROR` severity events:
- CPU > 90% sustained for 3 readings → ERROR
- Memory > 85% → WARN, >95% → ERROR
- Disk > 90% → WARN, >95% → ERROR
- Load average > 2× CPU count → WARN

---

### 5. Application Collector

**Source**: Structured events sent directly from applications via a local socket or HTTP endpoint.

**Mechanism**: Runs a lightweight HTTP server on `localhost:{port}` accepting POST requests with JSON event payloads.

```
POST /events
Content-Type: application/json

{
    "service": "payment-service",
    "level": "error",
    "message": "Payment processing failed",
    "context": {"order_id": "12345", "provider": "stripe"}
}
```

**Configuration**:
```toml
[collectors.app]
enabled = false  # opt-in
listen = "127.0.0.1:9876"
max_payload_bytes = 65536
```

---

## Backpressure and Buffering

### Architecture

```
Collector ──▶ Per-Collector Buffer ──▶ Central Sink Queue ──▶ Normalization Pipeline
              (bounded ring)           (bounded queue)
```

### Per-Collector Buffer
Each collector has a ring buffer of 1000 events. If the central sink queue is full:
1. Events accumulate in the ring buffer
2. If the ring buffer fills, oldest events are dropped (with a counter incremented)
3. A `collector_backpressure` warning event is emitted when drops occur

### Central Sink Queue
- Capacity: 10,000 events (configurable)
- If full: collectors block on `queue.put()` (async, so they yield to the event loop but don't process new source events)
- Monitoring: queue depth is exposed as a metric and shown in the UI health panel

### Throughput Limits

| Level | Events/sec | Behavior |
|---|---|---|
| Normal | 1–50 | No backpressure |
| Elevated | 50–100 | Queue depth increases, within capacity |
| High | 100–500 | Per-collector buffers absorb burst |
| Overload | >500 sustained | Drops begin, warning emitted, sampling activated |

### Sampling Under Overload

When sustained throughput exceeds the configured limit (default: 200 events/sec for >30 seconds):
1. Calculate sample rate: `target_rate / actual_rate`
2. Apply reservoir sampling: keep all ERROR/CRITICAL events, sample INFO/DEBUG
3. Tag sampled events with `_sampled: true` and `_sample_rate: N`
4. Log a warning: `"Event sampling activated: keeping 1 in {N} INFO/DEBUG events"`

---

## Collector Lifecycle

```
                  ┌──────────┐
                  │ DISABLED  │  (not in config or enabled=false)
                  └─────┬────┘
                        │ config enabled
                        ▼
                  ┌──────────┐
           ┌──────│ STARTING │
           │      └─────┬────┘
           │            │ source accessible
           │            ▼
           │      ┌──────────┐
           │      │ RUNNING  │◀──────────────┐
           │      └─────┬────┘               │
           │            │                    │
           │    source error              recovered
           │            │                    │
           │            ▼                    │
           │      ┌──────────┐               │
           │      │ RETRYING │───────────────┘
           │      └─────┬────┘
           │            │ max retries exceeded
           │            ▼
           │      ┌──────────┐
           └──────│  FAILED  │
                  └─────┬────┘
                        │ shutdown signal
                        ▼
                  ┌──────────┐
                  │ STOPPED  │
                  └──────────┘
```

**Retry policy**: Exponential backoff starting at 1 second, doubling each attempt, capped at 60 seconds. After 10 consecutive failures without any successful event emission, the collector enters FAILED state and stops retrying. Manual restart via UI or config reload required.

---

## Collector Registration and Discovery

On startup, Inferra reads `inferra.toml` and instantiates collectors:

```python
async def start_collectors(config: InferraConfig, sink: asyncio.Queue) -> list[Collector]:
    collectors = []
    if config.collectors.docker.enabled:
        collectors.append(DockerCollector(config.collectors.docker, sink))
    if config.collectors.journald.enabled:
        collectors.append(JournaldCollector(config.collectors.journald, sink))
    for file_config in config.collectors.file:
        collectors.append(FileCollector(file_config, sink))
    if config.collectors.procfs.enabled:
        collectors.append(ProcfsCollector(config.collectors.procfs, sink))
    if config.collectors.app.enabled:
        collectors.append(AppCollector(config.collectors.app, sink))

    for c in collectors:
        asyncio.create_task(run_collector_with_supervision(c, sink))
    return collectors
```

The supervisor wrapper handles retry logic, health reporting, and clean shutdown.

---

## Monitoring

Each collector exposes health data via the `health_check()` method, aggregated into a system-wide collector health dashboard:

| Metric | Description |
|---|---|
| `collector_events_total` | Total events emitted, by collector_id |
| `collector_errors_total` | Errors encountered, by collector_id |
| `collector_lag_seconds` | How far behind real-time (file collectors) |
| `collector_queue_depth` | Central sink queue occupancy |
| `collector_drops_total` | Events dropped due to backpressure |
| `collector_state` | Current lifecycle state |
