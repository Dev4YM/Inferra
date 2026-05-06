# Inferra

Inferra is a local-first runtime failure explanation system. It collects operational signals, stores them locally in SQLite, builds deterministic incident hypotheses, and optionally uses a local Ollama model to explain the evidence in plain language.

**Documentation (operator guides, install, troubleshooting):** all Markdown under [`docs/`](docs/) — browse in the repo or run a local site with MkDocs: `python -m pip install -e ".[docs]"` then `mkdocs serve` and open the URL it prints (usually [http://127.0.0.1:8000](http://127.0.0.1:8000)). A pre-built static copy may exist under `site/` after `mkdocs build`.

| What Inferra **is** | What Inferra **is not** |
| --- | --- |
| A read-only diagnostic that correlates local signals into explainable incidents | An auto-remediation or remote-control tool |
| Deterministic ranking and scoring (rules + auditable state) with optional language explanations | A black-box “root cause AI” that silently changes scores |
| Python + SQLite + optional on-prem Ollama | A cloud observability suite or mandatory SaaS dependency |

Product positioning and AI boundaries are documented in [ADR 0001: Local-first guided AI](docs/adr/0001-local-first-guided-ai.md).

## Current Capabilities

- Python + SQLite local storage.
- CLI-first setup, configuration, collection, and server control.
- Optional FastAPI web dashboard.
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
inferra --config inferra.toml setup --yes --skip-connection-test
inferra --config inferra.toml init-db
inferra --config inferra.toml serve --help
inferra --config inferra.toml collectors status
```

Start the live server with `inferra --config inferra.toml serve`, then open `http://127.0.0.1:7433`.

The static console uses a vendored Tailwind bundle (`src/web/static/tailwind.css`). Regenerate it after changing Tailwind classes in `index.html` or under `src/web/static/js/` by running `bash scripts/build-ui.sh` from the repository root (Git Bash on Windows is sufficient). Optional UI browser tests: `python -m pip install -e ".[ui]"`, `python -m playwright install chromium`, then `python -m pytest tests/integration/test_ui.py`.

Service anomaly status (closed time buckets, spike/sustained/absence signals) is exposed at `GET /api/anomaly/{service}/status` with optional `window_hours` (default 24, max 168).

If your shell cannot find the installed `inferra` script, use `python -m cli` from the repository checkout:

```powershell
python -m cli --config inferra.toml serve --help
```

## Ollama AI Setup

AI is disabled by default. Prepare the config for local Gemma 4:

```powershell
inferra --config inferra.toml config set ai.enabled false
inferra --config inferra.toml config set ai.model gemma4:e4b
inferra --config inferra.toml ai models
```

When Ollama is already running locally, enable and validate it with `inferra --config inferra.toml config set ai.enabled true`, `inferra --config inferra.toml ai status`, `inferra --config inferra.toml ai pull gemma4:e4b`, and `inferra --config inferra.toml ai test`.

Remote Ollama-compatible servers are configured through `ai.base_url`, `ai.allow_remote`, and optional `ai.token_env`.

## Command Surface

- Lifecycle: `setup`, `serve`, `run`, `run-collectors`, `init-db`, `check-config`, `reason-incident <id>`, `reset-weights`, `calibration show`, `completion bash|zsh|fish|powershell`
- AI: `ai status`, `ai models`, `ai test`, `ai pull`
- Collector control: `collectors status`, `collectors start`, `collectors stop`
- One-shot collection: `collect-host`, `collect-processes`, `collect-services`, `collect-eventlog`, `collect-syslog`, `collect-journald`, `collect-kubernetes`
- Config: `config show`, `config get`, `config set`, `config preset`

```powershell
inferra --config inferra.toml check-config
inferra --json --config inferra.toml check-config
inferra --config inferra.toml config show
inferra --config inferra.toml config get ai.model
inferra --config inferra.toml config set ai.enabled false
inferra --config inferra.toml ai status
inferra --config inferra.toml ai models
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

