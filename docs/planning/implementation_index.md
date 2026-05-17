# Implementation index

Each planning document describes intent and contracts; this index maps those
documents to the active Rust workspace. Deprecated Python code under
`deprecated/` is archival reference only and is not part of the shipped runtime.

For a crate-level truth pass on what is implemented versus merely planned, see
`docs/dossier/architecture/inferra_reality_matrix.md`.

| Planning doc | Primary implementation |
| --- | --- |
| [rust_runtime_contracts.md](rust_runtime_contracts.md) | `src/crates/inferra-api`, `src/crates/inferra-cli`, `src/crates/inferra-config`, `src/crates/inferra-core` |
| [architecture_overview.md](architecture_overview.md) | Cross-cutting: `src/crates/inferra-core`, `src/crates/inferra-api`, `src/crates/inferra-storage`, `src/crates/inferra-collectors`, `src/web/frontend/` |
| [full_build_architecture_plan.md](full_build_architecture_plan.md) | Product-wide map; see sections for module pointers |
| [data_flow_contracts.md](data_flow_contracts.md) | `src/crates/inferra-contracts`, `src/crates/inferra-storage`, `src/crates/inferra-core`, `src/crates/inferra-api` |
| [collector_architecture.md](collector_architecture.md) | `src/crates/inferra-collectors`, collector config in `src/config/defaults.toml` |
| [log_normalization_pipeline.md](log_normalization_pipeline.md) | Native ingest and incident reconciliation in `src/crates/inferra-core` plus collector-specific shaping in `src/crates/inferra-collectors` |
| [observability_logs_traces_platform.md](observability_logs_traces_platform.md) | End-to-end map: OTel-aligned signals, SQLite `event_attributes` EAV + optional `events_fts` / `[observability.fts]`, `/api/v2/logs` (`q` vs `search`) now surfaced by the frontend `Evidence` explorer, `GET /api/incidents/{id}/logs`, **`GET /api/traces/{trace_id}`** plus richer frontend route `/traces/:traceId`, row-level **`latest_trace_summary`** on incident/service overview payloads, **`POST /v1/logs`** OTLP HTTP ingest (JSON + protobuf), **`[observability.export]`** OTLP JSON forwarder with retry/split cursor hardening (Phase 8), trace/incident/workspace UX; crates `inferra-storage`, `inferra-collectors`, `inferra-api`, `inferra-contracts`, frontend; [ADR 0007](../adr/0007-observability-trace-columns.md). |
| [event_model.md](event_model.md) | `src/crates/inferra-contracts`, `src/crates/inferra-storage` |
| [event_deduplication.md](event_deduplication.md) | Config surface in `src/config/defaults.toml`; overview/runtime summaries in `src/crates/inferra-core` |
| [noise_filtering.md](noise_filtering.md) | Config surface in `src/config/defaults.toml`; overview/runtime summaries in `src/crates/inferra-core` |
| [anomaly_detection.md](anomaly_detection.md) | Planning reference; the scoped runtime crates do not implement the broader anomaly engine described there |
| [correlation_engine.md](correlation_engine.md) | Native incident clustering and service/topology grouping in `src/crates/inferra-core` |
| [incident_lifecycle.md](incident_lifecycle.md) | `src/crates/inferra-core`, `src/crates/inferra-storage` |
| [causal_graph_engine.md](causal_graph_engine.md) | Native heuristic hypothesis generation in `src/crates/inferra-core`; archived exploratory reference under `deprecated/python_packages/` |
| [hypothesis_engine.md](hypothesis_engine.md) | `src/crates/inferra-core`, `src/crates/inferra-storage` |
| [hypothesis_validation.md](hypothesis_validation.md) | Planning reference; current runtime stores hypotheses but does not implement the full validation subsystem described there |
| [scoring_engine.md](scoring_engine.md) | Partial implementation: current runtime exposes severity/event-rate summaries, not the full scoring engine described in the plan |
| [contradiction_handling.md](contradiction_handling.md) | Investigation output normalization and deterministic fallbacks in `src/crates/inferra-api` |
| [confidence_calibration.md](confidence_calibration.md) | Historical planning reference; active runtime currently exposes deterministic score labels without a real calibration subsystem |
| [failure_model.md](failure_model.md) | Native incident severity and hypothesis cause typing in `src/crates/inferra-core`, `src/crates/inferra-storage` |
| [failure_taxonomy.md](failure_taxonomy.md) | Native cause/severity taxonomy in `src/crates/inferra-core`, `src/crates/inferra-contracts` |
| [explanation_layer.md](explanation_layer.md) | `src/crates/inferra-api` native Ollama + deterministic fallback investigation flow |
| [runtime_context_builder.md](runtime_context_builder.md) | `src/crates/inferra-core` |
| [storage_architecture.md](storage_architecture.md) | `src/crates/inferra-storage` |
| [ui_spec.md](ui_spec.md) | `src/web/frontend/`, `src/web/ui_dist/`, `src/crates/inferra-api` |
| [system_limitations.md](system_limitations.md) | Cross-cutting constraints; see code references in doc |
| [constraints.md](constraints.md) | `src/crates/inferra-config`, `src/crates/inferra-core` |
| [testing_strategy.md](testing_strategy.md) | `tests/` |
