# Observability Platform: Logs, Traces, and Incident-Linked Signals

**Status:** design / implementation map (living document)  
**Audience:** contributors implementing Sentry-class (and Inferra-differentiated) observability on top of the existing event pipeline  
**Related docs:** [event_model.md](./event_model.md), [log_normalization_pipeline.md](./log_normalization_pipeline.md), [correlation_engine.md](./correlation_engine.md), [storage_architecture.md](./storage_architecture.md), [runtime_context_builder.md](./runtime_context_builder.md), [incident_lifecycle.md](./incident_lifecycle.md)

---

## 1. Purpose and north star

### 1.1 Problem

Operators debug faster when **logs, errors, metrics, and traces** share stable identifiers and appear in **one workflow**—especially when tied to **incidents, hypotheses, and explanations** (Inferra’s core differentiator). Third-party tools (e.g. Sentry Logs) optimize for generic APM; Inferra should optimize for **local-first, governed, incident-centric investigation**.

### 1.2 Product north star

1. **Correlation-first:** every log line can be located by `trace_id`, time window, `service_id`, incident membership, and high-value business keys (`order_id`, `job_id`, …).
2. **Structured by default:** search and alerts operate on **fields**, not only substring search on `message`.
3. **Same pipeline:** collectors, app ingest, and future OTLP ingest all normalize into one **canonical record** compatible with OpenTelemetry semantics.
4. **Governed volume:** dedupe, noise rules, sampling, retention, and PII scrubbing are **first-class**, not bolted on.
5. **Progressive scale:** SQLite + WAL remains the default **system of record** for self-hosted deployments; optional **analytics sink** supports high-cardinality exploration at SaaS scale.

### 1.3 Non-goals (initial phases)

- Replacing full enterprise SIEM.  
- Perfect parity with every Datadog/Grafana Cloud feature on day one.  
- Storing unbounded high-cardinality raw strings in SQLite indexes (must be allowlisted or offloaded).

---

## 2. Current implementation inventory (code anchors)

Use this table when touching behavior so migrations and APIs stay coherent.


| Concern                 | Location                                                                            | Notes                                                                                                           |
| ----------------------- | ----------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| Event row (write model) | `src/crates/inferra-storage/src/lib.rs` — `NewEventRecord`, `insert_batch_governed` | Includes `structured_data: Option<Value>` JSON                                                                  |
| Events DDL + indexes    | Same crate — `initialize_events_db`                                                 | `events` table; btree indexes on `timestamp`, `service_id`, `severity`, `fingerprint`, etc.                     |
| Log read API (narrow)   | `EventsStore::query_logs`                                                           | Fixed `datetime('now', '-24 hours')`; `message LIKE`; does not return `structured_data`                         |
| HTTP logs endpoint      | `src/crates/inferra-api/src/lib.rs` — `api_logs`                                    | Query params: `limit`, `severity`, `service`, `search`, `source_type`                                           |
| App HTTP ingest         | `src/crates/inferra-collectors/src/lib.rs` — `ingest_app_event`                     | Maps `timestamp`, `service`/`service_id`, `message`, `level`, `tags`, stores **full JSON** in `structured_data` |
| API DTO for lists       | `src/crates/inferra-contracts/src/lib.rs` — `EventRow`                              | Optional `trace_id`, `span_id`, `signal_kind`, etc.; no `structured_data` in list shape                         |
| Retention config        | `src/config/defaults.toml` — `[storage]` `retention_hours`                          | Should drive pruning and **log query windows**, not a hardcoded 24h in SQL                                      |
| Incidents + event links | `inferra-storage` — `incident_events`, `IncidentRecord`                             | Natural join surface for “logs for incident”                                                                    |
| Frontend consumption    | `src/web/frontend/src/pages/*.tsx`, `api.ts`                                        | Today mostly generic `/api/logs` usage                                                                          |


**Gap summary:** the **write path** and **incident graph** are strong; the **public log query model**, **OTel identity fields**, and **indexed attributes** are the main levers for parity with Sentry-class log UX.

---

## 3. Canonical data model

### 3.1 Naming: three layers


| Layer                   | Name                                    | Role                                                                  |
| ----------------------- | --------------------------------------- | --------------------------------------------------------------------- |
| Wire / interop          | **OTLP LogRecord** (+ Resource, Scope)  | Vendor-neutral ingest from OpenTelemetry SDKs, collectors, or proxies |
| Storage (logical)       | `**ObservedSignal`** (see below)        | Inferra’s normalized superset; maps to DB row(s)                      |
| Physical (SQLite today) | `**events` row + optional side tables** | Promoted columns + JSON + EAV/FTS as needed                           |


