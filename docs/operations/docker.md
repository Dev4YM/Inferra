# Docker Deployment

The root `compose.yaml` is a local workstation profile. It publishes Inferra on
`127.0.0.1:7433`, stores data in the `inferra-data` volume, and uses
`deploy/examples/inferra.container.toml`.

## Local Profile

```bash
docker compose up --build
```

The local profile uses `server.require_loopback = false` inside the container
because Docker port forwarding is not seen as a loopback client by the process.
The host publish address remains loopback-only, so the API is not exposed to the
LAN by default.

## Production Profile

For any LAN, reverse-proxy, or public bind:

1. Use `deploy/examples/inferra.container.production.toml`.
2. Set `INFERRA_API_TOKEN` to a high-entropy bearer token.
3. Terminate TLS at a trusted reverse proxy.
4. Keep `/healthz` and `/readyz` available for container health checks.
5. Send `Authorization: Bearer <token>` for `/api/*` and `/v1/*` routes.

If `server.auth_token_env` is configured but the environment variable is not
set, protected API routes fail closed with `503`.

## Health Checks

The Docker image and Compose file probe:

- `/healthz` for process liveness.
- `/readyz` for storage readiness.

These endpoints intentionally expose only minimal status. Detailed health
remains at `/api/health` and is protected when API auth is enabled.

## Metrics

`/api/metrics` is disabled unless `server.expose_prometheus_metrics = true`.
When metrics are enabled in a non-loopback deployment, scrape with the same
bearer token or restrict access at the reverse proxy/network layer.
