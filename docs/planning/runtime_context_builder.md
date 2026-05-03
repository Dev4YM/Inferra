# Runtime Context Builder

## Purpose

The runtime context builder constructs a snapshot of the system's state at the time of an incident. While the event model captures what happened (logs, errors, state changes), the runtime context captures what the system *looked like* when it happened — resource utilization, process states, container configurations, network topology.

This context is critical for hypothesis generation (e.g., "was the machine under memory pressure when the OOM occurred?") and for the explanation layer (e.g., "at the time of failure, the host was running at 94% CPU utilization with 12 containers competing for resources").

---

## Context Structure

```python
@dataclass
class RuntimeContext:
    """Snapshot of system state at incident time."""
    captured_at: datetime
    incident_id: str

    host_context: HostContext
    container_contexts: dict[str, ContainerContext]     # container_id → context
    service_contexts: dict[str, ServiceContext]          # service_id → context
    topology: TopologySnapshot
    resource_summary: ResourceSummary

@dataclass
class HostContext:
    hostname: str
    os_info: str                    # e.g., "Linux 5.15.0-generic x86_64"
    cpu_count: int
    total_memory_mb: int
    load_average: tuple[float, float, float]  # 1min, 5min, 15min
    cpu_percent: float              # 0–100
    memory_used_percent: float      # 0–100
    disk_usage: dict[str, DiskUsage]  # mount_point → usage
    uptime_seconds: float
    open_file_descriptors: int
    max_file_descriptors: int

@dataclass
class DiskUsage:
    total_gb: float
    used_gb: float
    free_gb: float
    used_percent: float

@dataclass
class ContainerContext:
    container_id: str
    container_name: str
    image: str
    state: str                      # "running" | "restarting" | "exited" | "paused"
    started_at: datetime | None
    restart_count: int
    cpu_percent: float              # container's CPU usage
    memory_usage_mb: float
    memory_limit_mb: float | None   # None if unlimited
    memory_percent: float
    network_rx_bytes: int
    network_tx_bytes: int
    pids: int                       # number of processes in container
    health_status: str | None       # "healthy" | "unhealthy" | "starting" | None (no healthcheck)
    labels: dict[str, str]
    environment_keys: list[str]     # only key names, NOT values (security constraint)
    ports: list[str]                # e.g., ["8080/tcp -> 0.0.0.0:8080"]

@dataclass
class ServiceContext:
    service_id: str
    containers: list[str]           # container_ids running this service
    event_rate_current: float       # events/sec in the last 5 minutes
    error_rate_current: float       # errors/sec in the last 5 minutes
    anomaly_score: float            # from anomaly detector
    last_restart: datetime | None
    restart_count_24h: int
    active_connections: int | None  # if determinable from procfs
    dependency_health: dict[str, str]  # dep_service_id → "healthy" | "degraded" | "down"

@dataclass
class TopologySnapshot:
    """Service dependency graph at capture time."""
    services: list[str]
    edges: list[TopologyEdge]
    isolated_services: list[str]    # services with no known dependencies

@dataclass
class TopologyEdge:
    source_service: str             # caller
    target_service: str             # callee
    relation_type: str              # "calls" | "depends_on" | "shares_host" | "shares_volume"
    inferred_from: str              # "config" | "log_pattern" | "network_observation"

@dataclass
class ResourceSummary:
    """High-level resource health assessment."""
    cpu_pressure: str               # "normal" | "elevated" | "critical"
    memory_pressure: str
    disk_pressure: str
    container_density: int          # number of running containers
    resource_contention_score: float  # 0.0-1.0, composite measure of resource pressure
```

---

## Data Collection

### Source: Procfs Collector (Continuous)

The procfs collector provides continuous system metrics. The runtime context builder aggregates these into a snapshot on demand.

```python
class RuntimeContextBuilder:
    def __init__(self, event_store: EventStore, docker_client: DockerClient | None,
                 procfs_data: ProcfsDataSource):
        self.event_store = event_store
        self.docker = docker_client
        self.procfs = procfs_data

    async def build_context(self, incident: Incident) -> RuntimeContext:
        """Build runtime context for an incident. Called when incident is created or updated."""
        host = await self._collect_host_context()
        containers = await self._collect_container_contexts()
        services = self._build_service_contexts(incident, containers)
        topology = self._build_topology_snapshot()
        resources = self._assess_resource_pressure(host, containers)

        return RuntimeContext(
            captured_at=datetime.utcnow(),
            incident_id=incident.incident_id,
            host_context=host,
            container_contexts=containers,
            service_contexts=services,
            topology=topology,
            resource_summary=resources,
        )
```

