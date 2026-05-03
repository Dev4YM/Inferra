# Hypothesis Validation

## Purpose

The hypothesis validator is a gate between the hypothesis engine and the scoring engine. It examines each hypothesis for internal consistency, evidence sufficiency, and logical soundness. Invalid hypotheses are rejected or downgraded before scoring.

This prevents the system from presenting logically impossible or unsupported explanations to the operator.

---

## Validation Pipeline

Each hypothesis passes through a sequence of validators. Each validator returns PASS, WARN, or FAIL:

```
Hypothesis
    │
    ├── V1: Evidence Existence Check
    ├── V2: Temporal Consistency Check
    ├── V3: Scope Consistency Check
    ├── V4: Contradiction Ratio Check
    ├── V5: Root Cause Plausibility Check
    ├── V6: Tautology Check
    │
    ▼
Validated Hypothesis (is_valid, invalidation_reasons)
```

**Result aggregation**:
- Any FAIL → `is_valid = False`, hypothesis rejected from scoring
- Any WARN → `is_valid = True`, but `invalidation_reasons` populated and `generation_confidence` reduced by 20% per warning

---

## Validator 1: Evidence Existence

**Check**: Every event_id referenced in `supporting_events` and `contradicting_events` must exist in the event store.

```python
def validate_evidence_existence(hypothesis: Hypothesis, event_store: EventStore) -> ValidationResult:
    missing = []
    for eid in hypothesis.supporting_events + hypothesis.contradicting_events:
        if event_store.get_event(eid) is None:
            missing.append(eid)

    if missing:
        return ValidationResult(
            status="FAIL",
            reason=f"References {len(missing)} non-existent events: {missing[:3]}..."
        )

    if len(hypothesis.supporting_events) == 0:
        return ValidationResult(
            status="FAIL",
            reason="No supporting evidence"
        )

    return ValidationResult(status="PASS")
```

**Rationale**: A hypothesis referencing deleted or pruned events is unverifiable. A hypothesis with zero evidence is speculative.

---

## Validator 2: Temporal Consistency

**Check**: The hypothesized root cause event must not occur after the majority of its claimed effects.

```python
def validate_temporal_consistency(hypothesis: Hypothesis,
                                    events: dict[str, NormalizedEvent]) -> ValidationResult:
    if hypothesis.root_cause_event_id is None:
        return ValidationResult(status="PASS")  # no root cause claimed

    root = events[hypothesis.root_cause_event_id]
    effects = [events[eid] for eid in hypothesis.supporting_events
               if eid != hypothesis.root_cause_event_id and eid in events]

    if not effects:
        return ValidationResult(status="PASS")

    effects_before_root = sum(1 for e in effects if e.timestamp < root.timestamp)
    ratio = effects_before_root / len(effects)

    if ratio > 0.5:
        return ValidationResult(
            status="FAIL",
            reason=f"{effects_before_root}/{len(effects)} effect events occur before the claimed root cause"
        )
    elif ratio > 0.2:
        return ValidationResult(
            status="WARN",
            reason=f"{effects_before_root}/{len(effects)} effect events occur before root cause (possible clock skew)"
        )

    return ValidationResult(status="PASS")
```

**Rationale**: If the majority of "effects" precede the "cause", the causal direction is likely wrong.

---

## Validator 3: Scope Consistency

**Check**: The hypothesis's affected services should match the events it references.

```python
def validate_scope_consistency(hypothesis: Hypothesis,
                                 events: dict[str, NormalizedEvent]) -> ValidationResult:
    evidence_services = {events[eid].service_id for eid in hypothesis.supporting_events if eid in events}
    claimed_services = set(hypothesis.affected_services)

    # Services claimed but with no evidence
    unsupported = claimed_services - evidence_services
    # Services with evidence but not claimed
    unclaimed = evidence_services - claimed_services

    issues = []
    if unsupported:
        issues.append(f"Claims affect on {unsupported} but has no evidence for them")
    if unclaimed and len(unclaimed) > len(claimed_services):
        issues.append(f"Evidence references {unclaimed} but they are not in affected_services")

    if issues:
        return ValidationResult(status="WARN", reason="; ".join(issues))

    return ValidationResult(status="PASS")
```

**Rationale**: A hypothesis claiming to affect 5 services but only having evidence from 1 is overreaching.

---

## Validator 4: Contradiction Ratio

**Check**: If contradicting evidence significantly outweighs supporting evidence, the hypothesis is suspect.

