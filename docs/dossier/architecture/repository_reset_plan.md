# Repository Reset Plan

The repo should be cleaned in phases to avoid breaking working tests and packaging.

## Target Top-Level Shape

```text
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
tests/
docs/
deploy/
scripts/
deprecated/
```

Top-level `webui/` should not remain as a separate product root after migration.
In the Rust-primary cutover, `src/Cargo.toml` + `src/crates/` became the runtime/control-plane home,
`src/web/frontend/` became the canonical UI source, and `deprecated/` now holds replaced
legacy paths after verification.

## Web Migration

Original:

```text
webui/
src/web/static/
src/web/ui_dist/
```

Target:

```text
src/web/frontend/
src/web/ui_dist/
```

Migration steps:

1. Move `webui/package.json`, lockfile, tsconfig, vite config, and frontend source into `src/web/frontend` (done).
2. Update Vite `outDir` from `../src/web/ui_dist` to `../ui_dist` (done).
3. Update build scripts to run from `src/web/frontend` (done).
4. Remove `webui/node_modules` (done).
5. Add or confirm ignore rules for `node_modules`, `tsconfig.tsbuildinfo`, and local build caches (done).
6. Keep `src/web/ui_dist` as package data (done).
7. Remove `src/web/static` after the React UI has route parity and tests pass (done for active source).

## Backend API Migration

Historical Python target during migration:

```text
src/web/api.py
src/web/app.py
src/web/routers/*.py
src/web/schemas/*.py
```

Live outcome after cutover:

```text
src/crates/inferra-api/
src/crates/inferra-core/
src/crates/inferra-storage/
src/web/frontend/
```

Migration steps below are archival notes from the Python-to-Rust transition and should not be treated as the current implementation plan.
5. Extract events/incidents/services routes.
6. Keep frontend serving isolated outside route handlers.
7. Preserve endpoint paths during first migration.
8. Add tests per router group.

## CLI Migration

Current:

```text
deprecated/inferra_legacy/cli.py
src/cli_core/
```

Target:

```text
deprecated/inferra_legacy/cli.py  # compatibility only; Rust CLI is primary
src/cli_core/
```

Migration steps:

1. Extract `CommandResult`, `CommandError`, and JSON helpers.
2. Extract HTTP client helpers.
3. Extract setup/config/AI/service command handlers.
4. Extract status/workspace display integration.
5. Keep parser behavior identical until tests pass.
6. Add command-level tests for new investigation commands (first pass done).

## Generated and Local Files

Should be ignored:

```text
src/web/frontend/node_modules/
*.tsbuildinfo
.pytest_cache/
.ruff_cache/
.coverage
build/
dist/
site/
*.egg-info/
__pycache__/
```

Package output should be generated in CI or release scripts, not manually dropped into the repo.

## Documentation Reset

README should become:

- product identity
- quick start
- CLI-first control plane
- web control plane
- AI investigation
- safety model
- install targets
- development

Detailed docs should live under:

```text
docs/operations/
docs/dossier/
docs/adr/
docs/reference/
```

## Acceptance for Repo Cleanup

The reset is done when:

- one official frontend source exists
- one official built frontend output exists
- old static UI is removed or explicitly archived
- no committed dependency folders remain
- API routes are split by domain
- CLI code is split by domain
- tests pass
- README no longer undersells the product
