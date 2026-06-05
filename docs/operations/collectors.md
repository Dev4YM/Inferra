# Collector Commands

Inferra collectors are read-only toward observed systems. The active collector runtime is implemented in Rust under `src/crates/inferra-collectors/`.

## Supervised collection

Collectors run inside the local runtime started by `inferra serve`. Bare `inferra` now shows a welcome/status screen; it does not start the runtime. The active collector CLI surface for operators is `collectors status`, `collectors start`, and `collectors stop`.

Windows:

```powershell
inferra --config inferra.toml config preset windows-server
inferra --config inferra.toml collectors status
inferra --config inferra.toml serve
inferra --config inferra.toml collectors start
```

Linux:

```bash
inferra --config inferra.toml config preset linux-node
inferra --config inferra.toml serve
inferra --config inferra.toml collectors status
```

Docker host:

```bash
inferra --config inferra.toml config preset docker-host
inferra --config inferra.toml serve
inferra --config inferra.toml collectors status
```

Kubernetes:

```bash
inferra --config inferra.toml config preset kubernetes
inferra --config inferra.toml serve
inferra --config inferra.toml collectors status
```

`inferra collectors status` prefers live runtime status. If no daemon is running, it falls back to configured collector rows from `inferra.toml`. `collectors start` and `collectors stop` call the local Rust API, so `inferra serve` must already be running (see [Troubleshooting](troubleshooting.md)).

## Diagnosing collector errors

The Overview health card only degrades on active collector errors. Recovered collectors can still show cumulative `error_count` history in Control, but they no longer keep system health degraded forever after a successful sample.

Collectors for optional runtimes report `unavailable` instead of `error` when the host dependency is absent. For example, a Windows machine without Docker will show the Docker collector as unavailable, emit one collector diagnostic log event, and avoid degrading overall system health.

Open **Control -> Collectors** to see `last_error`, `last_error_at`, `error_hint`, and the collector-specific `log_query`. Use **Copy collector report** to share a support-ready bundle of collector state plus recent normalized collector logs.

CLI/API equivalents:

```powershell
inferra collectors status --json
curl http://127.0.0.1:7433/api/collectors
curl "http://127.0.0.1:7433/api/logs?search=collector&limit=100"
```

## App ingest

Application ingest is part of the collector surface. It can run on the main API mount (`[collectors.app].enable_main_api = true`) and optionally on a standalone listener (`enable_standalone = true`).

---

## Meaningful collector events

Collectors emit governed `events.db` rows directly. Each event includes source identity,
severity, tags, stable fingerprints, and a `structured_data.attributes` object for fields
that can be indexed and queried. Common attributes include `service.name`, `host.name`,
`process.pid`, `host.cpu_percent`, `http.route`, and collector-specific keys such as
`windows.eventlog.event_id`, `windows.service.state`, `k8s.pod.restart_count`, and
`container.action`.

## Host metrics

Polls CPU/memory/disk thresholds and emits metric-style resource events. Events include
`resource_pressure` or `recovered` tags, host CPU/memory attributes, and the observed disk
samples so resource-pressure hypotheses can be tied back to concrete host evidence.

```toml
[collectors.host_metrics]
enabled = true
poll_interval_seconds = 10.0
warn_cpu_percent = 85.0
warn_memory_percent = 85.0
warn_disk_percent = 90.0
```

## Process snapshot

Tracks watched processes and PID lists with CPU/memory thresholds and disk paths.
Process `min_cpu_percent` is evaluated as a share of total host CPU. Events also include
`cpu_raw_percent` for the runtime's single-core-equivalent process reading. Process events
use `name:pid:create_time` identity, include command/status context, and tag threshold
entries as `resource_pressure` and recoveries as `recovered`.

```toml
[collectors.process]
enabled = true
poll_interval_seconds = 10.0
watch_processes = ["nginx", "postgres", "python", "node", "java"]
watch_pids = []
disk_paths = ["/", "/var/log", "/data"]
top_n = 20
min_cpu_percent = 75.0
min_memory_mb = 512.0
```

## Windows services

