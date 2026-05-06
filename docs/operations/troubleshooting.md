# Troubleshooting

## `storage verify` / `integrity_check` failures

If `inferra storage verify` reports corruption for `events.db` or `incidents.db`:

1. Confirm no other process has the files open (stop duplicate Inferra instances).
2. Run from the same account that owns the files; locked databases can surface as read failures on some platforms.
3. Restore from backup (see [Upgrade](upgrade.md)); avoid editing SQLite files directly unless you use proven repair tooling and accept data loss.

Implementation: `src/storage/migrations.py` (`integrity_check`).

## Windows Event Log permission errors

Reading specific channels requires membership in **Event Log Readers** or equivalent, and pywin32 must be installed for native APIs (`pip install -e ".[windows]"`).

Symptoms: collector health shows repeated errors, or one-shot `inferra collect-eventlog` raises access denied.

Mitigations:

- Grant Event Log Readers to the service account running Inferra.
- Narrow `[collectors.windows_eventlog].channels` to channels you are allowed to read.

Implementation: `src/collectors/windows_eventlog.py` (optional pywin32 paths guarded).

## Docker socket access

Docker collectors expect a reachable Engine socket (`[collectors.docker].socket`, default `/var/run/docker.sock` on Linux). On Windows, use the named pipe or TCP endpoint exposed by Docker Desktop or your engine configuration.

Symptoms: connection refused or 403 from the Docker HTTP API.

Mitigations:

- Run Inferra with permission to access the socket or named pipe.
- Align `socket` with `DOCKER_HOST` if you use a remote daemon.

Implementation: `src/collectors/docker.py`.

## Kubernetes RBAC

In-cluster collection needs permission to list/watch namespaces, pods, events, and logs as configured. Helm defaults enable RBAC objects under `deploy/helm/inferra/` when `rbac.create` is true.

Symptoms: 403 from the Kubernetes API in collector health or logs.

Mitigations:

- Bind the chart `ServiceAccount` to a `ClusterRole` aligned with your scope (`values.yaml` labels and namespaces).
- For single-namespace installs, reduce `all_namespaces` and tighten rules.

Implementation: `src/collectors/kubernetes.py`, chart templates under `deploy/helm/inferra/templates/`.

## Ollama refuses connections or models

Symptoms: `inferra ai status` shows unavailable; `ai models` lists registry rows but nothing installed; streaming endpoints time out.

Checks:

1. Process reachable at `[ai].base_url` (default `http://127.0.0.1:11434`).
2. Remote endpoints require `[ai].allow_remote = true` and correct network path.
3. Optional bearer token: set an environment variable and point `[ai].token_env` at its name (see [AI provider](ai_provider.md)).
4. Model pull: run `ollama pull <tag>` or `inferra ai pull <tag>` only when Ollama is running; tags must exist on the daemon (`GET /api/tags`).

AI stays optional: disable with `inferra config set ai.enabled false` and rely on template explanations.

Implementation: `src/ai/ollama.py`, `src/ai/service.py`.

## Collector supervisor not reachable

`inferra collectors start` and `inferra collectors stop` call the local HTTP API. If nothing listens on `[server].host`:`[server].port`, commands fail with a connection error.

Start the runtime with `inferra run` or `inferra serve` on that host, or use one-shot `inferra collect-*` commands that do not require a daemon.
