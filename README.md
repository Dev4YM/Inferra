# Inferra

Inferra is a local-first AI-integrated runtime intelligence control plane. It observes local systems, stores operational signals in SQLite, investigates incidents, explains evidence in plain language, and guides safe next steps without mutating the systems it watches.

**Documentation (operator guides, install, troubleshooting):** all Markdown under [`docs/`](docs/) — browse in the repo or run a local site with MkDocs: `python -m pip install -e ".[docs]"` then `mkdocs serve` and open the URL it prints (usually [http://127.0.0.1:8000](http://127.0.0.1:8000)). A pre-built static copy may exist under `site/` after `mkdocs build`.

| What Inferra **is** | What Inferra **is not** |
| --- | --- |
| A read-only observer, researcher, and investigator for local runtime signals | An auto-remediation or remote-control tool |
| Deterministic ranking and scoring (rules + auditable state) with optional language explanations | A black-box “root cause AI” that silently changes scores |
| Rust-first runtime shell + SQLite + React control plane | A cloud observability suite or mandatory SaaS dependency |

Product positioning and AI boundaries are documented in [ADR 0001: Local-first guided AI](docs/adr/0001-local-first-guided-ai.md).

## Current Capabilities

- Rust-first CLI/API/service runtime with SQLite local storage.
- Legacy Python code has been moved under `deprecated/`; the active product path is Rust-first.
- Native CLI for setup, database initialization, service management, incidents, events, services, collectors, config, workspace inspection, and AI investigation flows.
- Experience modes are reflected in configuration defaults and the web UI.
- React control plane under `src/web/frontend` with an Overview, Incidents, Systems, Evidence, AI Investigator, Workspace, Control, and Settings layout (no raw-JSON-first pages).
- Structured AI investigation contract (`/api/investigate/now|incident|service`, `/api/ai/ask`, `/api/ai/report/{incident_id}`, `/api/ai/investigate-stream`, `/api/ai/status`, `/api/ai/doctor`) with cited evidence, explicit uncertainty, a deterministic fallback when AI is disabled, persisted AI generations by scope, optional **`monitor_seconds`** on investigate/report URLs and on `ai ask`, and **SSE streaming** for token deltas plus a final JSON payload on `/api/ai/investigate-stream`.
- Native Ollama-backed investigation when `ai.enabled = true` and the configured local model is available.
- Workspace intelligence: project discovery, runtime app detection, live app resources, app-specific logs, service-to-project mapping with confidence and signals, explicit user mappings persisted into `inferra.toml`, and optional app-owned `.inferra/app.toml` manifests (`/api/workspace/*`).
- Native Rust-hosted local web dashboard/control plane.
- Native collectors across Windows/Linux/container/Kubernetes surfaces:
  - Windows Event Log with bookmark persistence and event metadata.
  - Windows service state-change snapshots with automatic-stopped detection.
  - host and process threshold crossings with indexed resource attributes.
  - syslog file ingestion with rotation tracking and JSON-line extraction.
  - journald ingestion via `journalctl` with unit/priority attributes.
  - file tailing with glob-style expansion, multiline grouping, and parsed JSON fields.
- Docker Engine lifecycle event collection with label-derived service identity.
- Kubernetes event and pod-state collection with readiness, restart, and OOM signals.
- Application HTTP ingest as a mounted API route or standalone listener.
- Native event-to-incident pipeline that writes incidents, clusters, hypotheses, and lifecycle state to SQLite as evidence arrives.
- Overview/dashboard payloads include native event-rate, severity-count, dedup, and noise-filter summaries instead of placeholder nulls.
- First-class install paths (Windows service, systemd, Docker/Compose, Helm, macOS); see **Install targets** below and [Install Guide](docs/operations/install.md).

## Quick Start

Build the Rust CLI workspace (`src/Cargo.toml`):

```powershell
cargo build --manifest-path src/Cargo.toml -p inferra-cli --release
inferra --config inferra.toml setup --yes
inferra --config inferra.toml init-db
inferra --config inferra.toml service repair
inferra --config inferra.toml service status
inferra --config inferra.toml serve
```

Start the live server with `inferra --config inferra.toml serve`, then open `http://127.0.0.1:7433`.

The web console source lives in `src/web/frontend` and builds into the packaged `src/web/ui_dist` bundle. Rebuild it with `scripts/build-web.ps1` on Windows or `bash scripts/build-web.sh` on Unix-like shells. Optional UI browser tests: `python -m pip install -e ".[ui,dev,legacy]"`, `python -m playwright install chromium`, then `python -m pytest tests/integration/test_ui.py`.

Service anomaly status (closed time buckets, spike/sustained/absence signals) is exposed at `GET /api/anomaly/{service}/status` with optional `window_hours` (default 24, max 168).

Legacy Python compatibility code lives under `deprecated/` and is no longer part of the active `src/` implementation path.

Packaged installs keep the same boundary: the Rust workspace lives under `src/` (`Cargo.toml` + `crates/`), while deprecated Python compatibility code lives under `deprecated/`.

## Ollama AI Setup

AI is disabled by default. Prepare the config for local Gemma 4:

```powershell
[ai]
enabled = true
provider = "ollama"
base_url = "http://127.0.0.1:11434"
model = "gemma4:e4b"
allow_remote = false
investigation_monitor_seconds = 5
investigation_monitor_interval_ms = 500
```

After `inferra --config inferra.toml serve` is running, validate native provider readiness from the Control page or with `curl http://127.0.0.1:7433/api/ai/status` and `curl http://127.0.0.1:7433/api/ai/doctor`.

Remote Ollama-compatible servers are configured through `ai.base_url`, `ai.allow_remote`, and optional `ai.token_env`.

**Investigation bundle timing:** before each investigation, the server can sample host CPU/memory for a wall-clock window (`ai.investigation_monitor_seconds`, default 5, and `ai.investigation_monitor_interval_ms`). Use **`monitor_seconds=0`** on `GET /api/investigate/*` and `GET /api/ai/report/*`, or **`monitor_seconds`** in the JSON body of `POST /api/ai/ask`, to skip the timed series (tests and quick checks). The CLI mirrors this with **`--monitor-seconds`** on `ai ask`, `ai investigate`, and `ai report`.

**Streaming (SSE):** live model deltas then a final structured payload:

```bash
curl -N -H "Content-Type: application/json" -H "Accept: text/event-stream" \
  -d "{\"question\":\"What should I check first?\",\"scope\":\"overview\",\"mode\":\"operator\",\"monitor_seconds\":0}" \
  http://127.0.0.1:7433/api/ai/investigate-stream
```

## Command Surface

- Lifecycle: `setup`, `serve`, `init-db`
- Incidents: `incidents list`, `incidents show <incident_id>`
- Events: `events list`, `events show <event_id>`
- Services: `services list`, `services show <service_id>`, `services events <service_id>`
- Collectors: `collectors status`, `collectors start`, `collectors stop`
- Config: `config show`, `config get <path>`, `config set <path> <value>`, `config preset <name>`
- Workspace: `workspace map`, `workspace services`, `workspace inspect <path>`, `workspace projects`
- AI: `ai status`, `ai doctor`, `ai ask "<question>" [--monitor-seconds N]`, `ai report <incident_id> [--monitor-seconds N]`, `ai investigate latest|incident <id>|service <id> [--monitor-seconds N]`
- Windows service: `service status`, `service install`, `service start`, `service stop`, `service restart`, `service remove`, `service repair`
- The local server exposes the same operator surfaces through `/api/*` for the web UI and loopback automation.

```powershell
inferra --config inferra.toml setup --yes
inferra --config inferra.toml init-db
inferra --config inferra.toml serve
inferra --config inferra.toml incidents list
inferra --config inferra.toml events list --limit 25
inferra --config inferra.toml services list
inferra --config inferra.toml collectors status
inferra --config inferra.toml config preset windows-server
inferra --config inferra.toml workspace map
inferra --config inferra.toml ai status
inferra --config inferra.toml ai investigate latest --monitor-seconds 5
inferra --config inferra.toml ai ask "What failed?" --monitor-seconds 0
inferra --config inferra.toml service status
inferra --config inferra.toml service install --startup auto
inferra --config inferra.toml service repair
```

## Collector Presets

```powershell
inferra config preset web-only
inferra config preset windows-server
inferra config preset linux-node
inferra config preset kubernetes
inferra config preset docker-host
```

Presets update collector configuration and `collectors.auto_start`.

## Install targets

- **Windows**: `deploy/windows/install-service.ps1` (Administrator) stages the runtime under `%ProgramFiles%\Inferra\`, keeps config/data/logs under `%ProgramData%\Inferra\`, and registers the `Inferra` service. Optional `-AllowFirewall`, **`-AddCliToPath`**, and **`-KillInferraProcessesBeforeInstall`** (stops service + every `inferra.exe`; opt-in) are supported. The preferred artifact is the Rust-native build via `deploy/windows/build-rust-exe.ps1`, and the native CLI now owns `inferra service install|remove|start|stop|status`. The old PyInstaller path is archived under `deprecated/windows-pyinstaller/`; see **[Windows exe build](docs/operations/windows_exe_build.md)**.
- **Linux**: `deploy/systemd/inferra.service` (`DynamicUser`, `ProtectSystem=strict`) plus `deploy/linux/fpm-package.sh` for `.deb` / `.rpm`.
- **Docker / Compose**: root `Dockerfile` and `compose.yaml` (`docker compose up --build`).
- **Kubernetes**: `helm install inferra ./deploy/helm/inferra` with optional `serviceMonitor.enabled` for Prometheus Operator.
- **macOS**: `sudo ./deploy/macos/install.sh` (LaunchDaemon) creates `/usr/local/bin/inferra` if needed and `sudo ./deploy/macos/uninstall.sh` to remove.

Release builds (tags `v*`) publish the Helm chart, Windows native executables, CycloneDX SBOM JSON, and a multi-arch GHCR image; see `docs/operations/release_signing.md`.

## Repository Layout

The active `src/` tree now contains only the Rust workspace and frontend assets:

```text
src/
  Cargo.toml       # Rust workspace root
  crates/          # inferra-cli, inferra-api, inferra-config, ...
  config/
  web/
    frontend/      # React Vite source
    ui_dist/       # built React bundle (packaged)
deprecated/
  inferra_legacy/  # archived Python CLI + windows_service reference
    cli.py
    windows_service.py
    app.py
  python_packages/ # archived Python backend/runtime modules
```

Archived Python reference code lives under `deprecated/` and is not part of the active runtime path.

## Documentation

**Architecture and planning**

- [Full build architecture plan](docs/planning/full_build_architecture_plan.md) (source-of-truth architecture)
- [Architecture overview](docs/planning/architecture_overview.md)
- [Data flow contracts](docs/planning/data_flow_contracts.md)
- [Implementation index (planning ↔ modules)](docs/planning/implementation_index.md)
- [Implementation roadmap](docs/implementation_roadmap.md)

**Architecture Decision Records**

- [ADR 0001 — Local-first guided AI](docs/adr/0001-local-first-guided-ai.md)
- [ADR 0002 — Flat `src/` layout](docs/adr/0002-flat-src-layout.md)
- [ADR 0003 — Storage protocols](docs/adr/0003-storage-protocols.md)
- [ADR 0004 — Windows-first collectors](docs/adr/0004-windows-first-collectors.md)
- [ADR 0005 — AI presentation-only](docs/adr/0005-ai-presentation-only.md)
- [ADR 0006 — Ollama Gemma default](docs/adr/0006-ollama-gemma-default.md)

**Operator guides**

- [Install](docs/operations/install.md) (Windows desktop, Windows Server, Linux, Docker, Kubernetes, macOS)
- [AI provider](docs/operations/ai_provider.md)
- [Collectors](docs/operations/collectors.md)
- [Tuning](docs/operations/tuning.md)
- [Upgrade](docs/operations/upgrade.md)
- [Troubleshooting](docs/operations/troubleshooting.md)
- [CI](docs/operations/ci.md)
- [Release signing and SBOM](docs/operations/release_signing.md)

Built HTML docs: `python -m pip install -e ".[docs]"` then `mkdocs build` (see `mkdocs.yml`).

**Licensing:** the project is licensed under Apache-2.0; see the root `LICENSE` file (matching `license` in `src/Cargo.toml`).

## Development

The **shipping runtime and CLI** live in the Rust workspace at `src/Cargo.toml`. Build and test with Cargo (from the repo root):

```text
cargo fmt --manifest-path src/Cargo.toml --all --check
cargo clippy --manifest-path src/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path src/Cargo.toml --workspace
```

The web console source is under `src/web/frontend`; `npm run build` (or `scripts/build-web.ps1` / `scripts/build-web.sh`) writes the bundle to `src/web/ui_dist`. That directory is **build output** — gitignored, not committed. CI and install scripts build it when needed.

Python is used for **developer tooling, docs, packaging-contract tests, and Rust black-box integration harnesses**:

```text
python -m pip install -e ".[dev]"
python -m compileall tests deploy deprecated
python -m pytest -q tests/unit/test_rust_packaging_contracts.py
INFERRA_BINARY=src/target/release/inferra python -m pytest -q tests/integration/test_rust_api.py -m integration
```

Archived Python runtime tests under `deprecated/` are reference coverage only and require the **`[legacy]`** extra:

```text
python -m pip install -e ".[dev,legacy]"
python -m pytest -q -m "not chaos and not perf"
```

## Non-Goals

- No autonomous remediation.
- No hidden cloud dependency.
- No replacement for full observability platforms.
- No AI-based mutation of deterministic scores or incident evidence.
