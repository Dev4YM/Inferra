# Execution Roadmap

This roadmap turns the reset into buildable phases and records the current implementation status.

## Phase 1: Repo and Product Reset

Status: first pass complete.

Goal:

Make the repo professional and pick one official product shape.

Work:

- update README positioning
- add `experience` config section
- remove committed frontend dependencies
- move `webui` into `src/web/frontend` (done)
- update frontend build scripts
- keep `src/web/ui_dist` as build output
- document official frontend source
- add ignore rules

Exit criteria:

- one frontend source
- no committed dependency folder
- tests pass
- docs point to new product identity

## Phase 2: API and CLI Modularization

Status: first pass complete, deeper decomposition still needed.

Goal:

Make the control planes maintainable.

Work:

- split `src/web/api.py` into routers
- split `src/cli.py` into command modules
- preserve endpoint paths
- preserve command behavior
- add tests per router/command group

Exit criteria:

- `create_app` still works
- existing API tests pass
- CLI tests pass
- files are smaller and domain-owned

Remaining polish:

- move shared API schemas/contracts out of route functions
- keep reducing app/websocket/dashboard glue in `src/web/api.py`
- move parser registration and remaining legacy command glue out of `src/cli.py`

## Phase 3: Experience Modes

Status: first pass complete.

Goal:

Support operator/developer modes across CLI and web.

Work:

- add config model
- add `inferra mode`
- add CLI output mode behavior
- add web mode toggle
- add dashboard density variants
- add tests

Exit criteria:

- operator is default
- developer exposes raw detail and workspace context
- mode persists

Remaining polish:

- reconcile web local mode selection with persisted config mode
- audit every CLI/web command for meaningful operator/developer differences

## Phase 4: Real Dashboard

Status: first pass complete, product polish needed.

Goal:

Replace JSON-first pages with a real overview control plane.

Work:

- overview dashboard
- top concern panel
- incident summary
- service health
- collector health
- AI next steps
- workspace summary
- developer diagnostics drawer or tab

Exit criteria:

- landing page answers "what matters now?"
- no raw JSON as primary UI
- mobile and desktop usable
- screenshot/browser tests updated

Remaining polish:

- add richer incident timelines and service drilldowns
- improve empty states and guided onboarding
- add browser coverage for the new control-plane screens

## Phase 5: AI Investigation

Status: first pass complete.

Goal:

Make AI drive investigation flow.

Work:

- investigation bundle builder
- structured AI output contract
- `inferra investigate now`
- `inferra ai ask`
- web AI investigator panel
- prompt trace improvements
- safe next-step suggestions

Exit criteria:

- AI returns prioritized next steps
- every AI claim cites evidence or uncertainty
- remote provider warnings visible
- no execution of suggested actions

Remaining polish:

- strengthen citation rendering in UI
- improve prompt trace ergonomics in developer mode
- add more fixtures around fallback and degraded AI behavior

## Phase 6: Workspace Intelligence

Status: first pass complete.

Goal:

Connect runtime and local projects.

Work:

- workspace config
- project model
- runtime mapping signals
- mapping confidence
- service/project UI
- AI workspace context

Exit criteria:

- services can link to projects
- incidents can link to projects
- CLI can inspect workspace mappings
- AI can use workspace evidence safely

Remaining polish:

- deepen process/container/project mapping signals
- improve low-confidence mapping guidance
- expose richer project details in service and incident views

## Phase 7: Polish and Release Hardening

Status: active next phase.

Goal:

Make the project feel complete.

Work:

- docs cleanup
- packaging smoke tests
- service repair command
- dashboard visual polish
- onboarding QA
- demo data mode
- release checklist

Exit criteria:

- fresh user can install, setup, run, inspect, and understand
- all tests pass
- docs match behavior
- repo has no obvious dropped artifacts
