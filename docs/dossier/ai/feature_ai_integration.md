# Inferra features and AI integration map

This document lists **operator-visible features** and how **optional AI** (Ollama, local-first) attaches to each. It reflects the Rust control plane as of the current tree.

## Global rules (all AI)

- **Deterministic core**: collectors, SQLite, incidents, hypotheses, scores, and lifecycle state are **not** mutated by model output (see [AI investigation system](ai_investigation_system.md) and [product vision](../product/product_vision.md)).
- **Evidence bundles** sent to the model are **redacted** when `ai.redact_raw_logs` is true (event bodies become summaries).
- **Grounding**: server-side validation strips `evidence[]` and `citations[]` entries whose IDs are not present in the bundle; results appear under `grounding` in the JSON response.
- **Runtime context for AI**: investigations include **`host_resources`** (CPU count, global CPU %, memory, swap, load average, disk mounts, process count, optional **GPU** via `nvidia-smi` when available) and **`runtime_monitor`** (time series over `monitor_seconds`, configurable interval).
- **Multimodel**: `ai.model` (default), `ai.model_status` (probe for `/api/ai/status` and doctor), `ai.model_investigate` (chat / investigation). Empty secondary fields fall back to `ai.model`.
- **Operator memory**: SQLite table `ai_operator_context`; keys `global`, `incident:{id}`, `service:{id}` merged into bundle field `operator_memory` for investigations.

## API routes and AI

| Route | Purpose | AI |
| --- | --- | --- |
| `GET /api/overview` | Dashboard + quick analysis | Fills `dashboard.health.ai_*` from **status** model probe only. |
| `GET /api/ai/status` | Provider summary | Uses **`model_status`** (or `model`) for Ollama `/api/tags` install check; returns `status_model` / `investigate_model` hints. |
| `GET /api/ai/doctor` | Operator health / warnings | Probes **status** model; warns if **investigate** model differs and is missing. |
| `POST /api/ai/ask` | Scoped question | Full bundle + Ollama chat; **`monitor_seconds`** in JSON body (or default from `ai.investigation_monitor_seconds`). |
| `GET /api/investigate/now` | Overview investigation | Same as ask with empty question; query **`monitor_seconds`**. |
| `GET /api/investigate/incident/{id}` | Incident-scoped | Includes hypotheses (with **rank**), linked events, **`similar_incidents`**; **`monitor_seconds`** query. |
| `GET /api/investigate/service/{id}` | Service-scoped | Service row + recent events + monitor + host resources; **`monitor_seconds`** query. |
| `GET /api/ai/report/{incident_id}` | Stored-style report | Same bundle path as incident investigation; **`monitor_seconds`** query. |
| `POST /api/ai/investigate-stream` | SSE streaming | Events: `meta`, `delta` (Ollama stream tokens), `done` (final JSON same shape as non-stream), `error`. Body: `scope`, `question`, `mode`, `monitor_seconds`. |
| `GET/PUT /api/ai/context` | Operator notes for AI | `GET ?scope=global` · `PUT { "scope", "body" }` — merged into future bundles as `operator_memory`. |
| `GET /api/incidents`, `GET /api/incidents/{id}` | Incident lists / detail | **No AI**; detail exposes persisted `explanation` / `latest_trace` from past AI runs. |
| `GET /api/events`, `GET /api/logs`, `GET /api/services`, … | Evidence and services | **No AI**; outputs feed investigation bundles only when those routes’ data is embedded in a bundle. |
| `GET /api/workspace/*` | Workspace map / mappings | **No AI**; workspace JSON is part of investigation bundles. |
| `GET /api/collectors`, ingest, metrics | Operations | **No AI**. |

## CLI and AI

| Command | AI |
| --- | --- |
| `inferra ai status` | Status model probe. |
| `inferra ai doctor` | Status + investigate model checks. |
| `inferra ai ask …` | POST `/api/ai/ask`; optional **`--monitor-seconds`**. |
| `inferra ai investigate …` | GET investigate endpoints; **`--monitor-seconds`** appends query param. |
| `inferra ai report …` | GET report; optional **`--monitor-seconds`**. |

## Web UI and AI

| Page / component | AI |
| --- | --- |
| **AI Investigator** | Doctor panel; ask + scope; **runtime monitor (s)**; **Ask (stream)** with live token transcript (SSE); incident report; grounding when present; raw bundle/trace in advanced mode. |
| **Overview** | Health line from AI probe only (no narrative). |
| **Incidents / Evidence / Workspace** | No inline AI; data is bundle input for investigator. |

## Configuration keys (AI-related)

| Key | Role |
| --- | --- |
| `ai.enabled` | Master switch. |
| `ai.model` | Default Ollama model name. |
| `ai.model_status` | Optional model for availability probe. |
| `ai.model_investigate` | Optional model for investigation/chat. |
| `ai.investigation_monitor_seconds` | Default wall-clock sampling window (0 = skip timed series; host snapshot still once). |
| `ai.investigation_monitor_interval_ms` | Sample interval inside the window (200–10000). |

## Enhancements already implemented in this pass

- Richer bundles: `host_resources`, `runtime_monitor`, `evidence_digest`, `similar_incidents`, `operator_memory`, overview **event preview** and **latest incident hypotheses**.
- Grounding metadata on responses; hypothesis-order hinting in the system prompt + alignment **warning** heuristic.
- Deterministic fallback **`likely_causes`** from hypothesis descriptions when AI is off.
- SSE **`/api/ai/investigate-stream`** for token deltas + final JSON.
- **`/api/ai/context`** for session memory.
- **Evaluation**: Rust unit test `grounding_removes_unknown_evidence_ids` in `inferra-api`.

## Likely next steps (not done here)

- Teach **more collectors’** structured fields into digest-only bundle sections (Kubernetes, Docker) without sending raw secrets.
- **Stream** mock in tests and UI “live transcript” polish.
- **Embedding**-based similar incidents behind a feature flag (still local).
