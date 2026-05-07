# Troubleshooting

## Database bootstrap or integrity failures

If `inferra init-db` fails or the runtime reports corruption for `events.db` or `incidents.db`:

1. Confirm no other process has the files open (stop duplicate Inferra instances).
2. Run from the same account that owns the files; locked databases can surface as read failures on some platforms.
3. Restore from backup (see [Upgrade](upgrade.md)); avoid editing SQLite files directly unless you use proven repair tooling and accept data loss.

The active runtime uses `src/crates/inferra-storage/` for schema bootstrap and write paths.

## Windows Event Log permission errors

Reading specific channels requires membership in **Event Log Readers** or equivalent.

Symptoms: collector health shows repeated errors, `inferra collectors status` shows `error`, or the Windows service/runtime log records access denied.

Mitigations:

- Grant Event Log Readers to the service account running Inferra.
- Narrow `[collectors.windows_eventlog].channels` to channels you are allowed to read.

Implementation: `src/crates/inferra-collectors/`.

## Docker socket access

Docker collectors expect a reachable Engine socket (`[collectors.docker].socket`, default `/var/run/docker.sock` on Linux). On Windows, use the named pipe or TCP endpoint exposed by Docker Desktop or your engine configuration.

Symptoms: connection refused or 403 from the Docker HTTP API.

Mitigations:

- Run Inferra with permission to access the socket or named pipe.
- Align `socket` with `DOCKER_HOST` if you use a remote daemon.

Implementation: `src/crates/inferra-collectors/`.

## Kubernetes RBAC

In-cluster collection needs permission to list/watch namespaces, pods, events, and logs as configured. Helm defaults enable RBAC objects under `deploy/helm/inferra/` when `rbac.create` is true.

Symptoms: 403 from the Kubernetes API in collector health or logs.

Mitigations:

- Bind the chart `ServiceAccount` to a `ClusterRole` aligned with your scope (`values.yaml` labels and namespaces).
- For single-namespace installs, reduce `all_namespaces` and tighten rules.

Implementation: `src/crates/inferra-collectors/`, chart templates under `deploy/helm/inferra/templates/`.

## Ollama refuses connections or models

Symptoms: `inferra ai status` shows unavailable; `inferra ai doctor` reports provider or model issues; investigation endpoints fall back to deterministic output.

Checks:

1. Process reachable at `[ai].base_url` (default `http://127.0.0.1:11434`).
2. Remote endpoints require `[ai].allow_remote = true` and correct network path.
3. Optional bearer token: set an environment variable and point `[ai].token_env` at its name (see [AI provider](ai_provider.md)).
4. Model pull: run `ollama pull <tag>` only when Ollama is running; tags must exist on the daemon.

AI stays optional: disable with `inferra config set ai.enabled false` and rely on template explanations.

Implementation: `src/crates/inferra-api/` native Ollama probe/investigation flow.

## Collector supervisor not reachable

`inferra collectors start` and `inferra collectors stop` call the local HTTP API. If nothing listens on `[server].host`:`[server].port`, commands fail with a connection error.

Start the runtime with `inferra serve` on that host, then retry `inferra collectors start|stop|status`. For packaged installs, `inferra service repair` is the fastest native readiness check on Windows and `curl http://127.0.0.1:7433/api/health` is the equivalent cross-platform probe.
