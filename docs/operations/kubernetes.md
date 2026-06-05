# Kubernetes Deployment

The Helm chart under `deploy/helm/inferra` is the production Kubernetes entry
point. It binds the pod to `0.0.0.0`, disables loopback-only enforcement, and
requires `server.auth_token_env` so exposed API routes are protected by a bearer
token.

## Required Secret

Use an existing secret, ExternalSecret, SealedSecret, or the chart-managed secret
option for small private installs.

```bash
kubectl create secret generic inferra-api-auth \
  --from-literal=api-token='<high-entropy-token>'

helm upgrade --install inferra ./deploy/helm/inferra \
  --set auth.existingSecret=inferra-api-auth
```

By default the chart renders `server.auth_token_env = "INFERRA_API_TOKEN"`.
If no secret is mounted, protected `/api/*` routes fail closed with `503`.

## Probes

The chart uses minimal unauthenticated probe endpoints:

- `/healthz`: process liveness.
- `/readyz`: storage/database readiness.

Do not point Kubernetes probes at `/api/health`; it contains detailed runtime
paths and is protected when auth is enabled.

## Network Exposure

The default Service is `ClusterIP`. If you add an Ingress or change the Service
type, keep bearer auth enabled and terminate TLS at the ingress controller or
service mesh. `server.requireLoopback=false` without `server.authTokenEnv` is
rejected by the chart.

## Service Account And RBAC

Service-account token mounting is disabled by default. It is automatically
enabled when the Kubernetes collector is enabled because that collector needs to
read Kubernetes API objects. Keep RBAC scoped to the namespaces you intend to
observe.

## Metrics

`/api/metrics` is disabled by default. Set
`server.exposePrometheusMetrics=true` to enable it. With API auth enabled,
configure Prometheus with the same bearer token or restrict scraping via
NetworkPolicy.
