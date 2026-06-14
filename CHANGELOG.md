# Changelog

All notable user-facing changes are staged here by release version. The canonical release number is in [`VERSION`](../VERSION); see [versioning](docs/operations/versioning.md).

## 0.3.0

### Workspace and API performance

- Add `GET /api/workspace/apps/{app_name}` for fast cache/disk app lookup without a full workspace rescan.
- Workspace map uses stale-while-revalidate: return the last snapshot immediately and refresh in the background.
- Move blocking workspace collector work to `spawn_blocking` so the API event loop stays responsive.
- Workspace app detail page loads from the dedicated app endpoint instead of blocking on `/api/workspace/map`.
- Frontend API fetches use a 30s timeout; live resource polling pauses when the tab is in the background.

### Systems, services, and navigation

- Service detail falls back to database lookup when the service is not in the overview top-30 list (fixes “Service not found”).
- Systems inventory links to workspace app detail when a mapped service row is missing from overview.
- Workspace app matching is case-insensitive for `name` and `display_name`.

### Windows install and collectors

- Add `deploy/windows/InferraInstall.psm1`, `scripts/install-inferra.ps1`, `scripts/uninstall-inferra.ps1`, and `scripts/build-all.ps1` for full build + install to `%ProgramFiles%\Inferra`.
- Default Windows service noise filtering reduces unmapped system-service clutter in workspace scans.

### Control plane UX

- Evidence page: clearer loading placeholder, error recovery, and Windows-service noise filter.
- Incident detail: deduplicated evidence, synced suggested checks, and improved investigation state.
- Systems and Control: faster load paths, idle collector visibility, grouped unmapped services panel.
- Overview: platform health summary and noise mute controls.
- Graph: search, reset layout, and filter controls with persisted layout storage.
- Workspace: empty state, AI scope pre-fill from context, and resilient list/detail navigation.

### Versioning, CI, and quality

- Introduce canonical `VERSION` file with `scripts/version.py sync|verify` and CI enforcement.
- Fix Ruff unused-variable failures in Rust API integration tests.
- Fix Clippy `single_char_add_str` in storage query builder.
- Fix cross-platform `ui_dist_candidates` CLI test for Linux CI.

## 0.2.0

### Resilience and operations

- Classify timestamps more than one hour in the future with a dedicated `clock_skew_future` quality flag while keeping shorter future drift on the existing `timestamp_in_future` path.
- Track storage and ingestion degradation (`disk_full`, `storage_readonly`, `sqlite_operational`, `disk_space_low`, `raw_queue_saturated`, `ai_unavailable`) and expose them on `/api/health` and `/api/dashboard`.
- Pause supervised collectors after disk-full or read-only SQLite write failures; ingestion skips events when the event store rejects writes.
- AI explain paths fall back to the deterministic template when the Ollama stream fails mid-flight or the provider raises; WebSocket explain/chat streams recover the same way.
- Collector supervisor retry sleeps include bounded jitter; collector emit retries add a small random delay under queue backpressure.
- Content-Security-Policy headers are applied to HTTP responses for the bundled UI.

### Tests and docs

- Add `pytest.mark.chaos` scenarios (POSIX SIGKILL mid-transaction, Ollama stream failure, disk-full degradation, clock skew) and an integration upgrade test from events schema v1 to current.
- Document threat model (`docs/security/threat_model.md`) and operator release steps (`docs/operations/release.md`).

## 0.1.0

Initial public development baseline: local-first ingestion, SQLite storage with migrations, deterministic reasoning, optional Ollama-backed explanations, and web/CLI surfaces.
