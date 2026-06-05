# Workspace Integration

Inferra discovers local projects from configured workspace roots and maps runtime
processes back to source directories where possible.

## Inspect Scope

`GET /api/workspace/inspect?path=...` only accepts paths under the configured
workspace roots. Relative roots are resolved from the directory containing
`inferra.toml`; when no roots are configured, the config directory is the default
root.

With `workspace.redact_env_files = true` (the default), `.env*` markers and
directory entries are omitted from inspection responses. Inferra reports project
structure and known build/runtime markers, not secret file contents.

## App Manifest Integration

Inferra can infer many local apps from process managers, runtime commands, project manifests, and framework files. For the strongest signal, add an app-owned manifest at:

```text
.inferra/app.toml
```

This file is read by the workspace scanner and attached to the workspace app context that the UI and AI monitor use. Inferra never reads `.env`, `.env.local`, or `.env.*` as part of workspace structure discovery.

## Example

```toml
[app]
name = "billing-api"
runtime = "nodejs"
framework = "fastify"
process_kind = "server"
url = "http://127.0.0.1:3001"

[heartbeat]
path = "/health"

[[logs]]
label = "Application log"
path = "logs/app.log"
kind = "file"

[[logs]]
label = "Error log"
path = "logs/error.log"
kind = "file"

[[endpoints]]
url = "http://127.0.0.1:3001/metrics"
source = "app_manifest"
confidence = 0.95
```

## Supported Fields

`[app]` can define `name`, `runtime`, `framework`, `process_kind`, `url`, or `app_url`.

`[heartbeat]` or `[health]` can define `url` or `path`. A `path` is resolved against the app URL when available.

`[[logs]]` can define `label`, `path`, `kind`, `command`, or `stream`. Relative file paths are resolved from the project root. File logs are tailed with a bounded read and sent to the Workspace app details view. PM2 apps use PM2 metadata and PM2 log files first, then fall back to a bounded `pm2 logs <app> --nostream --lines N` read. Node apps also auto-discover npm cache debug logs, common project log files, and Next.js trace output when present.

`[[endpoints]]` can define a full `url`, or `host` plus `port`. These become app endpoints in the UI and AI context.

## Detection Hierarchy

Inferra ranks workspace app evidence in this order:

1. Explicit `.inferra/app.toml` metadata.
2. Process manager metadata such as PM2 app name, PID, status, logs, and environment-derived URL.
3. Runtime process metadata from the OS process table.
4. Project manifests such as `package.json`, `pyproject.toml`, `Cargo.toml`, `requirements*.txt`, and framework files.
5. Conservative framework defaults such as common development ports.

The UI reads stored scanner snapshots and app-specific API views. Scanner intervals are bounded by the runtime configuration so visiting a page does not force a full scan every time.

AI investigations are persisted by scope. Incident and service pages load the latest saved generation by default; explicit re-runs create and store a new generation.
