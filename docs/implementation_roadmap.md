# Inferra Implementation Roadmap

This file is the living checklist for building Inferra in deep, complete slices while keeping the repository ready for GitHub.

## Current Slice: Collector Coverage + Operations

- [x] Canonicalize storage behind package-level protocols and split SQLite/JSON implementations.
- [x] Keep the flattened `src/` layout with no nested `inferra/` package.
- [x] Add an AI subsystem for provider logic, model registry, prompting, redaction, and explanation service.
- [x] Add Ollama local/remote HTTP support with optional bearer token from an environment variable.
- [x] Add verified Gemma 3 and Gemma 4 model registry from the official Ollama tag list.
- [x] Upgrade Ollama provider to async aiohttp, streaming chat/generate, streamed pull progress, and health probes.
- [x] Add first-run setup and AI control commands to the CLI.
- [x] Make `inferra.toml` the canonical typed config surface with round-trip CLI editing and startup checks.
- [x] Expand the CLI into a grouped dispatcher with `serve`, `init-db`, completion generation, API-backed collector control, one-shot `collect-*` commands, and JSON output for every command.
- [x] Add web API endpoints for AI status, model registry, incident explanation, and incident chat.
- [x] Full REST surface from architecture plan: `GET/PUT /api/config`, `GET /api/version`, optional `GET /api/metrics` (Prometheus), `GET /api/ai/trace/{id}`, `POST` feedback, anomaly and NL search routes; localhost loopback guard plus optional bearer auth from `[server] auth_token_env`; CORS from config; per-IP HTTP and per-connection WebSocket token buckets for chat/explain; `WebSocket /ws` with incident lifecycle, `event_count`, `collector_health`, `explanation_ready`, `baseline_status`, and `ai_stream_token` messages plus client control messages (`subscribe_incident`, `unsubscribe_incident`, `resolve_incident`, `chat_send`, `explain_request`).
- [x] Add tests for config, registry, redaction, Ollama provider behavior, CLI setup, and AI web endpoints.

## Next Slices

- [x] Collector depth: Windows Event Log, service/process state, performance counters, Linux journald/syslog, Docker, Kubernetes events, and app HTTP ingest.
  - [x] Portable process snapshot collector with CPU/memory threshold crossings and metric ringbuffer writes.
  - [x] Windows service collector emitting state-change events only.
  - [x] Windows Event Log bookmark persistence and channel metadata enrichment.
  - [x] Linux journald/syslog collectors with cursor or inode rotation tracking.
  - [x] File collector glob, multiline, and filename-derived service identity support.
  - [x] Docker event and log collection.
  - [x] Kubernetes event/workload collectors with restart and OOM detection.
  - [x] App HTTP collector in mounted and standalone modes.
- [x] Normalization depth: parser coverage, enrichment, validation, and stable fingerprinting.
  - [x] Parser package for JSON lines, syslog RFC 3164/5424, Windows Event Log, Kubernetes, Docker, and generic text.
  - [x] Deterministic host/service/process enrichment with config-based overrides and mappings.
  - [x] Validation for timestamp bounds, message truncation, and structured payload compaction.
  - [x] Stable template-based fingerprinting plus fixture-driven normalization and performance tests.
- [x] Reasoning depth: anomaly scoring, causal graph traversal, contradiction handling, confidence calibration.
  - [x] Signal detectors, inference graph builder (DAG with strategy edges and budgets), hypothesis composer with custom rules merge, `HypothesisEngine` replacing the legacy simple engine, CLI `reason-incident`, and fixture-driven coverage plus 100-run determinism tests.
  - [x] Deterministic anomaly scoring for severity, resource tags, host metrics, and process metrics.
  - [x] Baseline-backed anomaly signals: per-(service, fingerprint) EMA, closed-bucket metrics, spike z-score, sustained mean, heartbeat absence, JSON persistence under `data_dir/baselines/`, and `GET /api/anomaly/{service}/status`.
  - [x] Runtime context snapshot builder (`runtime.context`) for read-only host, process, disk, and Docker summaries fed to correlation-style consumers.
  - [x] Causal graph traversal for dependency proximity and root-cause event selection.
  - [x] Weighted hypothesis scoring (six components from `reasoning/scoring.py`), `ContradictionHandler` rules, validation gates, per-bucket calibration with staleness-aware labels, bounded `WeightStore` feedback updates, `POST /api/incidents/{id}/feedback`, `inferra reset-weights`, and `inferra calibration show`.
