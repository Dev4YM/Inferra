# Failure Model

## Why This Exists

"Failure" is implicit in every other document but never explicitly defined. This creates ambiguity: does "failure" mean an error log? A crashed container? A service responding slowly? A health check that returns 200 but with stale data?

Without a precise definition, the incident detection system is incomplete by design — it only catches failures it happens to recognize, rather than failures it was designed to detect.

---

## Failure States

A service in Inferra can be in one of five states. These states are not binary (up/down) — they model the real spectrum of system health.

```python
class ServiceHealthState(Enum):
    HEALTHY = "healthy"
    # Service is operating within normal parameters.
    # Anomaly score < 0.3. Error rate within 1 stddev of baseline.

    DEGRADED = "degraded"
    # Service is operating but with reduced performance or elevated errors.
    # Anomaly score 0.3–0.6. Error rate or latency above baseline but
    # the service is still responding to requests.

    FAILING = "failing"
    # Service is experiencing active failures.
    # Anomaly score > 0.6. Error rate significantly above baseline,
    # or ERROR/CRITICAL events detected.

    UNREACHABLE = "unreachable"
    # Service is not responding. No events received for >2× the expected interval,
    # or health checks are failing, or dependents report connection errors.

    UNKNOWN = "unknown"
    # Insufficient data to determine state (cold start, no events, no health checks).
```

### State Determination Algorithm

```python
def determine_health_state(service_id: str,
                             anomaly_score: float,
                             recent_events: list[NormalizedEvent],
                             health_checks: list[NormalizedEvent],
                             expected_interval_seconds: float,
                             baseline_error_rate: float) -> ServiceHealthState:
    now = datetime.utcnow()

    # Check for UNREACHABLE: no events at all for a long time
    if recent_events:
        last_event_age = (now - max(e.timestamp for e in recent_events)).total_seconds()
        if last_event_age > expected_interval_seconds * 3:
            return ServiceHealthState.UNREACHABLE
    else:
        return ServiceHealthState.UNKNOWN

    # Check health checks if available
    recent_hc = [e for e in health_checks
                 if (now - e.timestamp).total_seconds() < 120]
    if recent_hc:
        latest_hc = max(recent_hc, key=lambda e: e.timestamp)
        if "fail" in latest_hc.message.lower():
            return ServiceHealthState.FAILING

    # Check for FAILING: high anomaly + active errors
    error_events = [e for e in recent_events
                    if e.severity >= Severity.ERROR
                    and (now - e.timestamp).total_seconds() < 120]
    if anomaly_score > 0.6 and len(error_events) >= 2:
        return ServiceHealthState.FAILING

    # Check for DEGRADED: elevated anomaly or error rate
    if anomaly_score > 0.3:
        return ServiceHealthState.DEGRADED
    if len(error_events) >= 1:
        return ServiceHealthState.DEGRADED

    return ServiceHealthState.HEALTHY
```

---

## Failure Types

Not all failures look the same. The system must recognize these distinct failure patterns:

### 1. Hard Failure
**Definition**: The service is completely non-functional. Crashes, OOM kills, connection refused on all ports.
**Detection signals**: `restart` tags, `oom` tags, `crash` tags, `connection_refused` from all callers, no health check responses.
**Example**: PostgreSQL container exits with code 137 (OOM killed), all dependent services get connection errors.
**Difficulty**: Easy — produces loud, obvious signals.

### 2. Partial Failure
**Definition**: The service is running but some functionality is broken. Some requests succeed, others fail.
**Detection signals**: Mixed severity events (errors + successful health checks), intermittent `connection_refused` or `timeout` tags, error rate elevated but <100%.
**Example**: An API endpoint returns 500 for requests with certain parameters, but the health check endpoint returns 200.
**Difficulty**: Medium — health checks may mask the problem. The contradiction handler models this (intermittent failure pattern).

### 3. Degraded Performance
**Definition**: The service responds but slowly. Latency increases, throughput decreases.
**Detection signals**: `timeout` tags on callers (not on the service itself), elevated response times in structured logs, anomaly detection on latency metrics.
**Example**: Database query times increase from 5ms to 500ms due to missing index. No errors, but upstream services start timing out.
**Difficulty**: Hard — the failing service may produce no error logs at all. Only visible through caller-side timeouts and metric anomalies.

