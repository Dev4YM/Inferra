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

## Host metrics

Polls CPU/memory/disk thresholds and emits metric-style events and ringbuffer snapshots.

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
`cpu_raw_percent` for the runtime's single-core-equivalent process reading.

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

Emits state-change events for service status transitions.

```toml
[collectors.windows_service]
enabled = true
poll_interval_seconds = 30.0
include_stopped = false
names = []
```

## Windows Event Log

Bookmarked channel polling through the native Rust runtime.

```toml
[collectors.windows_eventlog]
enabled = true
channels = ["Application", "System"]
poll_interval_seconds = 5.0
```

## Linux syslog files

File-follow with rotation awareness.

```toml
[collectors.linux_syslog]
enabled = true
paths = ["/var/log/syslog", "/var/log/messages"]
poll_interval_seconds = 2.0
start_at_end = true
```

## journald

Native bindings or `journalctl` fallback.

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

Long-lived file tails with optional service identity from paths.

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

Events and container logs via the Docker HTTP API.

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
