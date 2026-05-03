# Scoring Engine

## Purpose

The scoring engine assigns a deterministic, reproducible numerical score to each validated hypothesis. The score represents how well the hypothesis is supported by evidence relative to other hypotheses for the same incident.

Scores are NOT probabilities. They are relative strength measures within a hypothesis set.

---

## Design Principles

1. **Deterministic at any point in time**: Given the same hypothesis, same evidence, and same weights, the score is always identical. Weights may change over time via the feedback tuning mechanism, but at any given moment the function is pure.
2. **Explainable**: The operator can inspect every component of the score and understand why one hypothesis outranks another.
3. **Improvable**: Operator feedback adjusts scoring weights within bounded ranges, so the system gets better over time without losing auditability.
4. **Normalized**: Final scores are in [0.0, 1.0].

---

## Score Components

### Component 1: Temporal Alignment (default weight: 0.25)

**Measures**: How well the hypothesis's inferred sequence matches the event timestamps.

```python
def temporal_alignment_score(hypothesis: Hypothesis,
                               events: dict[str, NormalizedEvent]) -> float:
    if hypothesis.root_cause_event_id is None:
        return 0.5

    root = events[hypothesis.root_cause_event_id]
    supporting = [events[eid] for eid in hypothesis.supporting_events if eid in events]

    if not supporting:
        return 0.0

    correct_order = sum(1 for e in supporting if e.timestamp >= root.timestamp)
    order_ratio = correct_order / len(supporting)

    timestamps = sorted(e.timestamp for e in supporting)
    time_span = (timestamps[-1] - timestamps[0]).total_seconds()
    if time_span == 0:
        tightness = 1.0
    else:
        tightness = math.exp(-0.693 * time_span / 60.0)

    return 0.6 * order_ratio + 0.4 * tightness
```

### Component 2: Correlation Strength (default weight: 0.20)

**Measures**: The strength of the correlation edges between events in the hypothesis.

```python
def correlation_strength_score(hypothesis: Hypothesis,
                                 cluster: EventCluster) -> float:
    hypothesis_events = set(hypothesis.supporting_events)
    relevant_edges = [
        edge for edge in cluster.correlation_edges
        if edge.source_event_id in hypothesis_events and edge.target_event_id in hypothesis_events
    ]

    if not relevant_edges:
        return 0.0

    avg_weight = sum(e.weight for e in relevant_edges) / len(relevant_edges)
    edge_types = {e.edge_type for e in relevant_edges}
    diversity_bonus = min(0.2, 0.1 * (len(edge_types) - 1))

    return min(1.0, avg_weight + diversity_bonus)
```

### Component 3: Frequency Weight (default weight: 0.15)

**Measures**: Volume of supporting evidence, adjusted for deduplication.

```python
def frequency_weight_score(hypothesis: Hypothesis,
                             events: dict[str, NormalizedEvent]) -> float:
    total_events = 0
    for eid in hypothesis.supporting_events:
        event = events.get(eid)
        if event is None:
            continue
        dedup_count = event.structured_data.get("_dedup_count", 1)
        total_events += dedup_count

    if total_events <= 1:
        return 0.0
    return min(1.0, math.log(total_events) / math.log(200))
```

### Component 4: Dependency Proximity (default weight: 0.15)

**Measures**: Topological coherence of affected services in the service graph.

```python
def dependency_proximity_score(hypothesis: Hypothesis,
                                 service_graph: ServiceGraphCache) -> float:
    services = hypothesis.affected_services
    if len(services) <= 1:
        return 0.5

    connected_pairs = 0
    total_pairs = 0
    total_distance = 0

    for i, s1 in enumerate(services):
        for s2 in services[i+1:]:
            total_pairs += 1
            path = service_graph.shortest_path(s1, s2)
            if path is not None:
                connected_pairs += 1
                total_distance += len(path) - 1

    if total_pairs == 0:
        return 0.5

    connectivity = connected_pairs / total_pairs
    avg_distance = total_distance / max(connected_pairs, 1)
    distance_score = 1.0 / (1.0 + avg_distance) if connected_pairs > 0 else 0.0

    return 0.5 * connectivity + 0.5 * distance_score
```

### Component 5: Evidence Coverage (default weight: 0.15)

**Measures**: What fraction of the incident's events does this hypothesis explain?

```python
def evidence_coverage_score(hypothesis: Hypothesis,
                              incident: Incident) -> float:
    if not incident.events:
        return 0.0

    explained = set(hypothesis.supporting_events)
    total = set(incident.events)
    coverage = len(explained & total) / len(total)

    if coverage < 0.05:
        return coverage * 2
    return min(1.0, coverage)
```