### 3.2 `ObservedSignal` (logical schema)

This is the target shape **after** normalization. It extends the planning doc `NormalizedEvent` concept with explicit observability fields.

**Identity and time**


| Field                | Type               | Required | Semantics                                                    |
| -------------------- | ------------------ | -------- | ------------------------------------------------------------ |
| `event_id`           | string (UUID)      | yes      | Stable primary key                                           |
| `timestamp`          | RFC3339 / epoch ns | yes      | Event time (not ingest time)                                 |
| `observed_timestamp` | optional           | no       | When collector/SDK observed (OTel `observed_time_unix_nano`) |
| `timestamp_source`   | enum string        | yes      | `parsed` | `client` | `collector` | `inferred`               |


**Resource (where / who)**


| Field                                         | Type            | Notes                                           |
| --------------------------------------------- | --------------- | ----------------------------------------------- |
| `service_id`                                  | string          | Maps from `resource.attributes["service.name"]` |
| `host_id`                                     | string          | Hostname, container short id, etc.              |
| `deployment_environment`                      | optional string | `development` / `staging` / `production`        |
| `telemetry_sdk_name`, `telemetry_sdk_version` | optional        | Provenance                                      |


**Signal classification**


| Field             | Type            | Notes                                                                       |
| ----------------- | --------------- | --------------------------------------------------------------------------- |
| `signal_kind`     | enum            | `log` | `exception` | `metric_point` | `span` | `state_change` | `internal` |
| `severity_number` | int             | OTel 1–24; map UI + legacy 0–4                                              |
| `severity_text`   | optional string | e.g. `INFO`, `WARN`                                                         |


**Trace correlation (critical for Sentry-class UX)**


| Field         | Type                                      | Notes                        |
| ------------- | ----------------------------------------- | ---------------------------- |
| `trace_id`    | 16-byte blob **or** 32-char lowercase hex | W3C trace id                 |
| `span_id`     | 8-byte blob **or** 16-char lowercase hex  | Active span when log emitted |
| `trace_flags` | optional uint8                            | Sampled flag etc.            |


**Content**


| Field                | Type                              | Notes                                                                     |
| -------------------- | --------------------------------- | ------------------------------------------------------------------------- |
| `body`               | string **or** JSON                | Human line; may be template-rendered                                      |
| `body_template`      | optional string                   | Original template for grouping (Sentry-style)                             |
| `body_template_hash` | optional string                   | Stable hash for dedupe/grouping                                           |
| `attributes`         | map string → scalar / nested JSON | OTel attributes; high cardinality allowed in blob, **not** all in indexes |


**Inferra-specific (differentiation)**


| Field                      | Type                | Notes                                                   |
| -------------------------- | ------------------- | ------------------------------------------------------- |
| `correlation_keys`         | map string → string | Business IDs (allowlist keys only in v1)                |
| `incident_id`              | optional string     | If known at ingest (rare); usually resolved server-side |
| `fingerprint`              | string              | Existing governance/dedupe                              |
| `quality`                  | optional string     | Existing `quality` column                               |
| `source_type`, `source_id` | strings             | Collector / `app_http` / `otlp`                         |


**Raw / audit**


| Field             | Type     | Notes                                              |
| ----------------- | -------- | -------------------------------------------------- |
| `structured_data` | JSON     | Full wire payload or OTLP JSON fragment for replay |
| `raw_event_id`    | optional | Link into `raw_events` when applicable             |


### 3.3 OpenTelemetry mapping (logs)

