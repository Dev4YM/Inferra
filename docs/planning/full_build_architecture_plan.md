# Full Build and Architecture Plan

Living module map: [implementation index](implementation_index.md).

## Purpose

This plan consolidates the existing Inferra planning documents into an implementation architecture for Inferra's local-first control plane.

Historically this document described a Python + SQLite monolith. The current
migration direction is a Rust-primary runtime shell and public control plane
with Python retained only for internal AI/analysis execution. The public runtime
contract in `rust_runtime_contracts.md` takes precedence over older
Python-first assumptions in this document.

Inferra remains a local-first, read-only runtime debugging assistant. It observes logs, containers, services, process metrics, and runtime context, then builds deterministic, evidence-backed hypotheses about failures. The LLM layer is optional and presentation-only.

## Product Position

Inferra is not a cloud observability platform and not an auto-remediation system. It is a local diagnostic system for developers, operators, and small teams who need fast failure explanation without sending operational data to a cloud service.

The core promise:

1. Ingest runtime events from local sources.
2. Normalize them into a stable event model.
3. Store them locally in SQLite.
4. Correlate events into incidents.
5. Build inference graphs and hypotheses deterministically.
6. Rank hypotheses using auditable scoring.
7. Present the result through a local web UI.

## Required Stack

### Core

- Rust workspace for the runtime shell, public HTTP API, Windows service shell,
  storage access, collector supervision, and operator-facing CLI
- SQLite in WAL mode
- Axum for the local public REST server and static UI host
- Tokio for async runtime and task orchestration on the Rust side
- Serde for typed contracts
- NetworkX for local graph operations
- psutil for cross-platform host/process metrics
- pytest for tests

### Optional Integrations

- Docker SDK or Docker Engine HTTP API for Docker collectors.
- pywin32 for Windows Event Log and Windows Service support.
- systemd journal bindings for Linux journald support.
- Kubernetes Python client for Kubernetes discovery and log collection.
- Ollama or OpenAI-compatible provider for optional explanations.

### Packaging

- Cargo-driven native binaries as the primary runtime artifact.
- Windows native service packaging and wrapper for Windows Server.
- Archived PyInstaller path retained only as historical reference.
- systemd unit for Linux.
- Docker image for Linux container runtime.
- Helm chart and Kubernetes manifests for cluster deployment.

## Architectural Principle Changes

The current planning set is mostly Linux-first. For the requested target platforms, the implementation should use a platform adapter architecture:

```text
Inferra Core
  config
  event model
  normalization
  storage
  analysis
  reasoning
  presentation

Platform Adapters
  windows
  linux
  macos
  kubernetes

Collectors
  file
  docker
  app_http
  host_metrics
  windows_event_log
  journald
  kubernetes_logs
```

The reasoning core must not depend directly on Windows APIs, Linux procfs, journald, Docker, or Kubernetes. All platform-specific behavior enters through collectors and runtime context providers.

## Target Architecture

```text
Sources
  Windows Event Log
  Windows services/processes
  Linux journald/syslog
  Docker logs/events
  Kubernetes pod logs/events
  application HTTP events
  configured log files
        |
        v
Collection Layer
  async collectors
  bounded queues
  backpressure
  checkpoints
        |
        v
Normalization Layer
  format detection
  parsing
  identity resolution
  enrichment
  fingerprinting
  validation
  deduplication
  noise filtering
        |
        v
SQLite Storage
  events.db
  incidents.db
  config/state files
  baselines
  service graph
        |
        v
Analysis Layer
  anomaly detection
  runtime context builder
  correlation engine
  incident lifecycle
        |
        v
Reasoning Layer
  inference graph
  signal detectors
  hypothesis composer
  contradiction handler
  validator
  scoring
  calibration
        |
        v
Presentation Layer
  local FastAPI server
  static web UI
  REST API
  WebSocket updates
  optional LLM explanation
```

## Repository Structure

Recommended implementation layout:

```text
pyproject.toml
README.md
Dockerfile
compose.yaml
src/
  Cargo.toml
  crates/
  ai/
    worker/
  analysis/
  collectors/
  config/
  core/
  events/
  explanation/
  normalization/
  reasoning/
  runtime/
  storage/
  web/
    static/
deploy/
  windows/
  systemd/
  macos/
  helm/
docs/
  adr/
  operations/
  planning/
deprecated/
  inferra_legacy/
  windows-pyinstaller/
tests/
  unit/
  integration/
```