### Component 6: Anomaly Severity (default weight: 0.10)

**Measures**: How anomalous is the system behavior attributed to this hypothesis?

```python
def anomaly_severity_score(hypothesis: Hypothesis,
                             anomaly_scores: dict[str, float]) -> float:
    if not hypothesis.affected_services:
        return 0.0

    service_scores = [
        anomaly_scores.get(sid, 0.0)
        for sid in hypothesis.affected_services
    ]
    return max(service_scores) if service_scores else 0.0
```

---

## Weight Tuning via Feedback

### The Problem with Static Weights

The default weights (0.25, 0.20, 0.15, 0.15, 0.15, 0.10) are heuristic starting points. They are not derived from empirical data. They are plausible guesses by the system designer. The system must be able to improve them based on actual operator feedback.

### Mechanism: Bounded Multiplicative Update

When an operator resolves an incident and identifies the correct hypothesis, the system measures which scoring components contributed to ranking that hypothesis correctly (or incorrectly).

```python
@dataclass
class WeightState:
    weights: dict[str, float]           # current weights
    default_weights: dict[str, float]   # original defaults (for reset)
    update_count: int                   # total feedback-driven updates
    history: list[WeightSnapshot]       # audit trail

@dataclass
class WeightSnapshot:
    timestamp: datetime
    weights_before: dict[str, float]
    weights_after: dict[str, float]
    trigger_incident_id: str
    reason: str

DEFAULT_WEIGHTS = {
    "temporal_alignment": 0.25,
    "correlation_strength": 0.20,
    "frequency_weight": 0.15,
    "dependency_proximity": 0.15,
    "evidence_coverage": 0.15,
    "anomaly_severity": 0.10,
}

MAX_DRIFT = 0.5  # no weight can deviate more than 50% from its default
LEARNING_RATE = 0.05  # small updates per feedback
MIN_WEIGHT = 0.03  # no weight can go below 3%
```

### Update Algorithm

```python
def update_weights(state: WeightState, feedback: IncidentFeedback,
                    hypotheses: list[ScoredHypothesis]) -> None:
    """Adjust weights based on feedback. Small, bounded, auditable updates."""
    if feedback.feedback_type == "skipped":
        return
    if feedback.correct_hypothesis_id is None and feedback.feedback_type == "none_correct":
        return  # can't learn from "all wrong" without knowing what's right

    correct = next((h for h in hypotheses if h.hypothesis_id == feedback.correct_hypothesis_id), None)
    if correct is None:
        return

    snapshot_before = dict(state.weights)

    if correct.rank == 1:
        # System got it right. Reward the components that scored highest for this hypothesis.
        _reward_discriminating_components(state, correct, hypotheses)
    else:
        # System got it wrong. The correct hypothesis was ranked below #1.
        top = hypotheses[0]
        _penalize_misleading_components(state, correct, top)

    # Enforce bounds
    _enforce_bounds(state)

    # Re-normalize to sum to 1.0
    total = sum(state.weights.values())
    for k in state.weights:
        state.weights[k] /= total

    state.update_count += 1
    state.history.append(WeightSnapshot(
        timestamp=datetime.utcnow(),
        weights_before=snapshot_before,
        weights_after=dict(state.weights),
        trigger_incident_id=feedback.incident_id,
        reason=f"rank_1={'correct' if correct.rank == 1 else 'incorrect'}",
    ))

def _reward_discriminating_components(state: WeightState,
                                        correct: ScoredHypothesis,
                                        all_hyps: list[ScoredHypothesis]) -> None:
    """If the system ranked correctly, slightly reward components
    where the correct hypothesis scored higher than the average."""
    avg_breakdown = _average_breakdown(all_hyps)
    correct_breakdown = correct.score_breakdown

    for component in state.weights:
        correct_val = getattr(correct_breakdown, component)
        avg_val = avg_breakdown[component]
        if correct_val > avg_val:
            # This component helped discriminate correctly — small reward
            state.weights[component] *= (1.0 + LEARNING_RATE * 0.5)

def _penalize_misleading_components(state: WeightState,
                                      correct: ScoredHypothesis,
                                      top_wrong: ScoredHypothesis) -> None:
    """If the system ranked incorrectly, slightly penalize components
    where the wrong hypothesis scored higher than the correct one."""
    correct_breakdown = correct.score_breakdown
    wrong_breakdown = top_wrong.score_breakdown

    for component in state.weights:
        correct_val = getattr(correct_breakdown, component)
        wrong_val = getattr(wrong_breakdown, component)
        if wrong_val > correct_val:
            # This component pushed the wrong hypothesis up — penalize
            state.weights[component] *= (1.0 - LEARNING_RATE)
        elif correct_val > wrong_val:
            # This component actually favored the correct answer — reward
            state.weights[component] *= (1.0 + LEARNING_RATE)

def _enforce_bounds(state: WeightState) -> None:
    """No weight can drift more than MAX_DRIFT from its default, or go below MIN_WEIGHT."""
    for k in state.weights:
        default = state.default_weights[k]
        lower = max(MIN_WEIGHT, default * (1.0 - MAX_DRIFT))
        upper = default * (1.0 + MAX_DRIFT)
        state.weights[k] = max(lower, min(upper, state.weights[k]))
```

