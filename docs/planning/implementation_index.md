# Implementation index

Each planning document describes intent and contracts; this index maps those documents to the primary Python packages under `src/` (flat layout; no nested `inferra/` package).

| Planning doc | Primary implementation |
| --- | --- |
| [architecture_overview.md](architecture_overview.md) | Cross-cutting: `app.py`, `runtime/`, `web/`, `reasoning/`, `storage/` |
| [full_build_architecture_plan.md](full_build_architecture_plan.md) | Product-wide map; see sections for module pointers |
| [data_flow_contracts.md](data_flow_contracts.md) | `events/`, `normalization/pipeline.py`, `storage/event_store.py`, `storage/incident_store.py` |
| [collector_architecture.md](collector_architecture.md) | `collectors/` |
| [log_normalization_pipeline.md](log_normalization_pipeline.md) | `normalization/` |
| [event_model.md](event_model.md) | `events/models.py`, `events/serialization.py` |
| [event_deduplication.md](event_deduplication.md) | `normalization/dedup.py` |
| [noise_filtering.md](noise_filtering.md) | `normalization/noise.py` |
| [anomaly_detection.md](anomaly_detection.md) | `analysis/anomaly.py`, `reasoning/signals/` |
| [correlation_engine.md](correlation_engine.md) | `analysis/correlation.py` |
| [incident_lifecycle.md](incident_lifecycle.md) | `analysis/lifecycle.py` |
| [causal_graph_engine.md](causal_graph_engine.md) | `reasoning/inference_graph.py` |
| [hypothesis_engine.md](hypothesis_engine.md) | `reasoning/engine.py`, `reasoning/composer.py` |
| [hypothesis_validation.md](hypothesis_validation.md) | `reasoning/validation.py` |
| [scoring_engine.md](scoring_engine.md) | `reasoning/scoring.py`, `storage/weight_store.py` |
| [contradiction_handling.md](contradiction_handling.md) | `reasoning/contradiction.py` |
| [confidence_calibration.md](confidence_calibration.md) | `storage/calibration_store.py`, `reasoning/scoring.py` |
| [failure_model.md](failure_model.md) | `analysis/models.py` |
| [failure_taxonomy.md](failure_taxonomy.md) | `core/enums.py`, `reasoning/` |
| [explanation_layer.md](explanation_layer.md) | `explanation/`, `ai/` (presentation only) |
| [runtime_context_builder.md](runtime_context_builder.md) | `runtime/context.py` |
| [storage_architecture.md](storage_architecture.md) | `storage/` |
| [ui_spec.md](ui_spec.md) | `web/frontend/`, `web/ui_dist/`, `web/api.py`, `web/live_hub.py` |
| [system_limitations.md](system_limitations.md) | Cross-cutting constraints; see code references in doc |
| [constraints.md](constraints.md) | `config/`, `core/` |
| [testing_strategy.md](testing_strategy.md) | `tests/` |
