# Current State Audit

This audit reflects the repo state after the dossier implementation and hardening pass.

## What Is Strong

The backend engine has real substance:

- collectors exist for multiple platforms
- normalization, deduplication, noise filtering, and storage are implemented
- SQLite migrations exist
- incident lifecycle exists
- deterministic reasoning and scoring exist
- Ollama AI integration exists
- structured AI investigation now exists through CLI and API paths
- workspace discovery and service mapping now exist
- the React dashboard has been consolidated into `src/web/frontend`
- the backend API is split into domain routers for the major new control-plane areas
- the CLI now has domain command modules for the first control-plane slice
- tests are broad and currently pass
- Windows service support exists
- Docker, systemd, Helm, macOS, and release docs exist

The project is no longer just a runtime failure explainer with scattered shells. It is now moving into the intended shape: a CLI-first, AI-assisted runtime intelligence control plane. The remaining issue is polish, consistency, deeper UX, and release-grade organization.

## What Feels Unprofessional

### UI Source Has Been Consolidated

The older `src/web/static/` UI and top-level `webui/` product root have been removed from the active source layout.

The official frontend source is now:

```text
src/web/frontend/
```

The packaged build output is:

```text
src/web/ui_dist/
```

This resolves the previous confusion about which UI should be edited and packaged. Remaining work is product polish: the React UI now has the right control-plane shape, but it still needs more refined workflows, stronger visual hierarchy, and browser-level coverage for the richer screens.

### Frontend Dependencies Are No Longer Source

`node_modules` and TypeScript build-info files are ignored and should not be committed.

### Backend API Split Is Now Mostly Done

`src/web/api.py` is no longer the only owner of web behavior. Frontend asset serving lives in `src/web/frontend_assets.py`, system/control-plane endpoints live in `src/web/routes/system.py`, and domain APIs now live under `src/web/routers/`.

The active split includes:

- `src/web/routers/ai.py`
- `src/web/routers/collectors.py`
- `src/web/routers/config.py`
- `src/web/routers/events.py`
- `src/web/routers/health.py`
- `src/web/routers/incidents.py`
- `src/web/routers/investigate.py`
- `src/web/routers/metrics.py`
- `src/web/routers/services.py`
- `src/web/routers/topology.py`
- `src/web/routers/workspace.py`

`src/web/api.py` still owns app creation, lifespan wiring, websocket behavior, and some shared dashboard composition. That is acceptable for the current pass, but the next polish wave should keep moving shared API contracts and schemas into clearer modules.

### CLI Parser Still Needs Deeper Decomposition

`src/cli.py` is the main control plane, but it is too large and owns too many domains in one file.

The first command-module split now exists under `src/cli_core/commands/`, including AI, config, runtime, service, workspace, and related command handlers. However, `src/cli.py` still owns a lot of parser construction and legacy behavior.

It still mixes:

- parser construction
- setup
- some config defaults
- some collector wiring
- storage/calibration command glue
- mode and status rendering glue
- HTTP client calls

The CLI should remain the main control plane. The direction is right now; the next issue is making the parser and command registration as clean as the command handlers.

### Product Identity Too Narrow

The README now describes Inferra as a local-first AI-integrated runtime intelligence control plane. This matches the intended product direction better than the older "runtime failure explanation system" framing.

### Dashboard Is Now a Control Plane, but Needs Product Polish

The current React UI now has the official home under `src/web/frontend` and provides the intended top-level control-plane shape:

- Overview
- Incidents
- Systems
- Evidence
- AI Investigator
- Workspace
- Control
- Settings

It is no longer just a JSON reader. Remaining work is polish: more guided empty states, browser QA, stronger mobile behavior, richer incident timelines, better service/workspace drilldowns, and persistence that cleanly reconciles UI mode with config mode.

## Current API Inventory

Current FastAPI endpoints include:

```text
GET  /api/version
GET  /api/metrics
GET  /api/config
PUT  /api/config
GET  /api/health
GET  /api/dashboard
GET  /api/runtime/context
GET  /api/workspace/projects
GET  /api/workspace/map
GET  /api/workspace/services
GET  /api/workspace/inspect
POST /api/workspace/mappings
GET  /api/overview
GET  /api/collectors
POST /api/collectors/start
POST /api/collectors/stop
POST /api/collectors/one/start
POST /api/collectors/one/stop
GET  /api/ai/status
GET  /api/ai/models
POST /api/ai/config
GET  /api/ai/doctor
POST /api/ai/ask
GET  /api/ai/report/{incident_id}
GET  /api/events
GET  /api/events/{event_id}
GET  /api/anomaly/{service}/status
GET  /api/logs
GET  /api/incidents
GET  /api/incidents/{incident_id}
GET  /api/incidents/{incident_id}/ai-trace
GET  /api/ai/trace/{incident_id}
GET  /api/incidents/{incident_id}/events
GET  /api/incidents/{incident_id}/hypotheses
POST /api/incidents/{incident_id}/feedback
GET  /api/incidents/{incident_id}/clusters
GET  /api/incidents/{incident_id}/explanation
POST /api/incidents/{incident_id}/chat
GET  /api/incidents/{incident_id}/chat/messages
GET  /api/search/natural
POST /api/incidents/{incident_id}/resolve
GET  /api/services
GET  /api/services/{service_id}
GET  /api/services/{service_id}/events
GET  /api/topology
POST /api/topology/edges
GET  /api/incidents/{incident_id}/state-log
GET  /api/investigate/now
GET  /api/investigate/incident/{incident_id}
GET  /api/investigate/service/{service_id}
```

This is now a good control-plane foundation. The next step is contract hardening: shared response schemas, browser coverage, and better docs for each endpoint family.

## Current CLI Inventory

The CLI currently covers:

```text
run
serve
run-collectors
onboard
setup
check-config
config show/get/set/preset
collectors status/start/stop
ai status/setup/models/pull/test/doctor/ask/investigate/report/trace
init-db
reason-incident
storage verify/vacuum/backup
one-shot collectors
reset-baselines
reset-weights
calibration show
service status/install/start/stop/restart/remove/repair
mode show/set
status
overview
investigate now/latest/incident/service/workspace
incidents list/show
events list/show
services list/show/events
workspace map/services/inspect
doctor
demo seed/clear
completion
```

This is now a credible CLI-first control plane. The next issue is implementation layout and UX finish: continue shrinking `src/cli.py`, make onboarding feel smoother, and ensure every important web action has an equally good CLI path.

## Immediate Cleanup Findings

- Continue moving shared API contracts and remaining websocket/dashboard glue into clearer modules.
- Continue decomposing `src/cli.py` so parser registration and command execution are easier to trust.
- Add browser coverage for the new Overview, AI Investigator, Workspace, Control, and Settings screens.
- Reconcile web mode persistence with persisted config mode so operator/developer behavior feels consistent everywhere.
- Deliberately decide what to do with generated docs/site artifacts in git instead of letting staged generated deletions linger.
