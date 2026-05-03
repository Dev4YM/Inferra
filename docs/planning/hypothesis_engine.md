# Hypothesis Engine

## Purpose

The hypothesis engine generates structured explanations for incidents. Given an incident (a cluster of correlated events, an inference graph, and runtime context), it produces multiple competing hypotheses about what went wrong.

The engine is entirely deterministic and rule-based. It does not use an LLM.

---

## Architecture: Composable Signal System

The previous design used 7 monolithic templates. The problem: any failure that doesn't match one of 7 patterns falls through to "anomaly detected — good luck." That's a pattern matcher pretending to be a reasoning engine.

The redesign separates **signal detection** from **hypothesis composition**:

```
Events + Context
      │
      ▼
┌─────────────────────────────────────┐
│ SIGNAL DETECTORS (many, independent) │
│                                     │
│ Each detector scans for one specific │
│ pattern and emits a typed Signal    │
│ if found.                           │
└───────────────┬─────────────────────┘
                │ list[Signal]
                ▼
┌─────────────────────────────────────┐
│ HYPOTHESIS COMPOSER                  │
│                                     │
│ Combines signals into hypotheses    │
│ using composition rules.            │
│ Single signal → simple hypothesis   │
│ Multiple signals → compound hyp     │
└───────────────┬─────────────────────┘
                │ list[Hypothesis]
                ▼
        Evidence assembly + validation
```

The key insight: **signals are small, cheap, and numerous. Hypotheses are built by combining signals.** This means the system can handle novel failure modes it's never explicitly templated for, because a new combination of known signals can produce a new hypothesis.

---

## Signal Detectors

A signal detector is a function that looks for one specific pattern in the incident data:

```python
@dataclass
class Signal:
    signal_type: str           # unique identifier for this signal type
    service_id: str | None     # which service this signal pertains to (None = system-wide)
    severity: str              # "info" | "warning" | "critical"
    description: str           # structured 1-line description
    evidence_event_ids: list[str]  # events that produced this signal
    metadata: dict             # signal-specific structured data
    detector: str              # which detector produced this signal
```

### Built-In Detectors