- [x] Web UI: multi-view console (dashboard, incidents workbench, virtualized logs, services, per-collector controls, settings), vendored Tailwind build (`scripts/build-ui.sh`), native ES modules under `src/web/static/js/`, dark theme + reduced-motion styling, live `/ws` updates (including `ai_stream_token` on Chat), optional Playwright smoke (`pip install -e ".[ui]"` then `pytest`).
- [x] Packaging: Windows service/installer, Linux systemd unit, Docker image, Helm chart, macOS launch agent.
  - [x] Windows `windows_service` helper (`install` / `remove` / `start` / `stop` / `debug`) with `service_runtime.json` for `--config` / `--data-dir`, pywin32 subprocess guard, and `deploy/windows/install-service.ps1` (ProgramData layout, ACLs, optional `-AllowFirewall`).
  - [x] PyInstaller `deploy/windows/inferra.spec` + `deploy/windows/pyi_entry.py` one-file `inferra.exe`.
  - [x] Linux `deploy/systemd/inferra.service` (`DynamicUser`, `ProtectSystem=strict`, `StateDirectory`) and `deploy/linux/fpm-package.sh` (.deb / .rpm via fpm).
  - [x] Root `Dockerfile` (python:3.12-slim, non-root), `compose.yaml`, `.dockerignore`.
  - [x] Helm chart: `values.yaml` collectors.kubernetes, ServiceAccount, least-privilege ClusterRole/RBAC, optional ServiceMonitor.
  - [x] macOS `deploy/macos/install.sh` and `uninstall.sh` for `com.inferra.agent`.
  - [x] CI matrix (Windows, Linux, macOS; Python 3.11/3.12), Helm `helm template` gate, `release.yml` (wheel/sdist, SBOM, Helm tgz, Windows exe, GHCR multi-arch image, cosign keyless + documented signtool path).
- [x] Documentation: install (all target platforms), AI provider, collectors, tuning, upgrade, troubleshooting, CI and release operator notes; ADRs 0001-0006; planning index; `mkdocs build` via `python -m pip install -e ".[docs]"`.
  - [x] Install guide.
  - [x] AI provider setup guide.
  - [x] Collector command guide.
  - [x] Architecture decision records.
- [x] Service supervision and health.
  - [x] Long-running collector supervisor with retry/backoff.
  - [x] `run-collectors` CLI command.
  - [x] Collector health API and dashboard panel.
  - [x] Configurable service mode presets.
- [x] Operator controls.
  - [x] Start/stop supervised collectors from API and web UI.
  - [x] CLI collector status command.

- [x] SQLite storage layer: production-grade multi-day continuous operation.
  - [x] Migration framework with ordered Migration objects, version-aware upgrades, and downgrade refusal.
  - [x] Full events.db schema: events, raw_events, collector_state, fingerprint_seen, dedup_window, schema_version.
  - [x] Full incidents.db schema: incidents, incident_events, incident_clusters, hypotheses, explanations, feedback, incident_state_log, schema_version.
  - [x] All read-path indexes: service_id+timestamp, severity+timestamp, fingerprint, incident_id composites.
  - [x] WAL tuning: journal_mode=WAL, synchronous=NORMAL, temp_store=MEMORY, mmap_size (configurable), busy_timeout=5000, auto_vacuum=INCREMENTAL.
  - [x] Connection pool: single writer + per-thread readers, BEGIN IMMEDIATE transaction context manager.
  - [x] Retention: event pruning daemon + shutdown prune; incident archival to separate database.
  - [x] Integrity: startup PRAGMA integrity_check, CLI `inferra storage verify/vacuum/backup`.
  - [x] CLI: `inferra init-db` idempotently creates or upgrades schemas.
  - [x] No raw SQL outside src/storage/.
  - [x] Tests: migration from empty to current, v1->current, v2->current fixture, retention, integrity check failure, concurrent 4-thread insert.

