# Inferra

Inferra is a local-first AI-integrated runtime intelligence control plane. It observes local systems, stores operational signals in SQLite, investigates incidents, explains evidence in plain language, and guides safe next steps without mutating the systems it watches.

**Documentation (operator guides, install, troubleshooting):** all Markdown under [`docs/`](docs/) — browse in the repo or run a local site with MkDocs: `python -m pip install -e ".[docs]"` then `mkdocs serve` and open the URL it prints (usually [http://127.0.0.1:8000](http://127.0.0.1:8000)). A pre-built static copy may exist under `site/` after `mkdocs build`.

| What Inferra **is** | What Inferra **is not** |
| --- | --- |
| A read-only observer, researcher, and investigator for local runtime signals | An auto-remediation or remote-control tool |
| Deterministic ranking and scoring (rules + auditable state) with optional language explanations | A black-box “root cause AI” that silently changes scores |
| Python + SQLite + optional on-prem Ollama | A cloud observability suite or mandatory SaaS dependency |

Product positioning and AI boundaries are documented in [ADR 0001: Local-first guided AI](docs/adr/0001-local-first-guided-ai.md).

## Current Capabilities

- Python + SQLite local storage.
- CLI-first onboarding, mode selection, configuration, collection, and server control.
- Two experience modes: operator (default) and developer (raw detail, diagnostics, and workspace-linked debugging), toggleable from CLI (`inferra mode set ...`) and from the web UI.
- React control plane under `src/web/frontend` with an Overview, Incidents, Systems, Evidence, AI Investigator, Workspace, Control, and Settings layout (no raw-JSON-first pages).
- Structured AI investigation contract (`/api/investigate/now|incident|service`, `inferra ai investigate|ask|report|trace|doctor`) with cited evidence, explicit uncertainty, and a deterministic fallback when AI is disabled.
- Workspace intelligence: project discovery, service-to-project mapping with confidence and signals, and explicit user mappings persisted into `inferra.toml` (`/api/workspace/*`, `inferra workspace map|services|inspect`).
- Optional FastAPI web dashboard/control plane.
- Optional Ollama AI explanations, incident chat (persisted in SQLite), operator-visible sanitized prompts on the AI Trace tab, and natural-language event search (`GET /api/search/natural?q=...` when AI is enabled).
- Gemma 3 and Gemma 4 model registry for Ollama tags.
- Windows-first collectors:
  - Windows Event Log with bookmark persistence.
  - Windows service state-change snapshots.
  - host and process threshold crossings with metric ringbuffers.
- Linux collectors:
  - syslog file ingestion with rotation tracking.
  - journald ingestion through native bindings or `journalctl`.
- Docker Engine event and container log collection.
- Kubernetes event and pod-state collection with restart and OOM detection.
- Application HTTP ingest as a mounted API route or standalone listener.
- Deterministic reasoning:
  - correlation clusters.
  - anomaly scoring.
  - topology-aware root-cause selection.
  - six-component weighted scoring with bounded weight learning from operator feedback (`POST /api/incidents/{id}/feedback`).
  - contradiction handling.
  - confidence calibration (`inferra calibration show`, `inferra reset-weights`).
- First-class install paths (Windows service, systemd, Docker/Compose, Helm, macOS); see **Install targets** below and [Install Guide](docs/operations/install.md).

## Quick Start

Install the project in editable mode with `python -m pip install -e ".[dev]"`.

```powershell
inferra --config inferra.toml onboard --yes --mode operator --preset windows-server --model gemma4:e4b --skip-connection-test
inferra --config inferra.toml guide
inferra --config inferra.toml init-db
inferra --config inferra.toml mode show
inferra --config inferra.toml serve --help
inferra --config inferra.toml dashboard --no-open
inferra --config inferra.toml investigate latest
inferra --config inferra.toml collectors status
```

Start the live server with `inferra --config inferra.toml serve`, then open `http://127.0.0.1:7433`.

The web console source lives in `src/web/frontend` and builds into the packaged `src/web/ui_dist` bundle. Rebuild it with `scripts/build-web.ps1` on Windows or `bash scripts/build-web.sh` on Unix-like shells. Optional UI browser tests: `python -m pip install -e ".[ui]"`, `python -m playwright install chromium`, then `python -m pytest tests/integration/test_ui.py`.

Service anomaly status (closed time buckets, spike/sustained/absence signals) is exposed at `GET /api/anomaly/{service}/status` with optional `window_hours` (default 24, max 168).

If your shell cannot find the installed `inferra` script, use `python -m cli` from the repository checkout:

```powershell
python -m cli --config inferra.toml serve --help
```

## Ollama AI Setup

AI is disabled by default. Prepare the config for local Gemma 4:

```powershell
inferra --config inferra.toml ai setup --disable
inferra --config inferra.toml ai setup --enable --model gemma4:e4b
inferra --config inferra.toml ai models
```

When Ollama is already running locally, enable and validate it with `inferra --config inferra.toml config set ai.enabled true`, `inferra --config inferra.toml ai status`, `inferra --config inferra.toml ai pull gemma4:e4b`, and `inferra --config inferra.toml ai test`.

Remote Ollama-compatible servers are configured through `ai.base_url`, `ai.allow_remote`, and optional `ai.token_env`.

## Command Surface

- Lifecycle: `onboard`, `setup`, `guide`, `dashboard`, `serve`, `run`, `run-collectors`, `init-db`, `check-config`, `reason-incident <id>`, `reset-weights`, `calibration show`, `completion bash|zsh|fish|powershell`
- AI: `ai setup`, `ai status`, `ai models`, `ai test`, `ai pull`, `ai ask "<question>"`, `ai investigate latest|incident <id>|service <id>`, `ai report <incident_id>`, `ai trace <incident_id>`, `ai doctor`
- Windows service: `service status`, `service install`, `service start`, `service stop`, `service restart`, `service remove`, `service repair`
- Investigation: `investigate now`, `investigate latest`, `investigate incident <id>`, `investigate service <service>`, `doctor`, `doctor --release`
- Runtime inspection: `incidents list|show`, `events list|show`, `services list|show|events`, `overview`, `status`, `workspace`
- Workspace: `workspace`, `workspace scan`, `workspace map`, `workspace services`, `workspace inspect <path>`
- Demo data: `demo seed [--service <id> --count <n>]`, `demo clear`
- Collector control: `collectors status`, `collectors start`, `collectors stop`
- One-shot collection: `collect-host`, `collect-processes`, `collect-services`, `collect-eventlog`, `collect-syslog`, `collect-journald`, `collect-kubernetes`
- Config: `config show`, `config get`, `config set`, `config preset`
- Experience: `mode show`, `mode set operator`, `mode set developer`

```powershell
inferra --config inferra.toml check-config
inferra --json --config inferra.toml check-config
inferra --config inferra.toml guide --profile operator
inferra --config inferra.toml guide --profile developer
inferra --config inferra.toml guide --profile server
inferra --config inferra.toml dashboard --section workspace
inferra --config inferra.toml config show
inferra --config inferra.toml config get ai.model
inferra --config inferra.toml config set ai.enabled false
inferra --config inferra.toml mode set developer
inferra --config inferra.toml doctor
inferra --config inferra.toml doctor --release
inferra --config inferra.toml investigate latest
inferra --config inferra.toml incidents list
inferra --config inferra.toml events list --limit 25
inferra --config inferra.toml services list
inferra --config inferra.toml ai setup --enable --model gemma4:e4b
inferra --config inferra.toml ai status
inferra --config inferra.toml ai models
inferra --config inferra.toml service status
inferra --config inferra.toml service install --startup auto
inferra --config inferra.toml collect-host
inferra --config inferra.toml collect-processes
inferra --config inferra.toml reset-weights
inferra --config inferra.toml calibration show
inferra --config inferra.toml completion powershell
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

- **Windows**: `deploy/windows/install-service.ps1` (Administrator) registers the `Inferra` service under `%ProgramData%\Inferra\`; optional `-AllowFirewall`, **`-AddCliToPath`**, and **`-KillInferraProcessesBeforeInstall`** (stops service + every `inferra.exe`; opt-in). PyInstaller: see **[Windows exe build](docs/operations/windows_exe_build.md)** — run **`deploy/windows/build-exe.ps1`** (staged `dist` + retries + smoke test). Optional **`deploy/windows/prepare-build-venv.ps1`** for an isolated build environment.
- **Linux**: `deploy/systemd/inferra.service` (`DynamicUser`, `ProtectSystem=strict`) plus `deploy/linux/fpm-package.sh` for `.deb` / `.rpm`.
- **Docker / Compose**: root `Dockerfile` and `compose.yaml` (`docker compose up --build`).
- **Kubernetes**: `helm install inferra ./deploy/helm/inferra` with optional `serviceMonitor.enabled` for Prometheus Operator.
- **macOS**: `sudo ./deploy/macos/install.sh` (LaunchDaemon) creates `/usr/local/bin/inferra` if needed and `sudo ./deploy/macos/uninstall.sh` to remove.

Release builds (tags `v*`) publish wheels, sdist, Helm chart, Windows `inferra.exe`, SBOM JSON, and a multi-arch GHCR image; see `docs/operations/release_signing.md`.

## Repository Layout

The project intentionally uses a flat `src/` layout with top-level packages:

```text
src/
  ai/
  analysis/
  cli_core/         # CommandResult/Error and HTTP client helpers
  collectors/
  config/
  core/
  events/
  explanation/
  normalization/
  reasoning/
  runtime/          # workspace_scan, workspace_map, runtime context
  storage/
  web/
    api.py          # FastAPI factory + /ws websocket
    _shared.py      # serialization helpers shared by routers
    frontend/       # React Vite source
    routers/        # ai, collectors, events, incidents, services,
                    # topology, investigate, workspace
    routes/system.py
    ui_dist/        # built React bundle (packaged)
  app.py
  cli.py
  windows_service.py
```

There is no nested `inferra/` package directory.

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

## Development

Developer setup uses `python -m pip install -e ".[dev]"`.

```text
python -m compileall src tests
python -m pytest -q
```

## Non-Goals

- No autonomous remediation.
- No hidden cloud dependency.
- No replacement for full observability platforms.
- No AI-based mutation of deterministic scores or incident evidence.
