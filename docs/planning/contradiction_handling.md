# Contradiction Handling

## Purpose

Contradictions arise when evidence for a hypothesis is inconsistent — some events support the hypothesis while others actively contradict it. The contradiction handler detects, classifies, and surfaces these conflicts so that the scoring engine can penalize contradicted hypotheses and the UI can present the operator with an honest assessment.

---

## What Is a Contradiction?

A contradiction is a piece of evidence that, if true, makes a hypothesis less likely or impossible. Examples:

| Hypothesis | Contradicting Evidence | Why It's a Contradiction |
|---|---|---|
| "postgres is down" | Successful health check on postgres at incident time | postgres was reachable |
| "OOM killed the process" | Memory usage was at 40% when the crash occurred | No memory pressure |
| "deployment broke the service" | Errors started 30 minutes before the deployment | Timeline doesn't fit |
| "network partition between A and B" | A successfully called B during the incident window | Connectivity existed |

---

## Contradiction Detection Rules

### Rule 1: Timeline Violation

A contradiction where the alleged cause occurred after its claimed effects.

```python
def detect_timeline_violation(hypothesis: Hypothesis,
                                events: dict[str, NormalizedEvent]) -> list[Contradiction]:
    contradictions = []
    if hypothesis.root_cause_event_id is None:
        return contradictions

    root = events[hypothesis.root_cause_event_id]
    for eid in hypothesis.supporting_events:
        event = events.get(eid)
        if event and event.timestamp < root.timestamp and eid != hypothesis.root_cause_event_id:
            latency = (root.timestamp - event.timestamp).total_seconds()
            if latency > 5.0:  # allow 5s tolerance for clock skew
                contradictions.append(Contradiction(
                    hypothesis_id=hypothesis.hypothesis_id,
                    contradicting_event_id=eid,
                    contradiction_type="timeline_violation",
                    explanation=(
                        f"Event on {event.service_id} at {event.timestamp.isoformat()} occurred "
                        f"{latency:.0f}s before the alleged root cause at {root.timestamp.isoformat()}"
                    ),
                    severity="strong" if latency > 30 else "weak",
                ))
    return contradictions
```

### Rule 2: Health Check Contradiction (with Temporal Resolution)

A successful health check on a service that the hypothesis claims was failing. **Crucially, this rule models intermittent failures** — a health check passing 2 minutes into a 5-minute incident does not definitively prove the service was healthy the entire time.

```python
def detect_health_contradiction(hypothesis: Hypothesis,
                                  events: dict[str, NormalizedEvent]) -> list[Contradiction]:
    contradictions = []
    hyp_events = [events[eid] for eid in hypothesis.supporting_events if eid in events]
    if not hyp_events:
        return contradictions

    hyp_start = min(e.timestamp for e in hyp_events)
    hyp_end = max(e.timestamp for e in hyp_events)

    for event in events.values():
        if (event.event_type != EventType.HEALTH_CHECK
            or event.service_id not in hypothesis.affected_services):
            continue
        if "pass" not in event.message.lower() and "healthy" not in event.message.lower():
            continue
        if not (hyp_start <= event.timestamp <= hyp_end):
            continue

        # Temporal resolution: divide the incident into sub-windows
        # and check if FAILURE events and HEALTH events co-occur in the same sub-window
        SUB_WINDOW = timedelta(seconds=30)
        hc_window_start = event.timestamp - SUB_WINDOW / 2
        hc_window_end = event.timestamp + SUB_WINDOW / 2

        # Are there failure events for this service in the SAME sub-window?
        concurrent_failures = [
            e for e in hyp_events
            if e.service_id == event.service_id
            and e.severity >= Severity.ERROR
            and hc_window_start <= e.timestamp <= hc_window_end
        ]

        if concurrent_failures:
            # Health check passed but errors exist in the same 30s window
            # → intermittent failure, weak contradiction
            contradictions.append(Contradiction(
                hypothesis_id=hypothesis.hypothesis_id,
                contradicting_event_id=event.event_id,
                contradiction_type="state_inconsistency",
                explanation=(
                    f"Health check on {event.service_id} passed at {event.timestamp.isoformat()}, "
                    f"but {len(concurrent_failures)} errors occurred within 30s. "
                    f"Likely intermittent failure — health check caught a good moment."
                ),
                severity="weak",  # intermittent, not strong
            ))
        else:
            # Health check passed and NO errors in the same sub-window
            # Are there errors BEFORE and AFTER this health check?
            errors_before = [e for e in hyp_events
                             if e.service_id == event.service_id
                             and e.severity >= Severity.ERROR
                             and e.timestamp < hc_window_start]
            errors_after = [e for e in hyp_events
                            if e.service_id == event.service_id
                            and e.severity >= Severity.ERROR
                            and e.timestamp > hc_window_end]

            if errors_before and errors_after:
                # Errors flanking a clean health check → intermittent
                contradictions.append(Contradiction(
                    hypothesis_id=hypothesis.hypothesis_id,
                    contradicting_event_id=event.event_id,
                    contradiction_type="state_inconsistency",
                    explanation=(
                        f"Health check on {event.service_id} passed at {event.timestamp.isoformat()}, "
                        f"with errors before and after. Pattern consistent with intermittent failure."
                    ),
                    severity="weak",
                ))
            elif not errors_before and not errors_after:
                # Health check passed and no errors for this service at all
                # → strong contradiction: hypothesis claims failure, but we see none
                contradictions.append(Contradiction(
                    hypothesis_id=hypothesis.hypothesis_id,
                    contradicting_event_id=event.event_id,
                    contradiction_type="state_inconsistency",
                    explanation=(
                        f"Health check on {event.service_id} passed at {event.timestamp.isoformat()}, "
                        f"and no errors found for {event.service_id} during the incident."
                    ),
                    severity="strong",
                ))
            else:
                # Health check passed, errors only before OR only after
                # → moderate: service may have recovered or not yet failed
                contradictions.append(Contradiction(
                    hypothesis_id=hypothesis.hypothesis_id,
                    contradicting_event_id=event.event_id,
                    contradiction_type="state_inconsistency",
                    explanation=(
                        f"Health check on {event.service_id} passed at {event.timestamp.isoformat()}. "
                        f"Errors {'preceded' if errors_before else 'followed'} this check."
                    ),
                    severity="weak",
                ))
    return contradictions
```