The active implementation now keeps the Rust workspace and frontend directly
under `src/`. Do not reintroduce a parallel Python application tree there.

Ownership rules:

- `src/Cargo.toml` + `src/crates/` own the public runtime, operator CLI, HTTP API, Windows service
  shell, hot-path storage/runtime logic, and packaging entrypoints.
- `src/web/frontend/` and `src/web/ui_dist/` own the shipped UI source and bundle.
- `deprecated/` holds superseded Python/runtime/build paths and should not be treated as the active implementation.

## Core Data Model

The existing `RawEvent`, `NormalizedEvent`, `EventCluster`, `Incident`, `ScoredHypothesisSet`, and `ExplanationResult` contracts should become actual Python models early in the build.

Rules:

- `NormalizedEvent` is immutable after storage.
- Every persisted object has `schema_version`.
- Every reference to an `event_id` must point to an existing event at the time it is written.
- Core reasoning receives only normalized, validated structures.
- LLM output never feeds back into analysis, scoring, or ranking.

## Storage Architecture

Use SQLite as the only required database.

### Databases

- `events.db`: append-heavy normalized events.
- `incidents.db`: incidents, clusters, hypotheses, explanations, feedback.
- JSON files for baselines, scoring weights, service graph, calibration, and checkpoints where write frequency is low.

### SQLite Rules

- Enable WAL mode.
- Use schema migrations from day one.
- Use repository interfaces instead of direct SQL from business logic.
- Batch event writes.
- Add retention pruning.
- Run integrity checks on startup.
- Keep query methods lazy and paginated.

### Future Portability

Even though SQLite is the required database, storage should be accessed through protocols:

- `EventStore`
- `IncidentStore`
- `ConfigStore`
- `ServiceGraphStore`

This preserves a future path to PostgreSQL without changing the reasoning engine.

## Platform Support Plan

### Windows and Windows Server

This is the main support target.

Collectors:

- File collector with polling and Windows file rotation handling.
- Windows Event Log collector using pywin32.
- Host metrics collector using psutil.
- Docker Desktop or Windows Docker Engine collector where available.
- Application HTTP collector on localhost.

Runtime context:

- CPU, memory, disk, process list, service state via psutil and Windows APIs.
- Windows service status where available.
- Docker Desktop container context where available.

Packaging:

- Single executable or installed package.
- Windows Service mode for Windows Server.
- Local web UI bound to `127.0.0.1` by default.
- Data directory under `%ProgramData%\Inferra` for service installs and user-local app data for desktop installs.

Constraints:

- No journald.
- No procfs.
- Some Docker and container metrics are limited compared to Linux.

### Linux

Collectors:

- File collector using inotify when available.
- journald collector.
- procfs/sys collector.
- Docker collector through Unix socket.
- Application HTTP collector.

Runtime context:

- Full host context via procfs, sysfs, psutil fallback.
- Full Docker context when Docker is available.

Packaging:

- Python package.
- Standalone binary.
- systemd service.
- Docker image.

### Kubernetes on Linux

SQLite means the first Kubernetes version should be node-local, not a fully distributed cluster brain.

Recommended deployment modes:

1. Single-node Kubernetes:
   - Inferra runs as one Deployment or DaemonSet pod.
   - SQLite is stored on a persistent volume.
   - Correlates pod logs and events visible from that node or namespace.

2. Multi-node Kubernetes, v1 practical mode:
   - DaemonSet, one Inferra per node.
   - Each node has its own SQLite database.
   - No cross-node causal correlation.
   - Optional UI per node or port-forward access.

3. Future multi-node mode:
   - Requires event relay, central storage, or a coordinator.
   - This exceeds pure SQLite local-first architecture unless the central process owns a single SQLite database and agents send events to it.

Kubernetes collectors:

- Pod log collector.
- Kubernetes event collector.
- Container restart/OOM collector.
- Namespace and label filters.

Kubernetes packaging:

- Docker image.
- Helm chart.
- RBAC with least privilege.
- hostPath or PVC for SQLite data.

### macOS

Collectors:

- File collector.
- Docker Desktop collector.
- Host metrics via psutil.
- Application HTTP collector.

Runtime context:

- Partial host metrics.
- Docker Desktop where available.

Packaging:

- Python package or standalone binary.
- launchd service optional.

## Build Phases

### Phase 0: Project Foundation

Deliverables:

- `pyproject.toml`
- package skeleton
- CLI entrypoint
- config loader
- typed model definitions
- logging setup
- migration framework
- test harness

Acceptance:

- `inferra --version` works.
- `inferra check-config` validates config.
- SQLite databases can initialize and migrate.

### Phase 1: Event Pipeline MVP

Deliverables:

- `RawEvent` and `NormalizedEvent`
- file collector
- application HTTP collector
- normalization pipeline
- fingerprinting
- validation
- SQLite event store
- basic REST event query endpoint

Acceptance:

- Feed sample log files.
- Events appear in `events.db`.
- Query by time range, service, severity.
- Deterministic fingerprint tests pass.

### Phase 2: Windows-First Collection

Deliverables:

- Windows Event Log collector.
- psutil host metrics collector.
- Windows service/process runtime context.
- Windows file collector checkpointing.
- Windows service packaging prototype.

Acceptance:

- Runs on Windows desktop and Windows Server.
- Collects Application/System event logs.
- Captures CPU, memory, disk, process metrics.
- Survives restart with checkpoints.

### Phase 3: Deduplication, Noise, and Baselines

Deliverables:

- dedup tracker
- noise filter
- metric ringbuffer
- anomaly baseline store
- cold start UI/API status

Acceptance:

- Duplicate storms are compressed.
- ERROR/CRITICAL events are preserved.
- Baselines enter learning mode and persist.
- Anomaly score API returns deterministic output for fixtures.

### Phase 4: Correlation and Incidents

Deliverables:

- service graph cache
- topology config
- correlation strategies
- event clustering
- incident creation and merge logic
- incident store
- active incident REST endpoints

Acceptance:

- Synthetic multi-service failure creates one incident.
- No topology means conservative same-service correlation only.
- Configured topology enables cascade clustering.

### Phase 5: Reasoning Engine

Deliverables:

- inference graph engine
- signal detectors
- hypothesis composer
- hypothesis validator
- contradiction handler
- scoring engine
- calibration model

Acceptance:

- Determinism fixtures pass.
- Top hypothesis is stable for repeated runs.
- Contradictions reduce scores and appear in API output.
- Feedback updates scoring weights within bounds.

### Phase 6: Local Web UI

Deliverables:

- Rust-hosted REST API
- WebSocket live updates
- static dashboard
- incident detail view
- services view
- timeline view
- settings view

Acceptance:

- Initial page load under 500ms locally.
- Dashboard shows status, incidents, service health, event rate.
- Incident page shows hypotheses, evidence timeline, graph, explanation, resolution controls.
- UI works without CDN or external assets.

### Phase 7: Explanation Layer

Deliverables:

- template fallback provider
- LLM provider abstraction
- Ollama provider
- OpenAI-compatible provider
- sanitization
- guardrails
- explanation cache

Acceptance:

- Template explanations always work.
- LLM is optional.
- Raw logs and secrets are not sent to remote providers.
- Guardrail violations are stored and visible.

### Phase 8: Linux, Docker, and Kubernetes

Deliverables:

- Linux journald collector.
- Linux procfs/sys metrics.
- Docker collector.
- Kubernetes collector.
- Docker image.
- systemd service.
- Helm chart.

Acceptance:

- Runs as Linux service.
- Runs in Docker.
- Runs as single-node Kubernetes deployment.
- Runs as DaemonSet in node-local mode.

### Phase 9: Packaging, Hardening, and Release

Deliverables:

- Windows installer/service package.
- Linux binaries/packages.
- macOS binary.
- Docker image and Helm chart.
- CI matrix for Windows/Linux/macOS.
- performance and determinism benchmarks.
- upgrade/migration tests.

Acceptance:

- Clean install and upgrade work.
- Crash recovery works.
- SQLite integrity check and degraded modes work.
- p99 latency budgets are met for supported workloads.

## API Plan

### REST