Emits state-change events for service status transitions. The Rust runtime enriches
changed services with `sc.exe qc` metadata such as display name, start type, binary path,
and service account. Automatic services that are stopped are emitted as `ERROR` even when
ordinary stopped services are hidden.

```toml
[collectors.windows_service]
enabled = true
poll_interval_seconds = 30.0
include_stopped = false
include_automatic_stopped = true
names = []
```

## Windows Event Log

Bookmarked channel polling through the native Rust runtime. Events preserve channel,
provider, event id, record id, computer name, level text, and event-data fields under
`structured_data.windows_eventlog`, with indexable `windows.eventlog.*` attributes.

```toml
[collectors.windows_eventlog]
enabled = true
channels = ["Application", "System"]
poll_interval_seconds = 5.0
```

## Linux syslog files

File-follow with rotation awareness. JSON log lines are parsed when possible to extract
timestamp, service, level, message, trace/span ids, deployment environment, and attributes;
non-JSON lines are kept as raw syslog text with path and offset attributes.

```toml
[collectors.linux_syslog]
enabled = true
paths = ["/var/log/syslog", "/var/log/messages"]
poll_interval_seconds = 2.0
start_at_end = true
```

## journald

`journalctl` JSON collection with cursor checkpoints. Events include systemd unit,
priority, process id, host, and service attributes while preserving the original journal
record.

```toml
[collectors.journald]
enabled = true
units = []
exclude_units = ["systemd-resolved.service", "systemd-timesyncd.service"]
min_priority = 6
poll_interval_seconds = 5.0
since = "-1 hour"
limit = 200
```

## File glob / multiline

Long-lived file tails with optional service identity from paths. JSON lines are parsed
using the same semantics as syslog; multiline records keep the raw body plus file path,
offset, parsed payload, and extracted trace/service attributes.

```toml
[collectors.file]
enabled = true
paths = []
poll_interval_seconds = 1.0
start_at_end = false

[[collectors.file.entries]]
path = "./logs/app.log"
glob = ""
service_id = "app"
service_id_from_filename = false
multiline_pattern = ""
```

## Docker Engine

Container lifecycle events via `docker events`. Service identity prefers Docker Compose or
application labels before the container name. OOM, kill, die, restart, and unhealthy health
status actions are tagged and severity-ranked for incident correlation.

```toml
[collectors.docker]
enabled = true
socket = "/var/run/docker.sock"
include_names = []
include_labels = []
exclude_names = ["inferra-*"]
include_all = true
```

## Kubernetes

Namespace-scoped events and workloads through the native Rust runtime and in-cluster `kubectl`/API access configured by the deployment target.
Kubernetes events derive workload identity from object names, classify warning/backoff/OOM
signals, and pod snapshots track phase, readiness, restart count, OOMKilled state, node,
namespace, and labels.

```toml
[collectors.kubernetes]
enabled = false
poll_interval_seconds = 15.0
namespaces = []
all_namespaces = true
label_selector = ""
limit = 200
include_pods = true
include_events = true
```

## Application HTTP ingest

Accepts JSON payloads over HTTP on the main app or a standalone listener.

```toml
[collectors.app]
enabled = true
listen = "127.0.0.1:9876"
max_payload_bytes = 65536
shared_token = ""
mount_path = "/api/ingest"
enable_main_api = true
enable_standalone = false
```

When `shared_token` is set, clients must send `Authorization: Bearer <token>`.

Example request:

```bash
curl -X POST http://127.0.0.1:7433/api/ingest \
  -H "Content-Type: application/json" \
  -d "{\"service\":\"api\",\"level\":\"error\",\"message\":\"timeout calling postgres\"}"
```

## Presets

Presets bundle collector toggles and `collectors.auto_start`:

```powershell
inferra config preset web-only
inferra config preset windows-server
inferra config preset linux-node
inferra config preset kubernetes
inferra config preset docker-host
```

Defaults are defined in `src/config/presets.py`.
Defaults are defined in `src/config/defaults.toml`, and the native CLI writes preset overlays with `inferra config preset <name>`.