```python
# ─── Service-level signals ────────────────────────────────────

def detect_error_spike(events: list, service_id: str, anomaly_scores: dict) -> Signal | None:
    """Elevated error rate on a service."""
    errors = [e for e in events if e.service_id == service_id and e.severity >= ERROR]
    if len(errors) >= 3 and anomaly_scores.get(service_id, 0) > 0.3:
        return Signal(
            signal_type="error_spike",
            service_id=service_id,
            severity="warning",
            description=f"{len(errors)} errors on {service_id} (anomaly: {anomaly_scores[service_id]:.2f})",
            evidence_event_ids=[e.event_id for e in errors],
            metadata={"error_count": len(errors), "anomaly_score": anomaly_scores[service_id]},
        )

def detect_restart_loop(events: list, service_id: str) -> Signal | None:
    """Multiple restarts on a single service."""
    restarts = [e for e in events if e.service_id == service_id and "restart" in e.tags]
    if len(restarts) >= 2:
        return Signal(
            signal_type="restart_loop",
            service_id=service_id,
            severity="critical",
            description=f"{service_id} restarted {len(restarts)} times",
            evidence_event_ids=[e.event_id for e in restarts],
            metadata={"restart_count": len(restarts)},
        )

def detect_connection_errors_outbound(events: list, service_id: str) -> Signal | None:
    """Service emitting connection refused / timeout to a target."""
    conn_events = [e for e in events if e.service_id == service_id
                   and ("connection_refused" in e.tags or "timeout" in e.tags)]
    if conn_events:
        targets = extract_connection_targets(conn_events)
        return Signal(
            signal_type="connection_errors_outbound",
            service_id=service_id,
            severity="warning",
            description=f"{service_id} failing to connect to: {', '.join(targets[:3])}",
            evidence_event_ids=[e.event_id for e in conn_events],
            metadata={"targets": targets, "count": len(conn_events)},
        )

def detect_connection_errors_inbound(events: list, service_id: str, graph: ServiceGraphCache) -> Signal | None:
    """Other services failing to connect to this service."""
    dependents = graph.get_dependents(service_id)
    conn_from_dependents = [
        e for e in events
        if e.service_id in dependents
        and ("connection_refused" in e.tags or "timeout" in e.tags)
        and service_id_mentioned_in(e.message, service_id)
    ]
    if conn_from_dependents:
        return Signal(
            signal_type="connection_errors_inbound",
            service_id=service_id,
            severity="critical",
            description=f"{len(conn_from_dependents)} services failing to reach {service_id}",
            evidence_event_ids=[e.event_id for e in conn_from_dependents],
            metadata={"affected_callers": list({e.service_id for e in conn_from_dependents})},
        )

def detect_health_check_failing(events: list, service_id: str) -> Signal | None:
    """Health check failures on a service."""
    hc_fails = [e for e in events if e.service_id == service_id
                and e.event_type == EventType.HEALTH_CHECK and "fail" in e.message.lower()]
    if hc_fails:
        return Signal(signal_type="health_check_failing", service_id=service_id,
                      severity="critical", description=f"{service_id} health check failing",
                      evidence_event_ids=[e.event_id for e in hc_fails], metadata={})

def detect_db_errors(events: list, service_id: str) -> Signal | None:
    """Database-specific error patterns."""
    DB_PATTERNS = ["connection pool", "too many connections", "deadlock",
                   "lock timeout", "query timeout", "could not connect"]
    db_events = [e for e in events if e.service_id == service_id
                 and any(p in e.message.lower() for p in DB_PATTERNS)]
    if db_events:
        return Signal(signal_type="database_errors", service_id=service_id,
                      severity="critical", description=f"Database errors on {service_id}",
                      evidence_event_ids=[e.event_id for e in db_events],
                      metadata={"db_error_type": classify_db_issue(db_events)})

# ─── System-level signals ────────────────────────────────────

def detect_memory_pressure(context: RuntimeContext) -> Signal | None:
    if context and context.resource_summary.memory_pressure in ("elevated", "critical"):
        return Signal(signal_type="memory_pressure", service_id=None,
                      severity=context.resource_summary.memory_pressure,
                      description=f"Memory at {context.host_context.memory_used_percent:.0f}%",
                      evidence_event_ids=[], metadata={"percent": context.host_context.memory_used_percent})

def detect_disk_pressure(context: RuntimeContext) -> Signal | None:
    if context and context.resource_summary.disk_pressure in ("elevated", "critical"):
        worst_mount = max(context.host_context.disk_usage.items(), key=lambda x: x[1].used_percent)
        return Signal(signal_type="disk_pressure", service_id=None,
                      severity=context.resource_summary.disk_pressure,
                      description=f"Disk {worst_mount[0]} at {worst_mount[1].used_percent:.0f}%",
                      evidence_event_ids=[], metadata={"mount": worst_mount[0]})

def detect_cpu_pressure(context: RuntimeContext) -> Signal | None:
    if context and context.resource_summary.cpu_pressure == "critical":
        return Signal(signal_type="cpu_pressure", service_id=None,
                      severity="critical",
                      description=f"CPU at {context.host_context.cpu_percent:.0f}%",
                      evidence_event_ids=[], metadata={"percent": context.host_context.cpu_percent})

# ─── Timeline signals ────────────────────────────────────────

def detect_deployment_event(events: list) -> Signal | None:
    deploys = [e for e in events if "deployment" in e.tags]
    if deploys:
        return Signal(signal_type="deployment_event", service_id=deploys[0].service_id,
                      severity="info",
                      description=f"Deployment on {deploys[0].service_id}",
                      evidence_event_ids=[e.event_id for e in deploys], metadata={})

def detect_config_change(events: list) -> Signal | None:
    configs = [e for e in events if "config_change" in e.tags]
    if configs:
        return Signal(signal_type="config_change_event", service_id=configs[0].service_id,
                      severity="info",
                      description=f"Config change on {configs[0].service_id}",
                      evidence_event_ids=[e.event_id for e in configs], metadata={})

def detect_oom_kill(events: list) -> Signal | None:
    ooms = [e for e in events if "oom" in e.tags]
    if ooms:
        return Signal(signal_type="oom_kill", service_id=ooms[0].service_id,
                      severity="critical",
                      description=f"OOM kill on {ooms[0].service_id}",
                      evidence_event_ids=[e.event_id for e in ooms], metadata={})

def detect_dns_failure(events: list) -> Signal | None:
    dns = [e for e in events if "dns_failure" in e.tags]
    if dns:
        services = {e.service_id for e in dns}
        return Signal(signal_type="dns_failure", service_id=None,
                      severity="critical",
                      description=f"DNS failures affecting {', '.join(services)}",
                      evidence_event_ids=[e.event_id for e in dns], metadata={"services": list(services)})
```