This handles the most common real-world pattern: intermittent failures where health checks pass between failure bursts. Instead of a blunt "health check passed = strong contradiction," the severity is calibrated based on whether the failure was likely intermittent.

### Rule 3: Resource State Contradiction

Runtime context shows resource metrics inconsistent with a resource exhaustion hypothesis.

```python
def detect_resource_contradiction(hypothesis: Hypothesis,
                                    runtime_context: RuntimeContext | None) -> list[Contradiction]:
    contradictions = []
    if runtime_context is None:
        return contradictions

    if hypothesis.cause_subtype == "memory_exhaustion":
        if runtime_context.host_context.memory_used_percent < 60:
            contradictions.append(Contradiction(
                hypothesis_id=hypothesis.hypothesis_id,
                contradicting_event_id="",  # context-based, not event-based
                contradiction_type="state_inconsistency",
                explanation=(
                    f"Hypothesis claims memory exhaustion but memory usage is "
                    f"{runtime_context.host_context.memory_used_percent:.0f}%"
                ),
                severity="strong",
            ))

    if hypothesis.cause_subtype == "cpu_saturation":
        if runtime_context.host_context.cpu_percent < 50:
            contradictions.append(Contradiction(
                hypothesis_id=hypothesis.hypothesis_id,
                contradicting_event_id="",
                contradiction_type="state_inconsistency",
                explanation=(
                    f"Hypothesis claims CPU saturation but CPU usage is "
                    f"{runtime_context.host_context.cpu_percent:.0f}%"
                ),
                severity="strong",
            ))

    return contradictions
```

### Rule 4: Scope Mismatch

The hypothesis claims a specific service or host is the root cause, but the evidence pattern doesn't match.

```python
def detect_scope_mismatch(hypothesis: Hypothesis,
                            events: dict[str, NormalizedEvent]) -> list[Contradiction]:
    contradictions = []

    if hypothesis.root_cause_event_id:
        root = events[hypothesis.root_cause_event_id]
        # If the root cause service has no other errors, the hypothesis may be wrong
        root_service_errors = [
            e for e in events.values()
            if e.service_id == root.service_id and e.severity >= Severity.ERROR
        ]
        other_service_errors = [
            e for e in events.values()
            if e.service_id != root.service_id and e.severity >= Severity.ERROR
        ]

        if len(root_service_errors) == 1 and len(other_service_errors) > 5:
            contradictions.append(Contradiction(
                hypothesis_id=hypothesis.hypothesis_id,
                contradicting_event_id="",
                contradiction_type="scope_mismatch",
                explanation=(
                    f"Root cause service {root.service_id} has only 1 error, "
                    f"while {len(other_service_errors)} errors occurred on other services. "
                    f"The scope may be incorrectly attributed."
                ),
                severity="weak",
            ))

    return contradictions
```

