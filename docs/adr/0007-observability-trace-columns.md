# ADR 0007: Trace and observability columns on `events`

## Status

Accepted (implemented in Rust `inferra-storage` and mirrored in deprecated Python migrations).

## Context

Inferra stores normalized signals in SQLite `events`. To match OpenTelemetry log semantics and Sentry-class correlation, we need stable **trace** and **span** identifiers plus lightweight **resource** hints without forcing every client to embed them only in JSON `structured_data`.

## Decision

1. **Store `trace_id` and `span_id` as normalized lowercase hex in `TEXT` columns**  
   - `trace_id`: 32 hex characters (128-bit W3C trace id).  
   - `span_id`: 16 hex characters (64-bit span id).  
   - Non-hex or wrong-length values are **dropped at ingest** (not stored in columns); the original payload remains in `structured_data` when applicable.

2. **Add `signal_kind` (default `log`), `deployment_environment`, and `severity_text`**  
   - `signal_kind` is `NOT NULL` with default `log` for backfill and collector rows.  
   - `deployment_environment` and `severity_text` are optional.

3. **Indexing**  
   - Composite index `idx_events_trace_ts(trace_id, timestamp)` for trace-scoped reads.

4. **Dedupe**  
   - Auto-computed fingerprints (empty preset fingerprint) include `::tr:{trace_id}` when present so identical messages on different traces are not collapsed.  
   - App ingest `semantic_fingerprint` appends the same trace suffix when `trace_id` is set.

## Consequences

- Existing databases gain columns via `ensure_column` (Rust) or migration v6 (Python deprecated package).  
- All `NewEventRecord` construction sites must supply the new fields (defaults for non-OTel collectors).  
- Query APIs use `LogsQuery` with configurable retention and optional `trace_id` filter.

## Alternatives considered

- **BLOB fixed-width ids** — smaller on disk but worse ergonomics for debugging and HTTP JSON.  
- **Single JSON column only** — avoids schema churn but prevents cheap indexed trace queries on SQLite.
