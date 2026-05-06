# Threat model (local-first)

This document describes the security posture of Inferra as shipped: a local operator runs the agent, SQLite stores evidence on disk, and an optional Ollama-compatible endpoint provides presentation-only explanations.

## Trust boundaries

- **Operator host**: trusted. Configuration files, TLS material for upstream proxies, and the optional `auth_token_env` bearer secret live here.
- **Inferra process**: trusted. It reads configured sources (logs, metrics, Kubernetes API, Docker socket) and writes only under `storage.data_dir` (events, incidents, JSON sidecars).
- **Observed systems**: untrusted inputs (log lines, API payloads). Inferra treats them as data, not code; parsers enforce size limits and normalization budgets.
- **Browser session**: semi-trusted. The bundled UI is static assets plus same-origin API/WebSocket calls.

## Network exposure

- **Default**: HTTP API binds to loopback (`127.0.0.1`) and `LocalSecurityMiddleware` rejects non-loopback clients when `require_loopback` is true.
- **Non-local binding**: binding to `0.0.0.0` or a routable interface is intentionally awkward: set `server.require_loopback = false` **and** configure `server.auth_token_env` to the name of an environment variable holding a bearer token. Requests without `Authorization: Bearer <token>` receive `401`. If the variable is unset, the server responds `503` so the instance is not accidentally wide open.
- **Ollama**: optional. With `ai.allow_remote = false`, only loopback-like hosts are accepted for the configured `base_url`. Remote Ollama requires explicit opt-in.

## Redaction (AI paths)

- Prompt construction uses structured incident and event payloads; raw log lines are optional and gated by `ai.redact_raw_logs`.
- Redactors strip common secret patterns (tokens, PEM blocks, AWS-style keys) before any model call. Coverage is pattern-based, not semantic: a novel secret format may leak until a rule is added.
- **Limits**: redaction does not prove absence of sensitive data; it reduces obvious leakage. Operators should keep models local and restrict who can call the API when auth is enabled.

## Web UI hardening

- **CSP**: `Content-Security-Policy` is set on HTTP responses with `default-src 'self'`, `script-src 'self'`, and `style-src 'self' 'unsafe-inline'` (Tailwind build emits utility classes). No inline script blocks are used after the UI build; entrypoint is `script type="module"` from `/static/js/main.js`.
- **Frame embedding**: `frame-ancestors 'none'` reduces clickjacking for operators who open the UI in a browser.

## Integrity and availability

- SQLite uses WAL mode with `BEGIN IMMEDIATE` for writer transactions. A crash or `SIGKILL` during an open transaction rolls back that transaction; previously committed rows remain readable after restart (`PRAGMA integrity_check` is supported via CLI storage commands).
- Disk-full and read-only SQLite errors mark the runtime **degraded**, pause supervised collectors on severe storage faults, and surface state via `/api/health` and `/api/dashboard` without mutating observed systems.

## Out of scope

- Inferra is not a secret manager, SIEM, or zero-trust service mesh.
- It does not authenticate end users beyond the optional static bearer token for the API.
