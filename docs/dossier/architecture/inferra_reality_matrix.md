# Inferra Reality Matrix

This document is the blunt crate-level reality check for the current Rust runtime.
It is intentionally narrower than the planning docs. If a capability is not backed
by active code in the current crates, it is not treated as implemented here.

Status vocabulary used below:

- `implemented`: active code path exists and is used by the current runtime
- `partial`: real code exists, but the behavior is thinner or less complete than the config or docs imply
- `external-only`: the field or knob is populated by another crate or runtime edge, not by the crate being described
- `dead`: schema, config, or contract surface exists without an active implementation path

## Crate Reality

### `src/crates/inferra-config`

- `implemented`
  - Config path discovery from explicit override, env vars, Windows ProgramData fallback, and local default.
  - Defaults merge from embedded `src/config/defaults.toml`.
  - JSON/TOML conversion, config patch merge, config writes, and `experience` extraction.
  - Server bind parsing with host/port validation.
  - Null values in config patches are now rejected instead of being silently converted into empty strings.
- `partial`
  - Config patching is additive and replace-oriented; it is not a full patch language with explicit delete semantics.
- `dead`
  - None in the crate itself.

### `src/crates/inferra-contracts`

- `implemented`
  - Shared DTOs for overview, collectors, incidents, services, workspace mapping, AI status, and AI doctor payloads.
  - `EventRow.severity` is now constrained to a typed `number-or-label` contract instead of raw JSON.
- `partial`
  - Several DTO fields still use `serde_json::Value` because the runtime payloads are genuinely loose or still need stronger typing.
- `external-only`
  - Fields like AI availability and collector queue depth are filled by API/runtime layers, not by the contracts crate.
- `dead`
  - `AiStatusResponse.registry_model` remains a contract hook without an active producer in the scoped crates.

### `src/crates/inferra-storage`

- `implemented`
  - SQLite schema creation and additive migrations for `events.db` and `incidents.db`.
  - Governed event ingest with dedup windows, blocklist, allowlist, fingerprint tracking, and governance counters.
  - Collector cursor/state storage.
  - Incident, hypothesis, cluster, explanation, feedback, AI trace, state-log, operator-context persistence.
  - Batch `get_events` lookup now uses a single query instead of one query per event id.
  - Inference graph snapshots and incident chat messages now have real store methods instead of schema-only tables.
- `partial`
  - `query_logs` is still hard-coded to the last 24 hours.
  - Governance supports dedup and list-based suppression, but not the broader adaptive or registry-driven behavior implied by the defaults file.
- `dead`
  - `raw_events` is still schema-only in the current scoped crates.

### `src/crates/inferra-core`

- `implemented`
  - Overview assembly from config, storage, host sampling, service enrichment, and workspace discovery.
  - Event-to-incident reconciliation with heuristic grouping, clustering, and hypothesis generation.
  - Workspace discovery now respects `workspace.enabled`, `workspace.roots`, `workspace.max_depth`, and `workspace.max_results`.
  - AI status projection now carries `model_status` and `model_investigate` overrides from config.
  - Process samples are sorted deterministically before truncation.
  - Storage open and governance-summary failures now propagate instead of being flattened into fake empty state.
- `partial`
  - Incident reasoning is still heuristic pattern matching, not the full inference/scoring/calibration system suggested by the planning docs.
  - Only a small, concrete subset of `incident_lifecycle` and `hypothesis_engine` is honored directly.
  - Containers are still not discovered by `inferra-core`; the runtime contract supports them, but the crate does not populate them.
- `external-only`
  - AI availability, queue depth, and collector errors are injected later by `inferra-api`.
- `dead`
  - Planning-era config families like `anomaly_detection`, `scoring`, `calibration`, and `contradiction_handling` are not implemented in this crate.

### `src/crates/inferra-collectors`

- `implemented`
  - Collector runtime with host metrics, process monitoring, Linux syslog, file tailing, journald, Windows Event Log, Windows service monitoring, Docker, Kubernetes, and app ingest.
  - Governed persistence bridge into `inferra-storage`, followed by incident reconciliation.
  - Standalone ingest server with optional bearer token.
  - App ingest now reports whether an event was actually accepted or suppressed.
  - Reconciliation failures are no longer swallowed after a successful write.
  - `queue_depth` now represents in-flight ingest operations instead of a permanently dead zero.
  - Collector rows now update `events_per_second` and `lag_seconds` with real runtime values.