### Host Context Collection

```python
async def _collect_host_context(self) -> HostContext:
    return HostContext(
        hostname=socket.gethostname(),
        os_info=platform.platform(),
        cpu_count=os.cpu_count(),
        total_memory_mb=self.procfs.total_memory_mb(),
        load_average=os.getloadavg(),    # Linux only
        cpu_percent=self.procfs.cpu_percent(),
        memory_used_percent=self.procfs.memory_percent(),
        disk_usage={
            mount: DiskUsage(
                total_gb=usage.total / 1e9,
                used_gb=usage.used / 1e9,
                free_gb=usage.free / 1e9,
                used_percent=usage.percent,
            )
            for mount, usage in self.procfs.disk_usage_all().items()
        },
        uptime_seconds=self.procfs.uptime_seconds(),
        open_file_descriptors=self.procfs.open_fds(),
        max_file_descriptors=self.procfs.max_fds(),
    )
```

### Container Context Collection

Uses the Docker API (if available):

```python
async def _collect_container_contexts(self) -> dict[str, ContainerContext]:
    if self.docker is None:
        return {}

    contexts = {}
    for container in await self.docker.containers.list():
        stats = await container.stats(stream=False)
        contexts[container.id[:12]] = ContainerContext(
            container_id=container.id[:12],
            container_name=container.name,
            image=container.image.tags[0] if container.image.tags else container.image.id[:12],
            state=container.status,
            started_at=parse_docker_time(container.attrs["State"]["StartedAt"]),
            restart_count=container.attrs["RestartCount"],
            cpu_percent=calculate_cpu_percent(stats),
            memory_usage_mb=stats["memory_stats"]["usage"] / 1e6,
            memory_limit_mb=stats["memory_stats"].get("limit", None),
            memory_percent=calculate_memory_percent(stats),
            network_rx_bytes=sum_network_stat(stats, "rx_bytes"),
            network_tx_bytes=sum_network_stat(stats, "tx_bytes"),
            pids=stats.get("pids_stats", {}).get("current", 0),
            health_status=container.attrs["State"].get("Health", {}).get("Status"),
            labels=container.labels,
            environment_keys=[e.split("=")[0] for e in container.attrs["Config"].get("Env", [])],
            ports=format_port_bindings(container.ports),
        )
    return contexts
```

### Service Context Assembly

```python
def _build_service_contexts(self, incident: Incident,
                             containers: dict[str, ContainerContext]) -> dict[str, ServiceContext]:
    contexts = {}
    for service_id in incident.affected_services:
        service_containers = [
            cid for cid, ctx in containers.items()
            if self._service_for_container(ctx.container_name) == service_id
        ]

        window = timedelta(minutes=5)
        event_count = self.event_store.count_by_service(service_id, window)
        error_count = self.event_store.count_by_severity(service_id, Severity.ERROR, window)

        contexts[service_id] = ServiceContext(
            service_id=service_id,
            containers=service_containers,
            event_rate_current=event_count / 300.0,
            error_rate_current=error_count / 300.0,
            anomaly_score=self.anomaly_detector.current_score(service_id),
            last_restart=self._find_last_restart(service_id),
            restart_count_24h=self._count_restarts(service_id, timedelta(hours=24)),
            active_connections=self._count_connections(service_id),
            dependency_health=self._assess_dependency_health(service_id),
        )
    return contexts
```

---

## Topology Discovery

The service dependency graph determines how well Inferra can reason about failure cascades. **Getting this wrong corrupts half the analysis pipeline.** The design is therefore **config-first, inference-second**.

### Priority 1: Explicit Configuration (REQUIRED for production use)

This is the primary path. The user defines their service topology in `inferra.toml`:

```toml
[[topology.edges]]
source = "api-gateway"
target = "user-service"
type = "calls"

[[topology.edges]]
source = "user-service"
target = "postgres"
type = "depends_on"

[[topology.edges]]
source = "api-gateway"
target = "redis"
type = "depends_on"
```

**First-run experience**: If no topology is configured and Inferra detects multiple services, it displays a prominent banner in the UI:

```
⚠ No service topology configured.
Inferra has detected 8 services but doesn't know how they depend on each other.
Without topology, dependency-based analysis is disabled.
→ [Configure topology] or [Auto-detect (experimental)]
```

The "Configure topology" button opens a simple UI where the user can draw edges between discovered services. This is persisted to `inferra.toml`.