That's **15 detectors**, each looking for one pattern. They run independently and each returns a single `Signal` or `None`.

---

## Hypothesis Composer

The composer takes the list of detected signals and builds hypotheses using **composition rules**. The critical difference from the old template system: **hypotheses can emerge from combinations of signals that no single template anticipated.**

### Composition Rules

```python
COMPOSITION_RULES: list[CompositionRule] = [

    # ── Single-signal hypotheses (equivalent to old templates) ──────

    CompositionRule(
        name="restart_loop",
        requires=["restart_loop"],
        cause_type=CauseType.APPLICATION_BUG,
        cause_subtype="crash_loop",
        title_template="{restart_loop.service_id} in restart loop",
        confidence=0.7,
    ),

    CompositionRule(
        name="database_failure",
        requires=["database_errors"],
        cause_type=CauseType.DATABASE_FAILURE,
        cause_subtype="{database_errors.metadata.db_error_type}",
        title_template="Database issue on {database_errors.service_id}",
        confidence=0.6,
    ),

    CompositionRule(
        name="dns_failure",
        requires=["dns_failure"],
        cause_type=CauseType.INFRASTRUCTURE_FAILURE,
        cause_subtype="dns_failure",
        title_template="DNS resolution failure",
        confidence=0.7,
    ),

    # ── Multi-signal hypotheses (the power of composition) ─────────

    CompositionRule(
        name="dependency_cascade",
        requires=["error_spike", "connection_errors_inbound"],
        requires_same_service=True,
        cause_type=CauseType.DEPENDENCY_FAILURE,
        cause_subtype="upstream_service_failure",
        title_template="{error_spike.service_id} failure cascading to dependents",
        confidence=0.8,
    ),

    CompositionRule(
        name="oom_crash",
        requires=["memory_pressure", "oom_kill"],
        cause_type=CauseType.RESOURCE_EXHAUSTION,
        cause_subtype="memory_exhaustion",
        title_template="OOM kill under memory pressure",
        confidence=0.9,  # strong: resource metric + kernel kill event
    ),

    CompositionRule(
        name="deployment_broke_it",
        requires=["deployment_event", "error_spike"],
        requires_temporal_order=True,  # deployment must precede error spike
        cause_type=CauseType.CONFIGURATION_ERROR,
        cause_subtype="deployment",
        title_template="Errors started after deployment on {deployment_event.service_id}",
        confidence=0.6,
    ),

    CompositionRule(
        name="config_broke_it",
        requires=["config_change_event", "error_spike"],
        requires_temporal_order=True,
        cause_type=CauseType.CONFIGURATION_ERROR,
        cause_subtype="config_change",
        title_template="Errors started after config change on {config_change_event.service_id}",
        confidence=0.6,
    ),

    CompositionRule(
        name="restart_then_cascade",
        requires=["restart_loop", "connection_errors_inbound"],
        requires_same_service=True,
        cause_type=CauseType.APPLICATION_BUG,
        cause_subtype="crash_loop_with_cascade",
        title_template="{restart_loop.service_id} crash loop causing dependent failures",
        confidence=0.85,
    ),

    CompositionRule(
        name="disk_caused_errors",
        requires=["disk_pressure", "error_spike"],
        cause_type=CauseType.RESOURCE_EXHAUSTION,
        cause_subtype="disk_exhaustion",
        title_template="Disk pressure causing service errors",
        confidence=0.7,
    ),

    CompositionRule(
        name="cpu_causing_timeouts",
        requires=["cpu_pressure", "connection_errors_outbound"],
        cause_type=CauseType.RESOURCE_EXHAUSTION,
        cause_subtype="cpu_saturation",
        title_template="CPU saturation causing timeouts",
        confidence=0.65,
    ),

    CompositionRule(
        name="db_cascade",
        requires=["database_errors", "connection_errors_inbound"],
        requires_same_service=True,
        cause_type=CauseType.DATABASE_FAILURE,
        cause_subtype="database_cascade",
        title_template="Database failure on {database_errors.service_id} cascading to callers",
        confidence=0.85,
    ),

    CompositionRule(
        name="deployment_crashed_service",
        requires=["deployment_event", "restart_loop"],
        requires_same_service=True,
        requires_temporal_order=True,
        cause_type=CauseType.CONFIGURATION_ERROR,
        cause_subtype="deployment_crash",
        title_template="Deployment on {deployment_event.service_id} caused crash loop",
        confidence=0.85,
    ),

    CompositionRule(
        name="oom_from_memory_leak_after_deploy",
        requires=["deployment_event", "memory_pressure", "oom_kill"],
        requires_temporal_order=True,
        cause_type=CauseType.CONFIGURATION_ERROR,
        cause_subtype="deployment_memory_leak",
        title_template="Memory leak introduced by deployment on {deployment_event.service_id}",
        confidence=0.75,
    ),
]
```

