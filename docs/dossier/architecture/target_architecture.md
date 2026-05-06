# Target Architecture

Inferra should become a modular monolith with strong internal boundaries.

The deployment stays simple:

- one local process
- local SQLite
- optional local or remote AI provider
- local web UI
- CLI as the primary control surface

The internal architecture becomes clearer and more product-shaped.

## Target Domain Model

```text
observe
  collectors
  source health
  checkpoints
  runtime snapshots

normalize
  parsing
  enrichment
  fingerprints
  deduplication
  noise filtering

store
  SQLite connections
  migrations
  repositories
  retention
  backup and integrity

detect
  anomaly detection
  correlation
  incident lifecycle
  service health

reason
  inference graph
  signal detectors
  hypothesis composition
  validation
  contradiction handling
  scoring
  calibration

assist
  AI provider config
  investigation plans
  next-step suggestions
  explanations
  chat
  prompt traces

workspace
  project discovery
  service-to-project mapping
  runtime-to-code context
  local config awareness

control
  CLI
  service management
  setup and onboarding
  config profiles
  exports

present
  FastAPI routers
  websocket/live events
  React frontend
  mode-aware UX
```

The current package names do not need to be renamed all at once, but the ownership model should move toward these domains.

## Backend Structure

Recommended `src/web` backend structure:

```text
src/web/
  __init__.py
  app.py
  dependencies.py
  http_security.py
  live_hub.py
  rate_limit.py
  routers/
    ai.py
    collectors.py
    config.py
    dashboard.py
    events.py
    incidents.py
    services.py
    topology.py
    workspace.py
  schemas/
    ai.py
    dashboard.py
    incidents.py
    shared.py
  frontend/
    package.json
    src/
    vite.config.ts
  ui_dist/
```

`src/web/api.py` should become a compatibility shim during migration, then disappear or only expose `create_app`.

## CLI Structure

Recommended CLI structure:

```text
src/cli.py
src/cli_core/
  __init__.py
  parser.py
  result.py
  http_client.py
  output.py
  commands/
    ai.py
    collectors.py
    config.py
    incidents.py
    service.py
    setup.py
    status.py
    storage.py
    workspace.py
```

`src/cli.py` remains the entry point for packaging compatibility.

## Data Flow

```text
sources
  -> collectors
  -> raw event queue
  -> normalization pipeline
  -> dedup/noise
  -> event store
  -> anomaly/correlation
  -> incident lifecycle
  -> reasoning/hypotheses
  -> AI investigation assist
  -> CLI/web presentation
```

AI may read structured outputs and suggest next inspection steps. AI must not write directly into observed systems and must not silently mutate deterministic evidence.

## Control Flow

CLI controls:

- setup
- config
- service lifecycle
- collectors
- AI provider
- storage maintenance
- workspace discovery
- exports
- investigation commands

Web controls:

- visual investigation
- mode selection
- collector start/stop
- config editing with guardrails
- AI investigation sessions
- evidence review
- topology edits
- incident feedback and resolution state

## Experience Modes

Mode should be part of configuration:

```toml
[experience]
mode = "operator"
ai_role = "investigator"
suggest_safe_actions = true
execute_actions = false
show_raw_evidence_by_default = false
```

The implementation now starts with this typed config section. Remaining work is to make output density and web view behavior consistently honor it.

## Safety Model

Allowed:

- read logs
- read local runtime state
- read workspace metadata
- explain evidence
- suggest safe checks
- generate commands for the user to run
- export reports

Not allowed:

- auto-remediate
- restart services
- edit app code
- change infrastructure
- delete logs
- mutate observed systems
- hide AI uncertainty

The service management commands are for managing Inferra itself, not observed systems.
