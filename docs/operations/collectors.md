# Collector Commands

Inferra collectors are read-only toward observed systems. Implementation lives under `src/collectors/` with factories in `src/collectors/factory.py`.

## Supervised collection

Supervisors run inside `inferra run` / `inferra serve` or standalone `inferra run-collectors`. Example sequences:

Windows:

```powershell
inferra --config inferra.toml config preset windows-server
inferra --config inferra.toml run-collectors --help
inferra --config inferra.toml collectors status
inferra --config inferra.toml collectors start --help
inferra --config inferra.toml collectors stop --help
```

Linux:

```bash
inferra --config inferra.toml config preset linux-node
inferra --config inferra.toml run-collectors --help
inferra --config inferra.toml collectors status
```

Docker host:

```bash
inferra --config inferra.toml config preset docker-host
inferra --config inferra.toml run-collectors --help
inferra --config inferra.toml collectors status
```

Kubernetes:

```bash
python -m pip install ".[kubernetes]"
inferra --config inferra.toml config preset kubernetes
inferra --config inferra.toml run-collectors --help
inferra --config inferra.toml collectors status
```

`inferra collectors status` prefers live daemon status. If no daemon is running, it reports configured collectors as `not_running` and hints to start `inferra run`. `collectors start` and `collectors stop` require a running API (see [Troubleshooting](troubleshooting.md)).

## One-shot collection

Single pass plus normalization drain:

Windows:

```powershell
inferra --config inferra.toml collect-host
inferra --config inferra.toml collect-processes
inferra --config inferra.toml collect-services
inferra --config inferra.toml collect-eventlog
```

Linux:

```bash
inferra --config inferra.toml collect-host
inferra --config inferra.toml collect-processes
inferra --config inferra.toml collect-syslog
inferra --config inferra.toml collect-journald
```

Kubernetes:

```bash
inferra --config inferra.toml collect-kubernetes
```

Use `--json` for machine-readable summaries.

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

Bookmarked channel polling (pywin32 when available).

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

Namespace-scoped events and workloads; requires Python kubernetes client when enabled.

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