### Priority 2: Docker Compose Inference (automatic, medium confidence)

If Docker Compose labels are present, `depends_on` relationships are extracted automatically:

```python
for container in containers:
    depends_on = container.labels.get("com.docker.compose.depends_on", "")
    for dep in depends_on.split(","):
        add_edge(container_service, dep_service, "depends_on",
                 source="docker_compose", confidence="medium")
```

These edges are **shown to the user for confirmation** on first detection. Once confirmed or rejected, the decision is persisted.

### Priority 3: Log Pattern Inference (OFF by default, opt-in only)

Connection strings and URL patterns in log messages can reveal dependencies:

```python
DEPENDENCY_PATTERNS = [
    (r'connecting to (\S+):(\d+)', "network_connect"),
    (r'host=(\S+)\s+port=(\d+)', "database_connect"),
    (r'redis://(\S+)', "redis_connect"),
]
```

**This is disabled by default** because it produces false positives. To enable:

```toml
[topology]
log_inference_enabled = true  # default: false
log_inference_confirmation_required = true  # require user confirmation before adding
```

When enabled, inferred edges appear as "suggested" in the UI with an "Accept" / "Reject" button. They do NOT enter the service graph until confirmed.

### Why Config-First Matters

Half the inference graph construction and 4+ signal detectors depend on the service graph. If the graph is wrong:
- Dependency propagation edges point the wrong way
- Connection error signals are attributed to wrong services
- Cascade detection produces nonsensical hypotheses
- Scoring penalties from dependency_proximity are inverted

An inaccurate graph is worse than no graph. With no graph, the system says "I don't know the topology." With a wrong graph, it says "A caused B" when B caused A. The first is honest; the second destroys trust.

---

## Resource Pressure Assessment

```python
def _assess_resource_pressure(self, host: HostContext,
                                containers: dict[str, ContainerContext]) -> ResourceSummary:
    cpu_p = "critical" if host.cpu_percent > 90 else "elevated" if host.cpu_percent > 70 else "normal"
    mem_p = "critical" if host.memory_used_percent > 95 else "elevated" if host.memory_used_percent > 80 else "normal"

    disk_worst = max((d.used_percent for d in host.disk_usage.values()), default=0)
    disk_p = "critical" if disk_worst > 95 else "elevated" if disk_worst > 85 else "normal"

    pressure_map = {"normal": 0.0, "elevated": 0.5, "critical": 1.0}
    contention = (pressure_map[cpu_p] + pressure_map[mem_p] + pressure_map[disk_p]) / 3.0

    return ResourceSummary(
        cpu_pressure=cpu_p,
        memory_pressure=mem_p,
        disk_pressure=disk_p,
        container_density=len([c for c in containers.values() if c.state == "running"]),
        resource_contention_score=contention,
    )
```

---

## Caching and Update Strategy

Runtime context is **not** collected continuously — it is captured on demand:

1. **On incident creation**: Full context capture
2. **On incident update (new events added)**: Incremental update (host metrics refreshed, container states refreshed, service contexts updated)
3. **On explanation request**: Verify context is <5 minutes old; refresh if stale

The context is stored as part of the incident in the incident store.

---

## Performance Budget

| Operation | Budget | Notes |
|---|---|---|
| Host context collection | <50ms | Procfs reads, cached CPU percent |
| Container context collection | <200ms | Docker API calls (main bottleneck) |
| Service context assembly | <50ms | Event store queries (indexed) |
| Topology snapshot | <10ms | In-memory graph read |
| Resource assessment | <5ms | Arithmetic on collected data |
| **Total** | **<315ms** | Acceptable for per-incident capture |

---

## Failure Modes

| Failure | Impact | Mitigation |
|---|---|---|
| Docker socket unavailable | No container context | Container fields set to None; log warning |
| Procfs read error | Missing host metrics | Use last known values; flag as stale |
| Docker API timeout | Context capture delayed | 2-second timeout; use cached container data |
| Too many containers (>100) | Slow collection | Collect only containers related to incident services |

---

## Platform Support

| Platform | Host Context | Container Context | Procfs Metrics |
|---|---|---|---|
| Linux | Full | Full (Docker) | Full |
| macOS | Partial (no procfs) | Full (Docker Desktop) | Via `psutil` fallback |
| Windows | Partial (no procfs) | Limited (Docker Desktop) | Via `psutil` fallback |

The system is designed primarily for Linux. macOS and Windows support is best-effort, with `psutil` as the fallback for system metrics where procfs is unavailable.
