# Inferra Reality Matrix

This document is the blunt crate-level reality check for the current Rust runtime.
It is intentionally narrower than the planning docs. If a capability is not backed
by active code in the current crates, it is not treated as implemented here.

Status vocabulary used below:

- `implemented`: active code path exists and is used by the current runtime
- `partial`: real code exists, but the behavior is thinner or less complete than the config or docs imply
- `external-only`: the field or knob is populated by another crate or runtime edge, not by the crate being described
- `dead`: schema, config, or contract surface exists without an active implementation path

## Execution Ledger

This section is the live implementation ledger for the current hardening pass.
Every substantial next move should be written here before or while it is being implemented.

- `completed`
  - Exposed learned adaptive artifacts through the runtime/API so operators can inspect what the engine learned instead of treating `adaptive_learning.json` as a hidden sidecar.
  - Added bounded governance controls for learned detectors, templates, compositions, and edge profiles so bad learned artifacts can be retired or re-enabled without editing files by hand.
  - Synced the crate and config-surface sections with the real runtime instead of leaving earlier hardening work stranded in chat only.
  - Surface learned-artifact provenance directly on hypotheses and incident detail payloads so operators can see which learned detector, composition, or edge prior materially influenced a result.
  - Add a durable audit log for manual artifact retirement/restore actions instead of only storing current disabled state in `adaptive_learning.json`.
  - Quantify learned-artifact impact inside provenance instead of only reporting participation.
  - Expose a richer adaptive-learning review workflow that groups active influence, recent governance actions, and artifacts needing operator attention.
  - Add longitudinal adaptive-artifact effectiveness plus score-rank and edge-delta history instead of stopping at current incident-local summaries.
  - Add richer approval/review semantics on top of history and governance instead of only enable/disable actions.
  - Push the review workflow into first-class operator UX instead of only route-level JSON surfaces.
  - Replace file-backed adaptive audit/history sidecars with a first-class queryable persistence surface instead of append-only JSONL files.
  - Embed adaptive-artifact review directly into incident detail workflows instead of forcing operators to pivot to a separate learning-review page.
  - Normalize the learned-artifact registry itself out of `adaptive_learning.json` and into first-class relational storage instead of a file-backed snapshot.
  - Add bulk adaptive-learning triage and compare-many analytics instead of leaving operators with only single-artifact review flows and filtered raw lists.
  - Add saved adaptive-review views, reviewer assignment/aging cues, and richer cohort trend drilldowns instead of making operators rebuild bulk selections from scratch every session.
- `next`
  - Replace free-text reviewer ownership with authenticated operator identity plus SLA/notification/reporting surfaces instead of relying on manually typed assignee names and passive queue watching.

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
  - Hypothesis and incident-detail DTOs now expose optional adaptive-learning provenance instead of forcing callers to infer learned influence from internal score blobs.
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
  - Hypothesis reads now surface stored provenance from `score_breakdown` instead of dropping learned-influence metadata on the floor.
  - Adaptive-learning audit and longitudinal history now live in indexed `incidents.db` tables with filterable queries instead of append-only JSONL sidecars.
  - The adaptive-learning registry itself now lives in first-class `incidents.db` tables for processed feedback, learned detectors, learned templates, learned compositions, and learned edge profiles instead of a single file-backed snapshot blob.
  - Adaptive review views now persist in `incidents.db` with saved artifact cohorts, search text, reviewer assignment, and last-used timestamps instead of living only in transient frontend state.
- `partial`
  - `query_logs` is still hard-coded to the last 24 hours.
  - Governance supports dedup and list-based suppression, but not the broader adaptive or registry-driven behavior implied by the defaults file.
- `dead`
  - `raw_events` is still schema-only in the current scoped crates.

### `src/crates/inferra-core`

