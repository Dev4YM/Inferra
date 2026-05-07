# Implementation index

Each planning document describes intent and contracts; this index maps those
documents to the active Rust workspace. Deprecated Python code under
`deprecated/` is archival reference only and is not part of the shipped runtime.

| Planning doc | Primary implementation |
| --- | --- |
| [rust_runtime_contracts.md](rust_runtime_contracts.md) | `src/crates/inferra-api`, `src/crates/inferra-cli`, `src/crates/inferra-config`, `src/crates/inferra-core` |
| [architecture_overview.md](architecture_overview.md) | Cross-cutting: `src/crates/inferra-core`, `src/crates/inferra-api`, `src/crates/inferra-storage`, `src/crates/inferra-collectors`, `src/web/frontend/` |
| [full_build_architecture_plan.md](full_build_architecture_plan.md) | Product-wide map; see sections for module pointers |
| [data_flow_contracts.md](data_flow_contracts.md) | `src/crates/inferra-contracts`, `src/crates/inferra-storage`, `src/crates/inferra-core`, `src/crates/inferra-api` |
| [collector_architecture.md](collector_architecture.md) | `src/crates/inferra-collectors`, collector config in `src/config/defaults.toml` |
| [log_normalization_pipeline.md](log_normalization_pipeline.md) | Native ingest and incident reconciliation in `src/crates/inferra-core` plus collector-specific shaping in `src/crates/inferra-collectors` |
| [event_model.md](event_model.md) | `src/crates/inferra-contracts`, `src/crates/inferra-storage` |
| [event_deduplication.md](event_deduplication.md) | Config surface in `src/config/defaults.toml`; overview/runtime summaries in `src/crates/inferra-core` |
| [noise_filtering.md](noise_filtering.md) | Config surface in `src/config/defaults.toml`; overview/runtime summaries in `src/crates/inferra-core` |
| [anomaly_detection.md](anomaly_detection.md) | `src/crates/inferra-api` (`/api/anomaly/...`) plus incident heuristics in `src/crates/inferra-core` |
| [correlation_engine.md](correlation_engine.md) | Native incident clustering and service/topology grouping in `src/crates/inferra-core` |
| [incident_lifecycle.md](incident_lifecycle.md) | `src/crates/inferra-core`, `src/crates/inferra-storage` |
| [causal_graph_engine.md](causal_graph_engine.md) | Native heuristic hypothesis generation in `src/crates/inferra-core`; archived exploratory reference under `deprecated/python_packages/` |
| [hypothesis_engine.md](hypothesis_engine.md) | `src/crates/inferra-core`, `src/crates/inferra-storage` |
| [hypothesis_validation.md](hypothesis_validation.md) | Native stored hypothesis validation/state in `src/crates/inferra-core`, `src/crates/inferra-storage` |
| [scoring_engine.md](scoring_engine.md) | Native severity/event-rate/anomaly summaries in `src/crates/inferra-core` |
| [contradiction_handling.md](contradiction_handling.md) | Investigation output normalization and deterministic fallbacks in `src/crates/inferra-api` |
| [confidence_calibration.md](confidence_calibration.md) | Historical planning reference; active runtime currently exposes deterministic score labels without a separate calibration CLI |
| [failure_model.md](failure_model.md) | Native incident severity and hypothesis cause typing in `src/crates/inferra-core`, `src/crates/inferra-storage` |
| [failure_taxonomy.md](failure_taxonomy.md) | Native cause/severity taxonomy in `src/crates/inferra-core`, `src/crates/inferra-contracts` |
| [explanation_layer.md](explanation_layer.md) | `src/crates/inferra-api` native Ollama + deterministic fallback investigation flow |
| [runtime_context_builder.md](runtime_context_builder.md) | `src/crates/inferra-core` |
| [storage_architecture.md](storage_architecture.md) | `src/crates/inferra-storage` |
| [ui_spec.md](ui_spec.md) | `src/web/frontend/`, `src/web/ui_dist/`, `src/crates/inferra-api` |
| [system_limitations.md](system_limitations.md) | Cross-cutting constraints; see code references in doc |
| [constraints.md](constraints.md) | `src/crates/inferra-config`, `src/crates/inferra-core` |
| [testing_strategy.md](testing_strategy.md) | `tests/` |