### Rule 5: Mutual Exclusion

Two hypotheses for the same incident propose mutually exclusive root causes.

```python
def detect_mutual_exclusion(hypotheses: list[Hypothesis]) -> list[Contradiction]:
    """Detect hypothesis pairs that cannot both be true."""
    contradictions = []

    EXCLUSIVE_PAIRS = [
        ("memory_exhaustion", "disk_exhaustion"),  # different resource constraints
        ("upstream_service_failure", "config_change"),  # if config change is root cause, upstream failure is a symptom
    ]

    for i, h1 in enumerate(hypotheses):
        for h2 in hypotheses[i+1:]:
            pair = (h1.cause_subtype, h2.cause_subtype)
            reverse = (h2.cause_subtype, h1.cause_subtype)
            if pair in EXCLUSIVE_PAIRS or reverse in EXCLUSIVE_PAIRS:
                contradictions.append(Contradiction(
                    hypothesis_id=h1.hypothesis_id,
                    contradicting_event_id="",
                    contradiction_type="mutual_exclusion",
                    explanation=f"Mutually exclusive with hypothesis '{h2.title}'",
                    severity="informational",
                ))
    return contradictions
```

---

## Contradiction Severity

```python
class ContradictionSeverity(Enum):
    STRONG = "strong"
    # The contradiction directly invalidates the hypothesis if the evidence is accurate.
    # Example: health check passed on an allegedly-down service.

    WEAK = "weak"
    # The contradiction raises doubt but doesn't definitively invalidate.
    # Example: root cause service has few errors (could be that errors were too fast to log).

    INFORMATIONAL = "informational"
    # Worth noting but doesn't impact scoring.
    # Example: mutual exclusion between two different hypotheses.
```

---

## Impact on Scoring

Contradictions affect the scoring engine through the `contradiction_penalty`:

```python
def contradiction_penalty(contradictions: list[Contradiction]) -> float:
    """Compute a multiplicative penalty based on contradictions.
    Returns a value in [0.5, 1.0] that multiplies the final score.
    """
    strong_count = sum(1 for c in contradictions if c.severity == "strong")
    weak_count = sum(1 for c in contradictions if c.severity == "weak")

    # Each strong contradiction reduces score by 15%, each weak by 5%
    penalty = 1.0 - (strong_count * 0.15 + weak_count * 0.05)
    return max(0.5, penalty)  # never reduce score by more than 50%
```

This penalty is applied multiplicatively to the final score:
```python
final_score = raw_score * validation_confidence * contradiction_penalty(contradictions)
```

---

## Ambiguity Representation

When contradictions exist, the system must communicate ambiguity honestly:

### In the Hypothesis Display

```
Hypothesis: "postgres failure caused API errors"
Score: 0.72
Confidence: medium

⚠ Contradictions:
  - Health check on postgres passed at 14:32:05 (1 minute into incident)
    → This weakens the hypothesis but doesn't definitively disprove it
      (health checks may have succeeded intermittently)
```

### In the Explanation Layer

The LLM prompt includes contradictions explicitly, and the explanation must address them:

```
Include in your explanation:
- The primary hypothesis and its evidence
- The following contradictions and how they affect confidence:
  [list of contradictions]
- An honest assessment of uncertainty
```

---

## Configuration

```toml
[contradiction_handling]
enabled = true
timeline_tolerance_seconds = 5.0     # allow this much clock skew before flagging
strong_penalty_per_contradiction = 0.15
weak_penalty_per_contradiction = 0.05
min_penalty_multiplier = 0.5          # never reduce score below 50%

[contradiction_handling.rules]
timeline_violation = true
health_check = true
resource_state = true
scope_mismatch = true
mutual_exclusion = true
```

---

## Failure Modes

| Failure | Impact | Mitigation |
|---|---|---|
| Health check data missing | Cannot detect health contradictions | Rule silently skips; no false contradictions |
| Runtime context unavailable | Cannot detect resource contradictions | Rule silently skips |
| Clock skew misidentified as timeline violation | False contradiction | 5-second tolerance; weak severity for borderline cases |
| Over-penalization (too many weak contradictions) | Good hypothesis scored too low | Min penalty multiplier caps at 0.5 |