### Properties of This Approach


| Property         | Guarantee                                                                                                                        |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| **Determinism**  | At any moment, weights are fixed values. Same weights + same input = same score.                                                 |
| **Auditability** | Every weight change is logged with timestamp, incident, and before/after values.                                                 |
| **Bounded**      | No weight can deviate more than 50% from its default. System can't go haywire.                                                   |
| **Reversible**   | Operator can reset to defaults at any time via `inferra config reset-weights`.                                                   |
| **Gradual**      | Learning rate of 0.05 means ~20 feedback events to move a weight significantly. No single feedback point causes dramatic shifts. |
| **Convergent**   | Bounds + normalization prevent divergence. Weights oscillate within a stable range.                                              |


### Weight Reset

```python
def reset_weights(state: WeightState) -> None:
    state.weights = dict(state.default_weights)
    state.history.append(WeightSnapshot(
        timestamp=datetime.utcnow(),
        weights_before=dict(state.weights),
        weights_after=dict(state.default_weights),
        trigger_incident_id="manual_reset",
        reason="operator reset to defaults",
    ))
```

---

## Final Score Computation

```python
def compute_total_score(hypothesis: ValidatedHypothesis,
                         incident: Incident,
                         cluster: EventCluster,
                         events: dict[str, NormalizedEvent],
                         service_graph: ServiceGraphCache,
                         anomaly_scores: dict[str, float],
                         weight_state: WeightState) -> ScoredHypothesis:

    components = {
        "temporal_alignment": temporal_alignment_score(hypothesis, events),
        "correlation_strength": correlation_strength_score(hypothesis, cluster),
        "frequency_weight": frequency_weight_score(hypothesis, events),
        "dependency_proximity": dependency_proximity_score(hypothesis, service_graph),
        "evidence_coverage": evidence_coverage_score(hypothesis, incident),
        "anomaly_severity": anomaly_severity_score(hypothesis, anomaly_scores),
    }

    raw_score = sum(weight_state.weights[k] * components[k] for k in weight_state.weights)
    adjusted_score = raw_score * hypothesis.adjusted_confidence
    final_score = max(0.0, min(1.0, adjusted_score))

    return ScoredHypothesis(
        hypothesis_id=hypothesis.hypothesis.hypothesis_id,
        rank=0,
        cause_type=hypothesis.hypothesis.cause_type,
        description=hypothesis.hypothesis.description,
        total_score=final_score,
        score_breakdown=ScoreBreakdown(**components),
        supporting_events=hypothesis.hypothesis.supporting_events,
        contradicting_events=hypothesis.hypothesis.contradicting_events,
        affected_services=hypothesis.hypothesis.affected_services,
        suggested_checks=hypothesis.hypothesis.suggested_checks,
        confidence_label="",
        is_valid=hypothesis.is_valid,
        invalidation_reasons=hypothesis.invalidation_reasons,
    )
```

---

## Hypothesis Interaction (Relative Scoring Dynamics)

Hypotheses for the same incident are not truly independent. If hypothesis A explains 80% of the events and hypothesis B explains 60% — and most of B's events are a subset of A's — then B's residual explanatory power is only 20%, not 60%.

The scoring engine applies an **evidence overlap penalty** after individual scoring but before ranking:

