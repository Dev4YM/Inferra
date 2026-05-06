# Acceptance Gates

These gates define what "polished" means.

Current status: the first dossier implementation pass satisfies the broad direction of these gates, but the project should still treat them as release gates. Passing tests is not the same thing as finished product polish.

## Repository Gate

Status: mostly passing, with one deliberate git hygiene decision still needed around generated documentation/site artifacts.

Pass conditions:

- no committed `node_modules`
- no duplicated active UI source
- no unexplained top-level junk files
- generated output is either ignored or intentionally packaged
- source folders have clear ownership
- README matches product direction

## CLI Gate

Status: first pass passing. The CLI is now a credible primary control plane, but command registration still needs deeper modularization and more UX QA.

Pass conditions:

- CLI can complete first-run setup
- CLI can configure AI
- CLI can configure mode
- CLI can manage Inferra service
- CLI can inspect collectors
- CLI can inspect incidents
- CLI can run investigation commands
- important commands support `--json`
- operator and developer modes produce different output density

## Web Gate

Status: first pass passing by shape. It has the intended tabs and workflows, but still needs browser coverage, interaction polish, and stronger mobile validation.

Pass conditions:

- overview is the default landing screen
- operator mode is useful without raw JSON
- developer mode exposes raw detail
- AI investigation is visible and useful
- workspace context is visible
- control plane can manage Inferra config/collectors/AI/service state where safe
- UI works on desktop and mobile
- browser tests cover the main screens

## AI Gate

Status: first pass passing. The system now has AI investigation, ask/report/trace commands, safety boundaries, and deterministic fallback behavior.

Pass conditions:

- AI can summarize current state
- AI can investigate latest incident
- AI can suggest safe next steps
- AI cites evidence or uncertainty
- AI does not execute actions
- remote provider risk is visible
- prompt trace exists for developer mode

## Workspace Gate

Status: first pass passing. Discovery, mapping, CLI, API, and web surfaces exist; richer runtime-to-project signals remain the next depth work.

Pass conditions:

- projects are discovered
- services can map to projects
- mappings have confidence
- incidents can show likely project ownership
- AI can use workspace context with redaction

## Backend Gate

Status: first pass passing. Routers exist and tests pass; schemas/contracts and remaining shared glue should continue to be cleaned up.

Pass conditions:

- API routers are split by domain
- endpoint contracts remain stable
- storage migrations pass
- core tests pass
- integration tests pass
- read-only safety model remains intact

## Release Gate

Status: not yet release-complete. This is the active hardening phase after the dossier implementation.

Pass conditions:

- fresh install works
- upgrade works
- service install path is tested or documented
- Docker path works
- docs command smoke tests pass
- full test suite passes