### 4. Silent Failure
**Definition**: The service appears healthy but produces incorrect results. No errors, no crashes, no latency changes.
**Detection signals**: Almost none visible to Inferra. May be detectable through: anomalous patterns in event volume (sudden drop to zero), downstream services producing unusual errors they've never produced before, config changes preceding subtle behavior shifts.
**Example**: A service starts returning cached stale data due to a broken cache invalidation. Health checks pass. No errors logged.
**Difficulty**: Very hard. Inferra can flag anomalous patterns but cannot verify correctness of output. This is honestly the biggest gap — see system_limitations.md.

### 5. Cascading Failure
**Definition**: A failure in one service propagates to dependent services, creating a widening blast radius.
**Detection signals**: Temporally ordered errors across the service dependency graph. Root service fails first, then direct dependents, then their dependents.
**Example**: Redis goes down → session service can't read sessions → API gateway returns 401 for all authenticated requests → frontend shows "logged out" to all users.
**Difficulty**: Medium for detection (visible cascade in logs), hard for root cause attribution (need accurate service graph).

### 6. Intermittent / Flapping Failure
**Definition**: The service alternates between healthy and failing states. Works for a while, breaks, recovers, breaks again.
**Detection signals**: Alternating health check pass/fail events, periodic error bursts followed by quiet periods, `restart` events with successful startup followed by another crash.
**Example**: A memory leak causes the service to crash every 30 minutes, restart successfully, run for 30 minutes, crash again.
**Difficulty**: Medium for detection (visible pattern in event timeline), hard for root cause (the actual bug is not in the crash, it's in the leak that caused the crash).

---

## How Failure Types Map to Detection Components

| Failure Type | Primary Detector | Supporting Detectors |
|---|---|---|
| Hard failure | Signal detector (`error_spike`, `restart_loop`, `connection_errors_inbound`) | Health check detector, inference graph |
| Partial failure | Contradiction handler (mixed health + errors) | Anomaly detector (elevated error rate) |
| Degraded performance | Anomaly detector (latency baselines) | Caller-side `timeout` signal detector |
| Silent failure | Anomaly detector (volume patterns) | **Mostly undetectable** — acknowledged limitation |
| Cascading failure | Inference graph (ordered propagation path) | Service graph traversal, temporal correlation |
| Intermittent / flapping | Restart loop detector, temporal pattern in events | Contradiction handler (health checks passing between failures) |

---

## Incident Trigger Criteria

An incident is created when the correlation engine produces a cluster meeting these criteria:

```python
def should_create_incident(cluster: EventCluster) -> bool:
    # Must have at least 2 events (single events are noise)
    if len(cluster.events) < 2:
        return False

    # Must include at least one WARN+ event
    if cluster.primary_severity < Severity.WARN:
        return False

    # At least one event must have quality.overall >= 0.5
    # (don't create incidents from garbage-parsed data)
    if not any(e.quality.overall >= 0.5 for e in cluster.events):
        return False

    # Service health state check: at least one service must be DEGRADED or worse
    for service_id in cluster.affected_services:
        state = determine_health_state(service_id, ...)
        if state in (ServiceHealthState.DEGRADED, ServiceHealthState.FAILING,
                     ServiceHealthState.UNREACHABLE):
            return True

    # Fallback: high anomaly score even without health state change
    for service_id in cluster.affected_services:
        if cluster.anomaly_scores.get(service_id, 0) > 0.7:
            return True

    return False
```

---

## What This Doesn't Cover (Honest Gaps)

1. **Correctness failures** (service returns wrong data): Inferra observes behavior, not correctness. It cannot tell if a 200 response contains the right answer.
2. **Security failures** (unauthorized access, data breach): Inferra is a debugging tool, not a SIEM. Security events may appear as anomalies but are not specifically modeled.
3. **Slow degradation over days/weeks** (disk filling at 1%/day, memory leak that takes 48 hours to crash): The 72-hour retention window and hourly baseline buckets can catch multi-hour patterns, but multi-day degradation may be too slow for the anomaly detector to flag.
4. **Failures in systems Inferra doesn't observe** (external APIs, CDN, DNS providers, cloud services): These are visible only through their downstream symptoms on observed services.