```python
def validate_contradiction_ratio(hypothesis: Hypothesis) -> ValidationResult:
    supporting = len(hypothesis.supporting_events)
    contradicting = len(hypothesis.contradicting_events)

    if supporting == 0:
        return ValidationResult(status="FAIL", reason="No supporting evidence")

    ratio = contradicting / (supporting + contradicting)

    if ratio > 0.6:
        return ValidationResult(
            status="FAIL",
            reason=f"Contradiction ratio {ratio:.0%}: {contradicting} contradicting vs {supporting} supporting"
        )
    elif ratio > 0.3:
        return ValidationResult(
            status="WARN",
            reason=f"High contradiction ratio {ratio:.0%}: hypothesis may be partially incorrect"
        )

    return ValidationResult(status="PASS")
```

**Rationale**: A hypothesis contradicted by more events than it's supported by is likely wrong.

---

## Validator 5: Root Cause Plausibility

**Check**: The claimed root cause event should have characteristics consistent with being a root cause.

```python
def validate_root_cause_plausibility(hypothesis: Hypothesis,
                                       events: dict[str, NormalizedEvent],
                                       inference_graph: InferenceGraph | None) -> ValidationResult:
    if hypothesis.root_cause_event_id is None:
        return ValidationResult(status="PASS")

    root = events.get(hypothesis.root_cause_event_id)
    if root is None:
        return ValidationResult(status="FAIL", reason="Root cause event does not exist")

    issues = []

    # Root cause should not be a low-severity event
    if root.severity < Severity.WARN:
        issues.append(f"Root cause event has severity {root.severity.name} (expected WARN+)")

    # Root cause should be among the earliest events in the incident
    all_timestamps = sorted(events[eid].timestamp for eid in hypothesis.supporting_events if eid in events)
    if all_timestamps:
        root_position = sum(1 for t in all_timestamps if t < root.timestamp) / len(all_timestamps)
        if root_position > 0.7:
            issues.append(f"Root cause event is in the latest 30% of events (position: {root_position:.0%})")

    # If inference graph exists, root cause should have no or few incoming edges
    if inference_graph and hypothesis.root_cause_event_id in inference_graph.nodes:
        node = inference_graph.nodes[hypothesis.root_cause_event_id]
        if node.in_degree > 2:
            issues.append(f"Root cause has {node.in_degree} incoming causal edges (expected to be a source node)")

    if issues:
        return ValidationResult(status="WARN", reason="; ".join(issues))

    return ValidationResult(status="PASS")
```

---

## Validator 6: Tautology Check

**Check**: The hypothesis must not be trivially circular (e.g., "service X failed because service X failed").

```python
def validate_not_tautological(hypothesis: Hypothesis) -> ValidationResult:
    # Check: root cause service is the only affected service, and description
    # doesn't actually explain anything beyond restating the error
    if (hypothesis.root_cause_event_id and
        len(hypothesis.affected_services) == 1 and
        hypothesis.cause_type == CauseType.UNKNOWN):
        return ValidationResult(
            status="WARN",
            reason="Hypothesis may be tautological: restates the failure without explaining cause"
        )

    return ValidationResult(status="PASS")
```

---

## Validation Output

```python
@dataclass
class ValidationResult:
    status: str        # "PASS" | "WARN" | "FAIL"
    reason: str = ""

@dataclass
class ValidatedHypothesis:
    hypothesis: Hypothesis
    is_valid: bool
    validation_results: list[ValidationResult]
    invalidation_reasons: list[str]   # populated from FAIL/WARN results
    adjusted_confidence: float        # generation_confidence reduced by warnings
```

---

## Post-Validation Filtering

After validation:
1. Hypotheses with `is_valid = False` are stored but not scored (available for debugging in the UI with an "invalidated" badge).
2. Hypotheses with warnings have their `generation_confidence` reduced, which feeds into the scoring engine as a multiplicative factor.
3. If all hypotheses are invalidated, the `anomaly_only` template is forced to generate a fallback hypothesis (see `hypothesis_engine.md` Template 7).

---

## Configuration

```toml
[hypothesis_validation]
enabled = true
temporal_consistency_threshold = 0.5     # >50% effects before cause = FAIL
temporal_consistency_warn = 0.2          # >20% = WARN
contradiction_ratio_fail = 0.6
contradiction_ratio_warn = 0.3
min_root_cause_severity = "WARN"
confidence_reduction_per_warning = 0.2   # 20% reduction per WARN
```