Reference: [OTLP Log data model](https://opentelemetry.io/docs/specs/otel/logs/data-model/).


| OTLP                                | `ObservedSignal`                                                                                             |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `LogRecord.time_unix_nano`          | `timestamp`                                                                                                  |
| `LogRecord.observed_time_unix_nano` | `observed_timestamp`                                                                                         |
| `LogRecord.severity_number`         | `severity_number` (+ map to stored `severity`)                                                               |
| `LogRecord.severity_text`           | `severity_text`                                                                                              |
| `LogRecord.body` (AnyValue)         | `body` / nested in `attributes` if non-string                                                                |
| `LogRecord.attributes`              | `attributes`                                                                                                 |
| `LogRecord.trace_id`                | `trace_id`                                                                                                   |
| `LogRecord.span_id`                 | `span_id`                                                                                                    |
| `LogRecord.flags`                   | `trace_flags`                                                                                                |
| `Resource` + `Scope` attributes     | merge with precedence: resource < scope < log attrs for key collisions (document exact precedence in ingest) |


### 3.4 Severity mapping

Maintain a **single** Rust function used by all ingest paths:

- OTLv1–4 → legacy `severity` 0–4 used today.  
- Store **both** `severity` (compact index) and optional `severity_text` for display.  
- Unknown: default `INFO`, tag `severity_unmapped`.

### 3.5 Trace ID / span ID storage format

**Option A (recommended for SQLite):** `TEXT` columns with **normalized lowercase hex** (32 / 16 chars). Simple to debug, index, and pass to UI. Validate on ingest; reject malformed with 400 + metric counter.

**Option B:** `BLOB` fixed length (16 / 8 bytes). Slightly smaller; more conversion in API.

Pick one project-wide and document in ADR.

---

## 4. Physical storage design (SQLite-first)

### 4.1 `events` table evolution

**Promoted columns (add via `ensure_column` migrations, same pattern as existing schema init):**

- `trace_id TEXT` — nullable, indexed  
- `span_id TEXT` — nullable, indexed  
- `severity_text TEXT` — nullable  
- `deployment_environment TEXT` — nullable, optional index  
- `signal_kind TEXT NOT NULL DEFAULT 'log'` — or reuse `event_type` with explicit int→enum mapping documented in code  
- `observed_timestamp TEXT` — nullable

**Composite indexes (examples):**

- `(trace_id, timestamp)` — trace waterfall + ordered logs  
- `(service_id, timestamp)` — already aligned; keep  
- `(deployment_environment, severity, timestamp)` — env-scoped error triage

**Do not** add indexes on unbounded string attributes inside JSON.

### 4.2 Side table: `event_attributes` (EAV, allowlisted)

Purpose: field queries like `http.status_code = 500` without full JSON scan.

```sql
CREATE TABLE IF NOT EXISTS event_attributes (
  event_id TEXT NOT NULL REFERENCES events(event_id) ON DELETE CASCADE,
  attr_key TEXT NOT NULL,
  attr_value_text TEXT,
  attr_value_num REAL,
  attr_value_int INTEGER,
  PRIMARY KEY (event_id, attr_key)
);
CREATE INDEX IF NOT EXISTS idx_event_attr_key_text
  ON event_attributes(attr_key, attr_value_text);
CREATE INDEX IF NOT EXISTS idx_event_attr_key_num
  ON event_attributes(attr_key, attr_value_num);
```

**Population:** on ingest, a configured **allowlist** copies selected keys from `attributes` JSON into this table. Version the allowlist in config (`[observability.indexed_attributes]`).

**Cardinality policy:** reject or drop indexing for keys exceeding N distinct values per day (soft limit) with counter + log line—prevents SQLite bloat.

### 4.3 Full-text search (FTS5)

**Goal:** fast substring / token search across `message` + selected extracted fields (e.g. `exception.type`, `exception.message`).

Options:

1. **External content FTS5** table mirroring `event_id`, indexed text column built at insert time.
2. **Triggers** on `events` INSERT to update FTS.

**Scope:** cap indexed payload size (e.g. first 8–32 KiB); store truncation flag in `quality` or attributes.

### 4.4 Retention and pruning

- Align `query_logs` time filter with `[storage].retention_hours` (and any future per-tenant override).  
- Ensure background prune job deletes from `event_attributes` and FTS **before or with** event row deletion (CASCADE or explicit batch).  
- Document interaction with `dedup_window`, `fingerprint_seen`, and `raw_events`.

### 4.5 When SQLite is not enough

**Signals:** sustained insert rate beyond safe WAL single-writer limits; need sub-second arbitrary aggregations over billions of rows; multi-region.

**Mitigation (phased):**

1. **Read replica** not typical for SQLite—instead **export sink**.
2. Optional **OTLP / HTTP exporter** after local commit: ship copies to ClickHouse, BigQuery, S3+Trino, Loki, etc.
3. **Query router:** small installs query SQLite; “enterprise” profile queries remote columnar with **incident_id** as join key back to SQLite.

---

## 5. Ingest architecture

### 5.1 Paths today


| Path                                         | Entry                         | Output                                       |
| -------------------------------------------- | ----------------------------- | -------------------------------------------- |
| File / journald / syslog / docker / windows… | Collectors → `NewEventRecord` | `events` + governance                        |
| App JSON                                     | `ingest_app_event`            | `events` + full payload in `structured_data` |


### 5.2 New path: OTLP (logs)

**MVP:** JSON and protobuf OTLP/HTTP log ingest over HTTPS, both normalized into the same `LogRecord` mapping.

**Endpoints (proposal):**

- `POST /v1/logs` — OTLP/HTTP `ExportLogsServiceRequest` body over `application/json`, `application/x-protobuf`, or `application/protobuf`; response `ExportLogsServiceResponse` (`partialSuccess`). **gRPC:** not implemented (`415` for `application/grpc`). See `[observability.otlp]` and Phase 7 in §12.
- Or single-record compatibility: extend existing app ingest with `Content-Type: application/json` schema version field `schema: "otel_log_v1"`

**Validation:**

- Max body size (existing `max_payload_bytes` pattern).  
- Max attributes count / max key length / max value size.  
- Scrubbing hooks (see §8).

### 5.3 Normalization pipeline alignment

Implement OTLP → `ObservedSignal` → `NewEventRecord` inside one module (e.g. `inferra-collectors` or new `inferra-otel` crate) so **all** collectors can reuse it. Tie field extraction rules to [log_normalization_pipeline.md](./log_normalization_pipeline.md) stages (format detection less relevant for OTLP; still apply enrichment + fingerprint).

### 5.4 Fingerprinting for logs

Today app ingest uses `semantic_fingerprint("app", &service_id, "app_http", &message)`.

**Evolve:**

- Prefer fingerprint on `**body_template` + service + signal_kind + stable attrs** (e.g. `exception.type`) when template present.  
- Include **hashed list of sorted attribute keys** (not values) to avoid merging unrelated schemas.  
- Keep governance hooks (`IngestGovernance`, dedup windows) unchanged in spirit.

---

## 6. Query and API design

### 6.1 Problems with current `query_logs`

- Hardcoded **24 hours**.  
- **LIKE** only on `message`.  
- No `trace_id` / attribute filters.  
- Response DTO omits structured fields.

### 6.2 Proposed API surface (versioned)

**Option A:** extend `GET /api/logs` with new query params (backward compatible).  
**Option B:** introduce `GET /api/v2/logs` with stricter semantics and pagination.

Recommended: **v2** for pagination + structured filters; keep v1 as thin wrapper or deprecate slowly.

### 6.2.1 `GET /api/v2/logs`

**Query parameters:**


| Param                          | Meaning                                                                  |
| ------------------------------ | ------------------------------------------------------------------------ |
| `start`, `end`                 | ISO8601 bounds; required or default to retention window tail             |
| `limit`                        | default 100, max 2000 (tune)                                             |
| `cursor`                       | opaque keyset cursor `(timestamp,event_id)`                              |
| `service_id`                   | filter                                                                   |
| `source_type`                  | filter                                                                   |
| `severity_min`, `severity_max` | inclusive                                                                |
| `trace_id`                     | filter                                                                   |
| `signal_kind`                  | filter                                                                   |
| `q`                            | full-text query (if FTS enabled)                                         |
| `attr.`*                       | repeated: `attr.http.status_code=500` (only indexed keys in SQLite mode) |
| `include_payload`              | `0                                                                       |


**Response:**

```json
{
  "items": [ { "...ObservedSignal subset..." } ],
  "next_cursor": "opaque",
  "partial": false,
  "stats": { "returned": 100, "scanned_hint": null }
}
```

### 6.2.2 Incident-scoped logs

`GET /api/incidents/:id/logs`

- Resolve `event_id` list from `incident_events` + optional time expansion from `IncidentRecord.time_range_*`.  
- Merge with `trace_id` filter if `latest_trace` or incident runtime context provides one.  
- Pagination same as v2.

### 6.2.3 Trace assembly (MVP)

`GET /api/traces/{trace_id}`

- Path `trace_id`: non-hex characters are stripped, then the remainder must be exactly **32** lowercase hex digits (W3C trace id); otherwise **400** with `trace_id must be 32 hex characters (W3C trace id)`.
- Query: `limit` (default **500**, clamped **1–2000**), optional `start` / `end` (same retention semantics as v2 logs: omit `start` → implicit lower bound from `storage.retention_hours`).
- Response JSON: `trace_id`, `items` (array of `EventRow`, oldest first: `ORDER BY timestamp ASC, event_id ASC`), `limit`, `retention_hours`, `count`.

Today **items** are log-shaped `events` rows for that `trace_id` only (no dedicated span table yet). **No requirement** for full OpenTelemetry trace proto storage in MVP—**enough** to unify on `trace_id`; later phases can merge span signals when stored.

### 6.3 Contract crate changes

Extend or add types in `inferra-contracts`:

- `ObservabilityLogRow` (or extend `EventRow` with optional fields behind `serde` defaults for back compat).  
- `TraceSummary`, `TraceSpanRow` (MVP structs).

Regenerate any OpenAPI / TS types if the project uses codegen; otherwise update `api.ts` manually.

---

## 7. UI / UX map

### 7.1 Global Log Explorer

- Time range picker bound to retention.  
- Field bar: service, env, severity, trace id.  
- Free-text `q` when FTS on.  
- Row expansion: attributes table + link **Open in incident** if linked.

### 7.2 Incident detail

- Tab: **Timeline** (existing events) + **Logs** sub-mode: filter by `trace_id`, severity, full-text.  
- From hypothesis: “View logs for supporting events” → pre-filtered query.

### 7.3 Trace view (MVP)

- **Shipped (MVP):** frontend route `/traces/:traceId` renders the chronological timeline from `GET /api/traces/{trace_id}` and incident correlated-event cards link directly into it when `trace_id` is present.
- **Shipped (expanded UX slice):** incident rows, service rows, and workspace runtime apps now carry a lightweight `latest_trace_summary` so the Overview, Incidents, Systems, service-detail active-incident surfaces, and Workspace app flow can show the most recent correlated trace and jump directly into `/traces/:traceId` without requiring the incident detail page first.
- **Later:** add simple Gantt / span-duration rendering from `structured_data` or dedicated span columns.

### 7.4 Differentiation vs Sentry copy

Surface **governance** (why lines dropped), **dedupe** windows, and **AI trace** scrubbing summary next to log results (link to `incident_ai_traces` policy).

---

## 8. Security, privacy, compliance

### 8.1 Scrubbing pipeline (ordered)

1. **Key denylist** — strip `password`, `authorization`, `cookie`, credit card patterns, JWT-like tokens.
2. **Regex allowlist** for known safe business keys.
3. **Hash-in-place** for PII identifiers optional (`email` → `sha256:`).
4. **Size cap** per attribute and total JSON.

Apply **before** insert into `events` and **before** any AI bundle builder reads logs.

### 8.2 AuthZ

- Loopback defaults already emphasized in config.  
- For non-loopback: document which roles may hit `/api/v2/logs` and export.

### 8.3 Audit

- Log ingest counts, scrub drops, and rejected malformed trace IDs at INFO for operators.

---

## 9. Performance and SLOs (targets)

Self-hosted single-node SQLite (indicative, tune per hardware):


| Operation                                     | Target                                           |
| --------------------------------------------- | ------------------------------------------------ |
| Recent log tail (indexed, 1h, service filter) | < 100 ms p95                                     |
| Trace id lookup (10k rows in window)          | < 200 ms p95                                     |
| Heavy FTS ad-hoc                              | < 2 s p95 or “async job” response                |
| OTLP batch ingest                             | bounded by WAL + single writer; document max EPS |


Add benchmarks in `inferra-storage` tests with generated rows.

---

## 10. Configuration surface (`inferra.toml`)

New sections (proposal):

```toml
[observability]
enabled = true

[observability.logs]
# Max rows returned per request
max_limit = 2000
# Use FTS
fts_enabled = false

[observability.indexed_attributes]
allow = ["http.status_code", "http.route", "exception.type", "job_id"]

[observability.sampling]
# Head-based: keep 1/N info logs for high-volume services
service_defaults = { "nginx" = { "info" = 10 } }

[observability.export]
# Optional sink for analytics (future)
sink = "none"  # none | otlp_http | clickhouse_http
```

**Implemented (Phase 8 MVP):** see `[observability.export]` in packaged `defaults.toml` — HTTP `url`, `interval_seconds`, `batch_size`, `timeout_seconds`, `backfill_on_start`, `bearer_token` (plain string). This supersedes the illustrative `sink = "none"` enum above for the self-hosted forwarder path; analytics integrations may still add alternate sinks later.

Wire sampling **after** scrubbing; never sample severity ≥ WARN by default (configurable).

---

## 11. Testing strategy


| Layer                                           | Tests                                                                                                                           |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| OTLP → `NewEventRecord` mapping                 | unit tests: `otlp_logs` fixture JSON; API tests `/v1/logs`                                                                      |
| Export forwarder (Phase 8)                      | storage: `events_after_cursor` / `max_event_cursor`; API: `export_sink` JSON builder + retry/split tests + `/api/metrics` lines |
| SQL migrations                                  | integration: open DB, migrate, insert, query                                                                                    |
| `query_logs` / v2 API                           | API tests with temp db                                                                                                          |
| `query_trace_timeline` / `GET /api/traces/{id}` | storage unit test (order); API tests (fixture `trace_id`, 400 on bad id, parity route)                                          |
| Governance + trace fields                       | ensure dedupe does not collapse distinct traces                                                                                 |
| FTS                                             | tokenization + unicode edge cases                                                                                               |
| Frontend                                        | smoke on incident log tab filters                                                                                               |


---

## 12. Phased roadmap (implementation order)

### Phase 0 — Documentation and contracts (this file + small ADR)

Deliverables: ADR for `trace_id` format; JSON schema file `schemas/otel_log_v1.json` (or repo path TBD).

### Phase 1 — Schema + ingest promotion

- Migrate `events` with `trace_id`, `span_id`, `signal_kind`, `deployment_environment`, `severity_text`.  
- App ingest: read `trace_id`, `span_id`, nested `attributes` from JSON and populate columns.  
- Align `query_logs` retention with `storage.retention_hours` instead of hardcoded 24h.

**Acceptance:** ingest payload with W3C ids appears in DB columns; CLI/API can filter by `trace_id`.

### Phase 2 — API v2 + DTO

- Implement `GET /api/v2/logs` with time range + `trace_id` + keyset cursor.  
- Extend contracts for response shape.

**Acceptance:** frontend can render trace column; load test basic pagination.

### Phase 3 — `event_attributes` allowlist

- SQLite `event_attributes` EAV + btree indexes; schema version **7** (FTS adds **8**).  
- `[observability.indexed_attributes].keys` in merged config; `inferra_config::observability_indexed_attribute_keys`; threaded via `IngestGovernance::indexed_attribute_keys`.  
- On successful governed insert, copy scalars from JSON `attributes` (inside `structured_data`) for allowlisted keys only (`attr_value_text` / `attr_value_num` / `attr_value_int`).  
- `LogsQuery` + `push_log_query_filters`: `attr_key` + `attr_value` (numeric values match `attr_value_int` or string form).  
- `GET /api/logs` and `GET /api/v2/logs` accept `attr_key` / `attr_value`; incident logs route inherits via `LogsQuery`.  
- `prune_expired` deletes matching `event_attributes` rows before deleting aged events.

**Acceptance:** filter `http.status_code=503` (via `attr_key` / `attr_value`) returns only matching rows under a small fixture.

### Phase 4 — FTS5 (optional flag)

- SQLite `events_fts` FTS5 (`event_id` UNINDEXED + `message`, `unicode61`); triggers on `events` keep rows in sync; first 8192 chars of `message` indexed.  
- Schema version **8**; migration backfills from `events`.  
- `[observability.fts].enabled` (default `false`); `inferra_config::observability_logs_fts_enabled`.  
- `LogsQuery.fts_query` + `log_fts_enabled`: when enabled, `q=` uses sanitized `MATCH`; when disabled, `q=` uses `LIKE` on `message`. `search=` always uses `LIKE` (v2 splits `search` vs `q`).  
- `GET /api/v2/logs` response includes `log_fts_enabled`; incident logs honor the same `LogsQuery` fields.

**Acceptance:** with FTS enabled, `q=` token search hits only matching rows; with FTS off, `q=` still works via `LIKE`.

### Phase 5 — Incident logs endpoint

- `GET /api/incidents/{incident_id}/logs`: when `incident_events` lists ids, `EventsStore::query_logs_for_event_ids` (no implicit retention lower bound unless `start=`); otherwise `query_logs` with `primary_service` as `service_id`. Query params: `limit`, `severity`, `search`, `q` (FTS or LIKE per `[observability.fts].enabled`), `trace_id`, `start` / `end`, `cursor_timestamp` + `cursor_event_id`, `source_type`, `attr_key` / `attr_value`.

**Acceptance:** incident page can fetch related logs in one round-trip with pagination.

### Phase 6 — Trace MVP endpoint

- **Shipped:** `EventsStore::query_trace_timeline` + `GET /api/traces/{trace_id}` (chronological `EventRow` list for one trace; validation + retention-aligned window as above).
- **Shipped (follow-on UX):** `latest_trace_summary` is projected onto incident/service list rows from stored event evidence so overview and list screens can surface trace-aware badges and direct navigation outside incident detail.  
- **Later:** ingest minimal span records OR derive span-like rows from logs with duration attributes; extend response with span-specific rows or a merged timeline beyond log lines only.

### Phase 7 — OTLP ingest (batch)

- **Shipped (expanded MVP):** `POST /v1/logs` accepts OpenTelemetry `ExportLogsServiceRequest` over `Content-Type: application/json`, `application/x-protobuf`, or `application/protobuf`; protobuf is decoded and normalized into the same `inferra-collectors` `otlp_logs` JSON mapping so each `LogRecord` still lands as `NewEventRecord` (`source_type` `otlp_json`, `source_id` `otlp-http`). Trace/span id handling stays aligned with the existing JSON path (32/16 hex storage; Base64 still accepted on JSON input). Config remains `[observability.otlp].enabled` (default `false`), `max_logs_per_request`, `max_payload_bytes`; **same** `Authorization: Bearer …` as `[collectors.app].shared_token` when non-empty. JSON response includes OTLP `partialSuccess` (`rejectedLogRecords` counts parse/limit skips plus governance suppressions). **gRPC:** still **415 Unsupported Media Type** with JSON error body.
- **Later:** OTLP gRPC, dedicated auth tokens, token bucket rate limits, finer-grained `partialSuccess` reasons.

### Phase 8 — Export sink

- **Shipped (hardened backend slice):** `[observability.export]` — background task (Rust API process) reads new `events` rows after a persisted cursor (`data_dir/observability_export_cursor.json`), batches them as OTLP/HTTP JSON `ExportLogsServiceRequest`, and `POST`s to `url`.
  - **Retry/backoff:** retryable transport / `408` / `429` / `5xx` failures back off with `[observability.export].max_retries`, `retry_initial_seconds`, `retry_max_seconds`.
  - **Batch isolation:** OTLP `partialSuccess` rejections and sink-side validation errors (`400` / `413` / `422`) split the batch recursively so one poison row does not stall later exports.
  - **Cursor semantics:** successful sub-batches advance the cursor immediately; a single isolated poison row is dropped from export only after being narrowed to one event, then the cursor advances past it so later rows continue flowing. The event still remains in Inferra’s local store.
  - **Backpressure:** at most one export HTTP flow in flight (`EXPORT_BUSY`; skipped ticks increment `inferra_observability_export_ticks_skipped_busy_total`).
  - **Metrics:** Prometheus counters on `/api/metrics` for batches success/fail, events forwarded, events dropped, retries, split batches, downstream partial rejections, and busy ticks.
  - **No backfill by default:** first run seeds the cursor to the newest row so history is not re-sent (`backfill_on_start = true` to export from empty cursor).
  - **Bearer:** optional `bearer_token` in config.
  - **Do not** point `url` at the same Inferra instance’s `/v1/logs` (ingest loop).
- **Later:** protobuf export, compression, per-sink auth via env, adaptive batching, dead-letter persistence for dropped poison rows.

---

## 13. Open decisions (track in ADR or `open_decisions.md`)

1. **TEXT vs BLOB** for trace/span ids.
2. **Extend `EventRow` vs new DTO** for API compatibility.
3. **Single `events` table for spans** vs dedicated `spans` table (normalization vs simplicity).
4. **Default sampling** policy for noisy services.
5. **Cross-db joins** if incidents move to separate connection—today `events` + `incident_events` are separate DB files; document join strategy (application-level join only).
6. **Clock skew** handling for OTLP nanosecond timestamps vs SQLite `TEXT` ordering—may require integer `timestamp_ns` column for correct sort.

---

## 14. Appendix A — W3C Trace Context propagation

For app developers using Inferra without OTel SDK:

- **Shipped for app JSON ingest:** top-level `traceparent` or `headers.traceparent` is parsed as W3C Trace Context and used as a fallback source for `trace_id` / `span_id` when explicit fields are absent.  
- **Shipped for background jobs / async work:** app JSON ingest also promotes `attributes["inferra.trace_id"]` and optional `attributes["inferra.span_id"]` into canonical `trace_id` / `span_id` when explicit fields are absent.

---

## 15. Appendix B — Example promoted ingest payload (app JSON v2)

```json
{
  "schema": "inferra.observed_signal.v1",
  "timestamp": "2026-05-14T12:00:00.123Z",
  "service_id": "checkout-api",
  "signal_kind": "log",
  "severity_text": "ERROR",
  "body": "payment failed: insufficient funds",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "span_id": "00f067aa0ba902b7",
  "deployment_environment": "production",
  "attributes": {
    "http.route": "/v1/pay",
    "http.status_code": 402,
    "payment_id": "pay_123"
  }
}
```

---

## 16. Appendix C — File / module checklist (implementation tick list)


| Task                            | Likely files                                                                                             |
| ------------------------------- | -------------------------------------------------------------------------------------------------------- |
| DDL + migrations                | `inferra-storage/src/lib.rs` (`initialize_events_db`, `event_attributes`, `events_fts`, `ensure_column`) |
| `NewEventRecord` + insert       | `inferra-storage`                                                                                        |
| App ingest mapping              | `inferra-collectors/src/lib.rs`                                                                          |
| OTLP → `NewEventRecord` mapping | `inferra-collectors/src/otlp_logs.rs` + `CollectorRuntime::ingest_otlp_logs_json`; API `POST /v1/logs`   |
| REST routes                     | `inferra-api/src/lib.rs`                                                                                 |
| Trace timeline MVP              | `inferra-api` (`GET /api/traces/{trace_id}`), `inferra-storage` (`query_trace_timeline`)                 |
| Types                           | `inferra-contracts/src/lib.rs`                                                                           |
| Config                          | `inferra-config`, `defaults.toml`, `inferra.toml`                                                        |
| UI                              | `src/web/frontend/src/pages/*`, `api.ts`                                                                 |
| OTLP export forwarder (Phase 8) | `inferra-api/src/export_sink.rs`, `[observability.export]`, cursor file under `data_dir`                 |
| Docs                            | this file + ADR                                                                                          |


---

## 17. Revision history


| Date       | Author | Notes                                                                                                                                                                                                                                                            |
| ---------- | ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2026-05-14 | —      | Trace UI MVP: frontend route `/traces/:traceId` now renders the trace timeline and incident correlated events link to it when `trace_id` is present.                                                                                                             |
| 2026-05-14 | —      | App ingest enhancement: `attributes["inferra.trace_id"]` and optional `attributes["inferra.span_id"]` are promoted for background-job correlation when explicit fields are absent.                                                                               |
| 2026-05-14 | —      | App ingest enhancement: W3C `traceparent` fallback (`traceparent` / `headers.traceparent`) now derives `trace_id` / `span_id` when explicit fields are absent.                                                                                                   |
| 2026-05-16 | —      | Phase 8 hardening: `[observability.export]` now retries retryable failures, recursively splits sink-rejected batches, advances the cursor per successful sub-batch, drops isolated poison rows with metrics, and exposes retry/split/partial-rejection counters. |
| 2026-05-14 | —      | Phase 8 (MVP): `[observability.export]` OTLP JSON forwarder — cursor file, `EventsStore::events_after_cursor` / `max_event_cursor`, backpressure + `/api/metrics` counters; **do not** point `url` at same host `/v1/logs`.                                      |
| 2026-05-16 | —      | Phase 7 follow-up: `POST /v1/logs` now also accepts OTLP HTTP protobuf (`application/x-protobuf` / `application/protobuf`) by decoding into the existing `otlp_logs` JSON mapping; gRPC remains **415**.                                                             |
| 2026-05-17 | —      | Workspace trace UX follow-up: `runtime_apps[]` in `/api/workspace/map` now projects `latest_trace_summary` via workspace service mappings / app-name fallbacks, and the Workspace app logs endpoint reuses the same mapping-aware event lookup.                      |
| 2026-05-14 | —      | Phase 6: `GET /api/traces/{trace_id}` — `EventsStore::query_trace_timeline` (oldest-first `EventRow` list), W3C id validation, `limit`/`start`/`end` + retention; tests in `inferra-storage` + `inferra-api`.                                                    |
| 2026-05-14 | —      | Phase 4: `events_fts` FTS5 + triggers (schema v8), `[observability.fts].enabled`, `LogsQuery.fts_query` / `log_fts_enabled`, v2 `q` vs `search`, `log_fts_enabled` in JSON, incident logs aligned.                                                               |
| 2026-05-14 | —      | Phase 3: `event_attributes` EAV (schema v7), `[observability.indexed_attributes].keys`, ingest copy from `structured_data.attributes`, `LogsQuery.attr_key`/`attr_value` + `/api/logs` & `/api/v2/logs` & incident logs, `prune_expired` cleanup.                |
| 2026-05-14 | —      | Phase 5: `GET /api/incidents/{incident_id}/logs` — uses `query_logs_for_event_ids` when `incident_events` is non-empty (no implicit retention floor); otherwise `query_logs` with `primary_service` as `service_id`. Keyset cursor params match v2 logs.         |


