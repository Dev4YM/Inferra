# Changelog

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
