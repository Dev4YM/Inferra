# Inferra

Inferra is a local-first runtime failure explanation system. It collects operational signals, stores them locally in SQLite, builds deterministic incident hypotheses, and optionally uses a local Ollama model to explain the evidence in plain language.

Inferra does not remediate systems, execute fixes, or require a cloud service.

## Current Capabilities

- Python + SQLite local storage.
- CLI-first setup, configuration, collection, and server control.
- Optional FastAPI web dashboard.
- Optional Ollama AI explanations and incident chat.
- Gemma 4 model registry for Ollama tags.
- Windows-first collectors:
  - Windows Event Log with bookmark persistence.
  - Windows service snapshots.
  - host and process metrics.
- Linux collectors:
  - syslog file ingestion.
  - journald JSON ingestion through `journalctl`.
- Kubernetes event and pod-state collection.
- Deterministic reasoning:
  - correlation clusters.
  - anomaly scoring.
  - topology-aware root-cause selection.
  - contradiction handling.
  - confidence calibration.
- Deployment assets for Windows, Linux systemd, Docker, Helm, and macOS launchd.

## Quick Start

```powershell
python -m pip install -e ".[dev]"
inferra --config inferra.toml setup --yes --skip-connection-test
inferra --config inferra.toml serve
```

Open `http://127.0.0.1:7433`.

If your shell cannot find the installed `inferra` script, use `python -m cli` from the repository checkout:

```powershell
python -m cli --config inferra.toml serve
```

## Ollama AI Setup

AI is disabled by default. To use local Gemma 4 through Ollama:

```powershell
ollama pull gemma4:e4b
inferra --config inferra.toml config set ai.enabled true
inferra --config inferra.toml config set ai.model gemma4:e4b
inferra --config inferra.toml ai test
```

Remote Ollama-compatible servers are configured through `ai.base_url` and optional `ai.token_env`.

## Useful CLI Commands

```powershell
inferra check-config
inferra init-db
inferra ai status
inferra ai models
inferra collectors status
inferra run-collectors
inferra collect-host
inferra collect-processes
inferra collect-services --include-stopped
inferra collect-eventlog --channel Application
```

Linux:

```bash
inferra collect-syslog --path /var/log/syslog
inferra collect-journald --unit nginx.service --since "-1 hour"
```

Kubernetes:

```bash
python -m pip install -e ".[kubernetes]"
inferra collect-kubernetes --namespace default
```

## Collector Presets

```powershell
inferra config preset web-only
inferra config preset windows-server
inferra config preset linux-node
inferra config preset kubernetes
```

Presets update collector configuration and `collectors.auto_start`.

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

- [Install Guide](docs/operations/install.md)
- [AI Provider Setup](docs/operations/ai_provider.md)
- [Collector Commands](docs/operations/collectors.md)
- [Implementation Roadmap](docs/implementation_roadmap.md)
- [Architecture Planning](docs/planning/full_build_architecture_plan.md)
- [Architecture Decision Records](docs/adr/0001-local-first-guided-ai.md)

## Development

```powershell
python -m pip install -e ".[dev]"
python -m compileall src tests
python -m pytest -q
```

## Non-Goals

- No autonomous remediation.
- No hidden cloud dependency.
- No replacement for full observability platforms.
- No AI-based mutation of deterministic scores or incident evidence.