- `partial`
  - `queue_depth` is an in-flight operations metric, not a buffered queue implementation.
  - Main-API app ingest is still executed through `inferra-api`, not a dedicated collector task.
  - Collector behavior still depends heavily on shelling out to host tools like `journalctl`, `docker`, `kubectl`, `wevtutil`, and `sc.exe`.
- `dead`
  - None newly introduced, but several defaults-file knobs still feed config structs without deeper enforcement in storage.

### `src/crates/inferra-windows-service`

- `implemented`
  - Service install, remove, start, stop, restart, status query, and Windows service dispatcher integration.
  - `sc.exe` command-line generation for service install with retry on delete races.
  - Service-host shutdown path through `serve_with_shutdown`.
  - Restart now ignores only safe stop failures instead of swallowing every error.
  - `sc.exe` field parsing is stricter and the tests now compile on Unix as well as Windows.
- `partial`
  - Status still depends on parsing `sc.exe` text output rather than a stronger SCM API abstraction.
  - Service control support is still basically stop/interrogate only.
  - Logging is still best-effort file append, although it now writes timestamped lines.
- `external-only`
  - The service crate hosts `inferra-api`; it does not own runtime health, storage, or collector semantics itself.
- `dead`
  - None in the active surface.

## Config Surface Reality

### Real in the scoped crates

- `server.host`
- `server.port`
- `storage.data_dir`
- `storage.events_db`
- `storage.incidents_db`
- `collectors.*` source toggles and collector-specific poll/config values
- `deduplication.enabled`
- `deduplication.window_seconds`
- `deduplication.max_tracked_fingerprints`
- `deduplication.severity_escalation_splits`
- `noise_filter.enabled`
- `noise_filter.blocklist_enabled`
- `noise_filter.allowlist_enabled`
- `noise_filter.always_keep_severity`
- `noise_filter.blocklist`
- `noise_filter.allowlist`
- `correlation.cluster_min_events`
- `incident_lifecycle.merge_time_threshold_seconds`
- `incident_lifecycle.stale_timeout_seconds`
- `incident_lifecycle.limits.max_clusters_per_incident`
- `hypothesis_engine.max_hypotheses_per_incident`
- `topology.edges`
- `workspace.enabled`
- `workspace.roots`
- `workspace.max_depth`
- `workspace.max_results`
- `workspace.service_mappings`
- `ai.enabled`
- `ai.provider`
- `ai.base_url`
- `ai.model`
- `ai.allow_remote`
- `ai.model_status`
- `ai.model_investigate`
- `experience.*`

### Partial in the scoped crates

- `storage.retention_hours`: used by prune logic, but pruning must still be invoked by a caller.
- `noise_filter.registry_enabled`: config is carried through, but there is no deeper registry workflow here.
- `noise_filter.high_rate_threshold_per_minute`: visible in summaries/config structs, not enforced as a real adaptive suppressor in storage.
- `incident_lifecycle.*`: only a subset affects active incident logic.
- `workspace.map_runtime` and `workspace.redact_env_files`: exposed in defaults, not implemented by the scoped crates.

### External-only in the scoped crates

- `server.auth_token_env`
- `server.require_loopback`
- `server.cors_origins`
- `server.expose_prometheus_metrics`
- `server.rate_limit_chat_tokens_per_minute`
- `server.rate_limit_explain_tokens_per_minute`
- `collectors.auto_start`

These are handled by API or CLI layers outside the six audited crates.

### Dead in the scoped crates

- `anomaly_detection.*`
- `inference_graph.*` as a reasoning engine
- `hypothesis_validation.*`
- `scoring.*`
- `calibration.*`
- `contradiction_handling.*`
- Most of the planning-oriented knobs that describe a future scoring or calibration system instead of current runtime behavior

## What Changed In This Audit Pass

- Config patching became stricter and safer.
- Storage activated previously schema-only graph/chat surfaces.
- Collectors now expose honest ingest acceptance and stop swallowing correlation failures.
- Core now respects real workspace config and stops flattening storage failures into fake healthy state.
- Contracts now use a typed severity surface instead of raw JSON.
- Windows service behavior is less brittle and less silently permissive.