- `implemented`
  - Overview assembly from config, storage, host sampling, service enrichment, and workspace discovery.
  - Event-to-incident reconciliation with heuristic grouping, clustering, anomaly-aware incident opening, config-driven hypothesis generation, and persisted inference-graph snapshots.
  - Workspace discovery now respects `workspace.enabled`, `workspace.roots`, `workspace.max_depth`, and `workspace.max_results`.
  - AI status projection now carries `model_status` and `model_investigate` overrides from config.
  - Process samples are sorted deterministically before truncation.
  - Storage open and governance-summary failures now propagate instead of being flattened into fake empty state.
  - `incident_lifecycle.limits.max_events_per_incident`, `incident_lifecycle.limits.max_active_incidents`, `incident_lifecycle.enable_auto_split`, and `correlation.analysis_window_seconds` are now honored directly during reconciliation.
  - `hypothesis_engine.min_supporting_events`, `hypothesis_engine.min_generation_confidence`, `hypothesis_engine.dedup_overlap_threshold`, and `hypothesis_engine.custom_rules` now shape stored hypotheses instead of sitting dead in config.
  - `anomaly_detection`, `scoring`, `calibration`, and `contradiction_handling` now influence incident scoring, confidence labels, and hypothesis invalidation.
  - `inference_graph.*` now builds a directed plausibility graph, records root candidates and leaf symptoms, and feeds graph-origin candidates back into hypothesis generation.
  - `scoring.tuning.*` now performs bounded weight adaptation from operator feedback, persists learned weights, and applies deterministic tie-break ordering.
  - `calibration.*` now persists score buckets, tracks feedback accuracy, and assigns staleness-aware confidence labels instead of using fixed thresholds forever.
  - Confirmed operator feedback now mines stable lexical/tag/source-type patterns from supporting events, persists learned detectors/templates, and reuses them during later hypothesis generation instead of leaving the detection layer frozen.
  - Confirmed operator feedback now also learns multi-signal compositions from matched requirements plus incident-graph context, persists them, and reuses them as learned hypothesis rules on later incidents.
  - Confirmed and rejected feedback now update bounded edge priors per edge type/service/cause so later inference-graph construction can slightly strengthen or weaken plausibility based on audited incident history.
  - Stored hypotheses now carry explicit provenance for learned detectors, templates, compositions, and edge profiles instead of burying learned influence behind opaque scoring.
  - Stored hypothesis provenance now includes quantitative impact hints like prior contribution, matched-evidence counts, and learned edge-profile plausibility deltas instead of just naming participating artifacts.
  - Manual adaptive-artifact governance now persists durable audit entries in `incidents.db` instead of only mutating current artifact state or appending a sidecar log.
  - Incident reasoning refresh now persists longitudinal adaptive-learning history rows in `incidents.db` with per-artifact score movement, rank movement, and edge-delta observations instead of only keeping the latest state.
  - Learned adaptive artifacts now carry explicit review state (`unreviewed`, `approved`, `watch`, `rejected`), and rejected artifacts are retired from runtime influence through the same audited governance path instead of leaving review decisions purely advisory.
  - Learned detectors, templates, compositions, and edge profiles now load from and persist back to relational registry tables in `incidents.db`; `adaptive_learning.json` is only a legacy import path now, not the runtime source of truth.
  - Bulk adaptive-artifact review/runtime changes now apply across multiple selected artifacts in one bounded mutation pass and one reasoning refresh instead of forcing N separate operator actions.
  - Adaptive review payloads now compute saved-view aging cues plus richer per-artifact trend drilldowns from longitudinal history instead of leaving operators with only static aggregate counters.
  - `incident_lifecycle.archive_after_days` now archives stale/resolved incidents out of the active store into dated archive databases.
  - Containers are discovered on a best-effort basis from `docker` or `podman` and surfaced in the runtime overview.
  - Overview health composition for AI and collector runtime signals now happens in `inferra-core` rather than as an ad-hoc API-only patch.
- `partial`
  - The learning loop is still intentionally bounded and auditable: it learns detectors, compositions, and edge priors, but it does not invent brand-new graph-building strategies or an unconstrained self-modifying causal engine.
  - Legacy `adaptive_learning.json` compatibility import still exists for migration, but there is no explicit cleanup/export tooling around that legacy artifact yet.

### `src/crates/inferra-api`

- `implemented`
  - Native HTTP routes for config, overview, metrics, incidents, services, AI, workspace, topology, and collector control.
  - `api_overview` now passes live AI/collector runtime signals into `inferra-core` instead of patching health later.
  - `POST /api/incidents/{incident_id}/feedback` persists operator feedback and immediately refreshes incident reasoning.
  - `GET /api/learning/adaptive` now exposes learned detectors, templates, compositions, and edge profiles with active/suppressed/manual-disable status.
  - `GET /api/learning/adaptive/audit` now exposes durable governance actions for adaptive artifacts.
  - `GET /api/learning/adaptive/history` now exposes longitudinal artifact observations including score/rank movement and edge-delta history.
  - `GET /api/learning/adaptive/review` now groups active incident influence, artifacts needing attention, review counts, pending review queue, and recent review activity into a review-oriented payload instead of only raw per-route dumps.
  - `POST /api/learning/adaptive/{artifact_kind}/{artifact_id}` now allows bounded operator governance over learned artifacts by disabling or re-enabling them without hand-editing the sidecar file.
  - `POST /api/learning/adaptive/{artifact_kind}/{artifact_id}/review` now records bounded operator review decisions (`approve`, `watch`, `reject`, `reset`) and folds rejection into audited runtime retirement instead of treating review as a comment-only side channel.
  - Adaptive audit/history routes now query SQLite-backed persistence with real `limit`, `offset`, and filter parameters instead of only replaying whole JSONL sidecars.
  - Incident detail payloads now expose aggregated adaptive-learning provenance and per-hypothesis provenance, including quantified artifact impact hints instead of hiding learned influence from operators.
  - Adaptive summary/review surfaces now report the relational registry storage reference rather than pretending the live adaptive model still lives in a file snapshot.
  - Adaptive review now also exposes compare-many analytics plus bulk review/runtime mutation routes so operators can triage multiple artifacts in one pass instead of hand-repeating single-artifact calls.
  - Adaptive review now also exposes saved-view create/use/delete routes, reviewer assignment metadata, view aging cues, and cohort trend drilldowns so multi-session triage can be resumed instead of rebuilt.