- `GET /api/health`
- `GET /api/events`
- `GET /api/incidents`
- `GET /api/incidents/{id}`
- `GET /api/incidents/{id}/events`
- `GET /api/incidents/{id}/hypotheses`
- `GET /api/incidents/{id}/explanation`
- `POST /api/incidents/{id}/resolve`
- `GET /api/services`
- `GET /api/services/{id}`
- `GET /api/config`
- `PUT /api/config`

### WebSocket

Server messages:

- `incident_created`
- `incident_updated`
- `incident_resolved`
- `event_count`
- `collector_health`
- `explanation_ready`
- `baseline_status`

Client messages:

- `subscribe_incident`
- `unsubscribe_incident`
- `resolve_incident`

## Configuration Plan

Primary config file: `inferra.toml`.

Config sections:

- `[server]`
- `[storage]`
- `[collectors.file]`
- `[collectors.app]`
- `[collectors.windows_eventlog]`
- `[collectors.docker]`
- `[collectors.journald]`
- `[collectors.kubernetes]`
- `[normalization]`
- `[deduplication]`
- `[noise_filter]`
- `[anomaly_detection]`
- `[correlation]`
- `[inference_graph]`
- `[hypothesis_engine]`
- `[scoring]`
- `[calibration]`
- `[explanation]`
- `[[topology.edges]]`

## Testing Architecture

Test layers:

1. Unit tests for parsers, fingerprinting, deduplication, scoring, validators.
2. Integration tests for raw event to stored event.
3. Pipeline tests for raw event to incident.
4. Determinism snapshot tests for reasoning outputs.
5. Performance tests for normalization, analysis ticks, graph construction.
6. Platform tests for Windows, Linux, macOS.
7. Packaging smoke tests.

Minimum CI matrix:

- Windows latest, Python 3.11 and 3.12
- Windows Server runner if available
- Ubuntu latest, Python 3.11 and 3.12
- macOS latest, Python 3.11 and 3.12

## Key Risks

### SQLite in Kubernetes

SQLite is excellent for local-first mode, but it is not a distributed database. Kubernetes support should be node-local unless a central coordinator is introduced.

Decision:

- Support node-local Kubernetes in v1.
- Document no cross-node correlation.
- Do not pretend DaemonSet SQLite instances are one shared system.

### Windows Collector Complexity

Windows Event Log, service state, and Docker Desktop all behave differently from Linux sources.

Decision:

- Build Windows collector early.
- Treat Windows as first-class, not a late port.
- Keep OS-specific APIs behind adapters.

### Service Graph Accuracy

Wrong topology creates wrong explanations.

Decision:

- Config-first topology.
- Auto-detected edges require confidence labels.
- Log-pattern topology inference remains opt-in.

### LLM Trust

LLM explanations can hallucinate.

Decision:

- Template fallback is always available.
- LLM never affects ranking.
- UI clearly separates structured summary from LLM explanation.

## MVP Definition

The first useful MVP should be:

- Windows-first local application.
- File collector.
- Windows Event Log collector.
- Application HTTP collector.
- SQLite storage.
- Normalization, deduplication, noise filtering.
- Basic anomaly scores.
- Correlation into incidents.
- Hypothesis generation and scoring for common failure modes.
- Local web dashboard and incident detail view.
- Template explanation fallback.

Docker, Linux, Kubernetes, macOS, and LLM providers can follow after the MVP foundation is stable.

## Suggested Implementation Order

1. Define typed contracts and storage migrations.
2. Build event pipeline and event store.
3. Build Windows collectors and psutil runtime metrics.
4. Add deduplication, noise filtering, and baselines.
5. Add correlation and incident lifecycle.
6. Add inference graph, signal detectors, hypotheses, validation, and scoring.
7. Add local web UI.
8. Add template explanations.
9. Add Linux/Docker collectors.
10. Add Kubernetes deployment.
11. Add macOS support.
12. Add optional LLM providers.
13. Harden packaging and release.

## Final Architecture Decision

Use a single-process modular monolith for v1.

This is the best fit for Python + SQLite and local-first deployment. It keeps installation simple on Windows, keeps Linux and macOS support practical, and keeps Kubernetes possible in a node-local mode. The architecture should still enforce strong module boundaries through typed contracts and repository interfaces so that future multi-process or central-server modes are possible without rewriting the reasoning core.