```python
def apply_interaction_effects(hypotheses: list[ScoredHypothesis]) -> list[ScoredHypothesis]:
    """Adjust scores based on how hypotheses relate to each other.
    Higher-scoring hypotheses 'claim' evidence, reducing the effective
    coverage of lower-scoring hypotheses that share the same events."""

    # Sort by raw score descending
    sorted_hyps = sorted(hypotheses, key=lambda h: h.total_score, reverse=True)

    claimed_events: set[str] = set()

    for hyp in sorted_hyps:
        hyp_events = set(hyp.supporting_events)

        # What fraction of this hypothesis's evidence is already claimed?
        overlap = hyp_events & claimed_events
        overlap_ratio = len(overlap) / max(len(hyp_events), 1)

        # Apply diminishing returns: if 80% of your evidence is already
        # explained by a higher-ranked hypothesis, you add little value
        if overlap_ratio > 0.3:
            redundancy_penalty = 1.0 - (overlap_ratio * 0.5)
            hyp.total_score *= redundancy_penalty
            hyp.interaction_note = (
                f"{overlap_ratio:.0%} evidence overlap with higher-ranked hypothesis. "
                f"Score reduced by {(1-redundancy_penalty):.0%}."
            )

        # This hypothesis claims its events
        claimed_events |= hyp_events

    # Competing root causes: if two hypotheses claim different root causes
    # for the same downstream symptoms, penalize the weaker one
    for i, h1 in enumerate(sorted_hyps):
        for h2 in sorted_hyps[i+1:]:
            if (h1.root_cause_event_id and h2.root_cause_event_id
                and h1.root_cause_event_id != h2.root_cause_event_id):
                # Different root causes
                shared_symptoms = (set(h1.supporting_events) & set(h2.supporting_events)
                                   - {h1.root_cause_event_id, h2.root_cause_event_id})
                if len(shared_symptoms) > 2:
                    # Competing explanations for the same symptoms
                    # The lower-scored one gets a small penalty
                    h2.total_score *= 0.9
                    if not hasattr(h2, 'interaction_note') or not h2.interaction_note:
                        h2.interaction_note = ""
                    h2.interaction_note += (
                        f" Competes with '{h1.description[:50]}' for "
                        f"{len(shared_symptoms)} shared symptoms."
                    )

    return sorted_hyps
```

This ensures that:
1. A hypothesis that merely restates what a better hypothesis already explains doesn't get undeserved rank.
2. Competing root cause explanations for the same downstream events don't both appear equally confident.
3. The operator sees meaningful alternatives, not redundant restatements.

---

## Ranking

```python
def rank_hypotheses(scored: list[ScoredHypothesis]) -> list[ScoredHypothesis]:
    # Apply interaction effects first
    scored = apply_interaction_effects(scored)

    sorted_hyps = sorted(scored, key=lambda h: h.total_score, reverse=True)
    for i, hyp in enumerate(sorted_hyps):
        hyp.rank = i + 1
    return sorted_hyps
```

Ties broken by: evidence_coverage (desc), contradicting_events count (asc), root cause timestamp (asc).

---

## Weight Persistence

Stored in `./data/scoring_weights.json`:

```json
{
    "schema_version": 1,
    "weights": {
        "temporal_alignment": 0.27,
        "correlation_strength": 0.18,
        "frequency_weight": 0.16,
        "dependency_proximity": 0.14,
        "evidence_coverage": 0.16,
        "anomaly_severity": 0.09
    },
    "default_weights": {
        "temporal_alignment": 0.25,
        "correlation_strength": 0.20,
        "frequency_weight": 0.15,
        "dependency_proximity": 0.15,
        "evidence_coverage": 0.15,
        "anomaly_severity": 0.10
    },
    "update_count": 23,
    "last_updated": "2026-05-01T14:00:00Z"
}
```

Weight history is stored separately in `./data/weight_history.jsonl` (one JSON line per update, append-only).

---

## Score Interpretation


| Score Range | Interpretation                 | UI Display                                |
| ----------- | ------------------------------ | ----------------------------------------- |
| 0.80 – 1.00 | Strongly supported by evidence | Bold, green highlight                     |
| 0.50 – 0.79 | Moderately supported           | Normal display                            |
| 0.25 – 0.49 | Weakly supported               | Muted display, "low confidence" badge     |
| 0.00 – 0.24 | Poorly supported               | Collapsed by default, "speculative" badge |


---

## Configuration

```toml
[scoring]
# Default weights (used until feedback tunes them)
[scoring.default_weights]
temporal_alignment = 0.25
correlation_strength = 0.20
frequency_weight = 0.15
dependency_proximity = 0.15
evidence_coverage = 0.15
anomaly_severity = 0.10

# Tuning parameters
[scoring.tuning]
enabled = true
learning_rate = 0.05
max_drift_from_default = 0.5
min_weight = 0.03

# Ranking
tiebreak_order = ["evidence_coverage", "contradicting_events_asc", "root_cause_timestamp_asc"]
```

---

## Performance Budget


| Operation                      | Budget |
| ------------------------------ | ------ |
| Score one hypothesis           | <5ms   |
| Score full set (50 hypotheses) | <50ms  |
| Weight update (on feedback)    | <1ms   |
| Ranking + tiebreaking          | <1ms   |