- [x] Event deduplication and noise filtering.
  - [x] DedupTracker: sliding window by fingerprint with count, first/last timestamp.
  - [x] Severity escalation splits: INFO->WARN->ERROR starts a new window.
  - [x] Periodic SUMMARY events every configurable interval with count and sample event id.
  - [x] Bounded memory: max_tracked_fingerprints with LRU eviction via OrderedDict.
  - [x] NoiseFilter: static blocklist with severity_max guard.
  - [x] Static allowlist overrides blocklist and adaptive filter.
  - [x] Adaptive filter: per-fingerprint rate and coefficient-of-variation, demote stable high-rate to routine.
  - [x] Severity >= always_keep_severity never demoted (ERROR/CRITICAL protected).
  - [x] Noise registry: persist learned routines to disk, expire after registry_expiry_days.
  - [x] Integration: InferraRuntime.ingest_raw pipeline: normalize -> dedup.check -> noise_filter.annotate -> should_store -> event_store.
  - [x] Dashboard /api/dashboard exposes dedup suppressed counts and noise routine counts.
  - [x] Determinism tests: same fixture produces identical stored event ids across 100 runs.
  - [x] 1000 identical INFO events collapse to first + periodic summaries.
  - [x] ERROR never demoted even under adaptive filter.
  - [x] Allowlist always wins over blocklist.

- [x] Correlation engine and incident lifecycle: temporal half-life decay, topology-aware edges (dependency_propagation, shared_fate), shared-service edges, cascade detection, persisted clusters with correlation edges and reason codes; `IncidentLifecycleManager` replaces the ad-hoc analyzer with OPEN through INVESTIGATING/EXPLAINED/RESOLVED/MERGED, merge candidates by overlapping services and time proximity, staleness to RESOLVED, auto-split on oversized clusters; deterministic `primary_service` (severity weighted by topology centrality) and sorted `affected_services`; `GET /api/incidents/{id}/state-log`; cascade and no-topology fixtures plus staleness and 100-run incident id determinism tests.

- [x] Explanation layer (no-LLM path): structured template payload with stable `explanation_id`, `sanitize_plaintext` + `SanitizationReport`, guardrails (services, timestamps, causal phrases, overconfidence), SQLite explanation rows keyed by `hypotheses_hash` + `events_hash_head`, and API cache hits before regenerating.
- [x] AI presentation layer: per-surface prompt contracts with Pydantic JSON validation and schema-fallback to template explanations; streaming Ollama aggregation; persisted AI audit traces and chat history in `incidents.db`; natural-language event search via `GET /api/search/natural` with configurable confidence threshold and `422` suggestions when confidence is low.

- [x] Resilience: chaos tests (`pytest -m chaos`), degradation surfaced on `/api/health` and dashboard, AI template fallback on provider/stream failures, collector retry jitter and emit backpressure jitter, CSP on HTTP responses, v0.1.x events schema upgrade regression test, threat model and release checklist documentation.

## Operating Rules

- CI gates documented in `docs/operations/ci.md`: matrix-marked pytest on Ubuntu/macOS/Windows (excluding `pytest.mark.chaos` from the default matrix), dedicated **chaos** job on Linux, perf job with `perf_report.json`, coverage thresholds on reasoning/analysis/storage/normalization/ai via `[tool.coverage.report]` in `pyproject.toml`.
- Determinism hashes live under `tests/determinism/`; performance budgets under `tests/perf/` (`pytest.mark.perf`).
- AI is guided only: it explains, summarizes, chats, and suggests checks.
- Deterministic evidence, scoring, and incident ranking remain authoritative.
- No model weight is pulled automatically without explicit CLI confirmation or `--yes --pull`.
- Remote AI means an Ollama-compatible HTTP server configured by base URL and optional token environment variable.
