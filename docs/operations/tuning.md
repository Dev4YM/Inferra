# Tuning Inferra

Inferra keeps deterministic reasoning auditable: thresholds and weights are configuration, not hidden constants. This guide maps common operator controls to `inferra.toml` sections. Values below match `src/config/defaults.toml` unless noted.

## Anomaly and baselines

Anomaly signals use time-bucketed baselines and EMA-style updates. Tunables live under `[anomaly_detection]`:

- `bucket_interval_minutes`, `baseline_update_interval_hours`, `baseline_alpha`, `cold_start_hours`, `min_samples_for_confidence`
- `spike_z_threshold`, `sustained_lookback_buckets`, `absence_sensitivity`
- Component weights under `[anomaly_detection.weights]` and `[anomaly_detection.event_score_weights]`

Reset learned baselines after major topology or traffic changes:

```powershell
inferra --config inferra.toml reset-baselines
```

Implementation: `src/analysis/anomaly.py`, baseline persistence under the configured `storage.data_dir`.

## Correlation and clustering

Temporal decay and graph clustering thresholds are under `[correlation]` (for example `temporal_half_life_seconds`, `cluster_min_edge_weight`, `merge_on_shared_service_and_time`).

Implementation: `src/analysis/correlation.py`, `src/analysis/lifecycle.py`.

## Inference graph and hypotheses

Graph construction budgets and strategy toggles are under `[inference_graph]` and `[inference_graph.strategies]`. Hypothesis caps and custom merge rules use `[hypothesis_engine]` and `[[hypothesis_engine.custom_rules]]`.

Implementation: `src/reasoning/inference_graph.py`, `src/reasoning/engine.py`, `src/reasoning/composer.py`.

## Scoring weights and learning

Default component weights live under `[scoring]`. Bounded learning from operator feedback uses `[scoring.tuning]` (`learning_rate`, `max_drift_from_default`, `min_weight`, `tiebreak_order`).

Inspect calibration buckets (staleness-aware labels used with scoring):

```powershell
inferra --config inferra.toml calibration show
```

Reset weights to defaults after bad feedback or drift:

```powershell
inferra --config inferra.toml reset-weights
```

Implementation: `src/reasoning/scoring.py`, `src/storage/weight_store.py`, `src/storage/calibration_store.py`. Runtime feedback is accepted through the API (`POST /api/incidents/{id}/feedback`) and never routes through the AI layer.

## Validation and contradictions

Strictness for temporal consistency and contradiction penalties is under `[hypothesis_validation]` and `[contradiction_handling]`.

Implementation: `src/reasoning/validation.py`, `src/reasoning/contradiction.py`.

## Deduplication and noise

Dedup windows and severity escalation: `[deduplication]`. Noise classification and registry expiry: `[noise_filter]` plus `[[noise_filter.blocklist]]` / `[[noise_filter.allowlist]]`.

Implementation: `src/normalization/dedup.py`, `src/normalization/noise.py`.

## AI presentation (does not affect ranking)

Explanation timeouts and sanitization affect text only: `[explanation]`, `[explanation.sanitization]`, `[explanation.guardrails]`, and `[ai]` for provider endpoints. See [AI provider](ai_provider.md).
