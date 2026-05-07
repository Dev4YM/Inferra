# Rust Runtime Contracts

This document freezes the frontend-facing contracts that the Rust runtime must
honor as the active Inferra implementation. Python under `deprecated/` is
archival or compatibility-only and is not part of the live `src/` runtime.

## Public HTTP Contracts

The React control plane depends on the following stable JSON routes:

- `GET /api/version`
- `GET /api/health`
- `GET /api/config`
- `PUT /api/config`
- `GET /api/overview`
- `GET /api/incidents`
- `GET /api/incidents/{incident_id}`
- `GET /api/services`
- `GET /api/services/{service_id}`
- `GET /api/collectors`
- `POST /api/collectors/start`
- `POST /api/collectors/stop`
- `GET /api/workspace/map`
- `GET /api/ai/status`
- `GET /api/ai/doctor`
- `POST /api/ai/ask`
- `GET /api/investigate/incident/{incident_id}`
- `GET /api/investigate/service/{service_id}`
- `GET /api/events`
- `GET /api/events/{event_id}`
- `GET /api/anomaly/{service_id}/status`
- `GET /api/logs`
- `GET /api/topology`

The Rust Axum runtime is now responsible for the public `/api/*` surface. There
is no supported Rust-to-Python proxy path in the active product contract.

## Configuration Semantics

`PUT /api/config` follows the current Python API contract:

- accepts either `{ "config": { ... } }` or a top-level config object
- deep-merges nested objects
- rejects runtime changes to `storage.data_dir`
- returns `{ "config": ..., "applied": true }`

## Overview Response

The Rust runtime mirrors the frontend contract defined in
`src/web/frontend/src/api.ts`:

- `quick_analysis`
- `dashboard`
- `runtime`
- `workspace_projects`
- `experience`

Field names and nesting must remain stable even as internal heuristics,
correlation quality, and operator audit payloads continue to improve.

## Native Investigation Contract

Investigation execution now stays inside the Rust runtime. Public callers keep
the same operator-facing response schema, but no loopback worker process or
internal Python service is required to satisfy it.
