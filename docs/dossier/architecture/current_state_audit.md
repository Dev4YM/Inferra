# Current State Audit

This document is a higher-level narrative snapshot and should not be used as the
source of truth for implementation completeness. For the active crate-by-crate
reality check, use `docs/dossier/architecture/inferra_reality_matrix.md`.

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

### Native HTTP Surface Is The Live Backend

The active HTTP/runtime surface now lives in the Rust workspace (`src/crates/inferra-api/`, `src/crates/inferra-core/`, `src/crates/inferra-collectors/`, `src/crates/inferra-storage/`). The React app in `src/web/frontend/` talks to that native API, and archived Python web-layer notes should not be treated as the live control plane.

### CLI Parser Still Needs Deeper Decomposition

The active CLI control plane now lives in `src/crates/inferra-cli/src/main.rs` and delegates runtime concerns into the Rust workspace crates. Archived `src/cli.py` notes are historical only and should not guide new implementation work.

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

This is now a credible CLI-first control plane. The next issue is runtime UX finish: keep the Rust CLI ergonomics sharp, make onboarding feel smoother, and ensure every important web action has an equally good CLI path.

## Immediate Cleanup Findings

- Continue moving shared Rust API contracts and remaining dashboard glue into clearer modules.
- Keep simplifying the native Rust CLI command surface and operator ergonomics.
- Add browser coverage for the new Overview, AI Investigator, Workspace, Control, and Settings screens.
- Reconcile web mode persistence with persisted config mode so operator/developer behavior feels consistent everywhere.
- Deliberately decide what to do with generated docs/site artifacts in git instead of letting staged generated deletions linger.