### Composition Algorithm

```python
def compose_hypotheses(signals: list[Signal],
                        rules: list[CompositionRule],
                        events: list[NormalizedEvent]) -> list[Hypothesis]:
    hypotheses = []

    for rule in rules:
        matching_signal_sets = find_matching_signal_sets(signals, rule)
        for signal_set in matching_signal_sets:
            if rule.requires_temporal_order and not signals_in_temporal_order(signal_set, events):
                continue
            if rule.requires_same_service and not signals_share_service(signal_set):
                continue

            all_evidence = []
            for sig in signal_set:
                all_evidence.extend(sig.evidence_event_ids)

            hypotheses.append(Hypothesis(
                hypothesis_id=str(uuid4()),
                cause_type=rule.cause_type,
                cause_subtype=rule.cause_subtype.format(**signal_dict(signal_set)),
                title=rule.title_template.format(**signal_dict(signal_set)),
                description=build_description(rule, signal_set),
                root_cause_event_id=find_earliest_evidence(signal_set, events),
                affected_services=list({s.service_id for s in signal_set if s.service_id}),
                supporting_events=all_evidence,
                contradicting_events=[],
                evidence_chain=build_evidence_chain(signal_set),
                suggested_checks=build_checks(rule, signal_set),
                generation_rule=rule.name,
                generation_confidence=rule.confidence,
            ))

    # Fallback: if no composed hypothesis matched, generate one per critical signal
    if not hypotheses:
        for sig in signals:
            if sig.severity == "critical":
                hypotheses.append(signal_to_standalone_hypothesis(sig))

    return hypotheses
```

---

## Why This Is Better Than 7 Templates

| Aspect | Old (7 Templates) | New (Signals + Composition) |
|---|---|---|
| Coverage | 7 fixed patterns | 15 detectors × combinatorial composition rules = dozens of patterns |
| Novel failures | Falls through to "anomaly detected" | New signal combinations produce new hypotheses automatically |
| Adding a failure mode | Write a 50-line template function | Add a 10-line signal detector + a 5-line composition rule |
| Compound failures | Each template handles one mode | Multi-signal rules naturally express "A + B happened together" |
| Extensibility | Write Python templates | Add detectors and rules in TOML (future: no code needed) |

### Example: A Failure the Old System Missed

**Scenario**: Deployment on `api-gateway` → increased memory usage → OOM kill 10 minutes later → dependent services get connection errors.

**Old system**: The `config_change` template sees the deployment and errors, generating "deployment broke it." The `resource_exhaustion` template sees memory pressure and OOM. Two independent, disconnected hypotheses. Neither captures the full chain.

**New system**: Signals detected: `deployment_event(api-gateway)`, `memory_pressure`, `oom_kill(api-gateway)`, `connection_errors_inbound(api-gateway)`. Composition rule `oom_from_memory_leak_after_deploy` fires, producing: "Memory leak introduced by deployment on api-gateway." One hypothesis, full causal chain, confidence 0.75.

---

## Configuration

```toml
[hypothesis_engine]
max_hypotheses_per_incident = 50
min_supporting_events = 1
min_generation_confidence = 0.1
dedup_overlap_threshold = 0.5

# User-defined signal detectors (advanced)
# custom_detectors_dir = "./detectors/"

# User-defined composition rules
# Can be defined in TOML without writing Python
[[hypothesis_engine.custom_rules]]
name = "redis_timeout_cascade"
requires = ["connection_errors_outbound", "error_spike"]
requires_same_service = false
cause_type = "dependency_failure"
cause_subtype = "redis_timeout"
title_template = "Redis timeout causing service errors"
confidence = 0.65
```

---

## Performance Budget

| Operation | Budget | Notes |
|---|---|---|
| Run all signal detectors | <50ms | Each detector is O(N) scan with early exit |
| Compose hypotheses from signals | <30ms | Rule matching is small (signals × rules) |
| Evidence assembly per hypothesis | <10ms | Event scan + consistency checks |
| Deduplication | <5ms | Pairwise comparison |
| **Total per incident** | **<100ms** | |