- `partial`
  - Adaptive-learning governance now has durable saved-review surfaces, but reviewer ownership is still a free-text field rather than an authenticated operator identity with notification or SLA semantics.
- `external-only`
  - The API surfaces adaptive-learning governance, but the learning logic itself still lives in `inferra-core`.

### `src/web/frontend`

- `implemented`
  - The React operator console now has a dedicated `Learning Review` page instead of forcing operators to work from raw adaptive-learning JSON routes.
  - The learning-review UI exposes a pending review queue, attention list, searchable artifact inventory, incident influence view, recent review activity, and per-artifact history summaries.
  - Operators can now approve, watch, reject, reset, enable, and disable learned artifacts from the real UI instead of hand-crafting POST payloads.
  - Adaptive-learning review/governance payloads now have typed frontend contracts instead of being consumed as anonymous blobs.
  - Incident detail pages now expose inline adaptive-artifact review and runtime-governance actions for the learned artifacts actually influencing that incident instead of forcing a context switch to the dedicated review page.
  - The learning-review page now supports compare-many analytics, multi-select artifact comparison, and bulk approve/watch/reject/reset/enable/disable actions instead of trapping operators in one-artifact-at-a-time triage.
  - The learning-review page now supports saved review cohorts, reviewer assignment cues, persisted queue recall, and trend drilldowns for the selected cohort instead of forcing operators to reconstruct the same artifact sets every visit.
- `partial`
  - The operator UX still lacks authenticated reviewer identity, notifications, and SLA/reporting surfaces around saved adaptive-review queues.

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
- `correlation.analysis_window_seconds`
- `incident_lifecycle.merge_time_threshold_seconds`
- `incident_lifecycle.stale_timeout_seconds`
- `incident_lifecycle.archive_after_days`
- `incident_lifecycle.enable_auto_split`
- `incident_lifecycle.limits.max_clusters_per_incident`
- `incident_lifecycle.limits.max_events_per_incident`
- `incident_lifecycle.limits.max_active_incidents`
- `hypothesis_engine.max_hypotheses_per_incident`
- `hypothesis_engine.min_supporting_events`
- `hypothesis_engine.min_generation_confidence`
- `hypothesis_engine.dedup_overlap_threshold`
- `hypothesis_engine.custom_rules`
- `anomaly_detection.*`
- `inference_graph.*`
- `hypothesis_validation.contradiction_ratio_fail`
- `hypothesis_validation.contradiction_ratio_warn`
- `scoring.*`
- `scoring.tuning.*`
- `calibration.*`
- `contradiction_handling.*`
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
- `incident_lifecycle.*`: lifecycle automation is real now, but some planning-era ambitions like richer archival policies and first-class operator UX are still not present.
- Adaptive-artifact governance is runtime/API-driven, not a config family.

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

- `workspace.map_runtime`
- `workspace.redact_env_files`
- Most of the planning-oriented knobs that still describe future registry/review workflows rather than active runtime behavior

## What Changed In This Audit Pass

- Config patching became stricter and safer.
- Storage activated previously schema-only graph/chat surfaces.
- Collectors now expose honest ingest acceptance and stop swallowing correlation failures.
- Core now respects real workspace config and stops flattening storage failures into fake healthy state.
- Contracts now use a typed severity surface instead of raw JSON.
- Windows service behavior is less brittle and less silently permissive.
- Core adaptive learning now covers learned detectors, learned compositions, and learned edge priors instead of stopping at score tuning.
- API now exposes and governs adaptive-learning artifacts so operators can inspect and retire learned behavior without hand-editing sidecar files.
- Incident and hypothesis payloads now surface learned-artifact provenance instead of hiding it inside internal scoring state.
- Manual adaptive-artifact governance now leaves a durable audit trail instead of mutating sidecar state silently.
- Adaptive-learning review now has a dedicated summary route and provenance includes quantitative impact hints instead of only yes/no participation.
- Adaptive-learning now records and exposes longitudinal score/rank and edge-delta history instead of only current-state summaries.
- Adaptive-learning artifacts now support explicit review decisions, and rejected artifacts can be retired through the same audited operator workflow instead of leaving approval as an implicit social process.
- The frontend now turns adaptive-learning governance into a first-class operator workflow with a real review queue and action surface instead of leaving the feature stranded behind JSON endpoints.
- Adaptive audit/history persistence now lives in indexed SQLite tables with filterable API queries and legacy sidecar import, instead of append-only JSONL files.
- Incident detail workflows now let operators review and retire the learned artifacts influencing that incident instead of forcing a separate-page pivot for every decision.
- The adaptive-learning registry itself now lives in relational `incidents.db` tables with legacy `adaptive_learning.json` import-only compatibility, so the runtime no longer depends on a file-backed learned-artifact snapshot.
- Adaptive-learning review now supports compare-many analytics and bulk triage mutations across selected artifacts instead of forcing repetitive single-artifact operator clicks.
- Adaptive-learning review now persists saved views with reviewer/aging cues and exposes cohort trend drilldowns so operators can resume and inspect triage queues across sessions instead of rebuilding them manually.
