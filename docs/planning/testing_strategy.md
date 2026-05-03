# Testing Strategy

## Why This Document Exists

A system that claims determinism as a core feature but has no mechanism to verify that determinism is making a promise it can't keep. This document defines what "correct" means for each subsystem, how to verify it, and what test infrastructure is required.

---

## Test Categories

### 1. Determinism Tests (Snapshot Tests)

**What they verify**: Given identical input, the system produces identical output.

**Mechanism**: Pre-built test fixtures (JSON files) containing input events and expected outputs. On every run, the system processes the fixture and compares the output byte-for-byte against the stored snapshot.

```
tests/
├── fixtures/
│   ├── incident_dependency_cascade.json     # input events
│   ├── incident_dependency_cascade.snap.json # expected output (snapshot)
│   ├── incident_resource_exhaustion.json
│   ├── incident_resource_exhaustion.snap.json
│   ├── incident_restart_loop.json
│   ├── incident_restart_loop.snap.json
│   └── ...
```

**Fixture format**:
```json
{
    "fixture_id": "dependency_cascade_postgres",
    "description": "postgres fails, api-gateway and user-service get connection errors",
    "events": [
        {
            "event_id": "evt-001",
            "timestamp": "2026-01-15T14:30:02.000Z",
            "service_id": "postgres",
            "host_id": "container-abc",
            "severity": 3,
            "event_type": 0,
            "message": "FATAL: too many connections for role \"app\"",
            "tags": ["connection_refused"],
            "fingerprint": "abc123..."
        }
    ],
    "service_graph": {
        "edges": [
            {"source": "api-gateway", "target": "postgres", "type": "depends_on"},
            {"source": "user-service", "target": "postgres", "type": "depends_on"}
        ]
    },
    "runtime_context": {
        "host_memory_percent": 72.0,
        "host_cpu_percent": 45.0
    }
}
```

**Snapshot format** (expected output):
```json
{
    "clusters": [{"cluster_id": "...", "event_count": 8, "services": ["postgres", "api-gateway", "user-service"]}],
    "hypotheses": [
        {"rank": 1, "cause_type": "dependency_failure", "root_service": "postgres"},
        {"rank": 2, "cause_type": "resource_exhaustion", "root_service": null}
    ],
    "top_score_range": [0.75, 0.90]
}
```

**When snapshots break**: If a code change alters output, the test fails. The developer must either:
1. Fix the regression (output should not have changed), OR
2. Update the snapshot (output correctly changed due to intentional algorithm improvement), with a justification in the commit message.

**Coverage target**: At least one fixture per hypothesis template (7 minimum), plus edge cases (empty incidents, single-event incidents, 1000-event stress test).

---

### 2. Component Unit Tests

Each scoring component, correlation strategy, and inference rule is independently testable.

**Temporal Alignment Score**:
```python
def test_temporal_alignment_perfect_order():
    """All effects after root cause → score near 1.0."""
    root = make_event(timestamp="14:30:00", severity=ERROR)
    effects = [make_event(timestamp=f"14:30:0{i}", severity=ERROR) for i in range(1, 6)]
    score = temporal_alignment_score(make_hypothesis(root, effects), to_dict([root] + effects))
    assert score > 0.8

def test_temporal_alignment_reversed_order():
    """All effects before root cause → score near 0.0."""
    root = make_event(timestamp="14:30:10", severity=ERROR)
    effects = [make_event(timestamp=f"14:30:0{i}", severity=ERROR) for i in range(1, 6)]
    score = temporal_alignment_score(make_hypothesis(root, effects), to_dict([root] + effects))
    assert score < 0.3

def test_temporal_alignment_no_root_cause():
    """No root cause claimed → neutral 0.5."""
    score = temporal_alignment_score(make_hypothesis(root=None, effects=[...]), ...)
    assert score == 0.5
```

**Inference Edge Generation**:
```python
def test_dependency_edge_created_when_upstream_fails_first():
    events = [
        make_event(service="postgres", timestamp="14:30:00", severity=ERROR),
        make_event(service="api-gateway", timestamp="14:30:03", severity=ERROR),
    ]
    graph = ServiceGraph(edges=[("api-gateway", "postgres", "depends_on")])
    edges = generate_edges(events, build_indexes(events), graph)
    assert any(e.edge_type == InferenceEdgeType.DEPENDENCY_PROPAGATION for e in edges)

def test_no_edge_when_services_unrelated():
    events = [
        make_event(service="postgres", timestamp="14:30:00", severity=ERROR),
        make_event(service="redis", timestamp="14:30:03", severity=ERROR),
    ]
    graph = ServiceGraph(edges=[])  # no dependencies
    edges = generate_edges(events, build_indexes(events), graph)
    dep_edges = [e for e in edges if e.edge_type == InferenceEdgeType.DEPENDENCY_PROPAGATION]
    assert len(dep_edges) == 0
```

**Deduplication**:
```python
def test_dedup_suppresses_identical_events():
    tracker = DedupTracker(window_seconds=60)
    e1 = make_event(fingerprint="abc", timestamp="14:30:00")
    e2 = make_event(fingerprint="abc", timestamp="14:30:05")
    assert tracker.check(e1) == DedupDecision.STORE
    assert tracker.check(e2) == DedupDecision.SUPPRESS

def test_dedup_stores_different_fingerprints():
    tracker = DedupTracker(window_seconds=60)
    e1 = make_event(fingerprint="abc", timestamp="14:30:00")
    e2 = make_event(fingerprint="def", timestamp="14:30:05")
    assert tracker.check(e1) == DedupDecision.STORE
    assert tracker.check(e2) == DedupDecision.STORE
```

---

### 3. Scoring Accuracy Benchmarks

**What they verify**: Does the top-ranked hypothesis match what a human would identify as the root cause?

**Labeled dataset**: A set of incidents with human-annotated correct hypotheses.

```
tests/benchmarks/
├── labeled_incidents.json
└── benchmark_runner.py
```

```json
{
    "incidents": [
        {
            "fixture_file": "incident_dependency_cascade.json",
            "correct_cause_type": "dependency_failure",
            "correct_root_service": "postgres",
            "annotator": "system_designer",
            "difficulty": "easy"
        },
        {
            "fixture_file": "incident_config_plus_resource.json",
            "correct_cause_type": "configuration_error",
            "correct_root_service": "api-gateway",
            "annotator": "system_designer",
            "difficulty": "hard"
        }
    ]
}
```

**Metrics**:
- **Top-1 accuracy**: How often is the correct hypothesis ranked #1?
- **Top-3 accuracy**: How often is the correct hypothesis in the top 3?
- **Mean reciprocal rank (MRR)**: Average of 1/rank for the correct hypothesis.

**Target**: Top-1 accuracy ≥ 60%, Top-3 accuracy ≥ 85% on the labeled dataset.

**When to run**: Before every release. After any change to scoring weights, inference rules, or signal detectors.

---

### 4. Integration Tests

End-to-end tests that feed raw log lines through the full pipeline and verify the incident output.

```python
async def test_full_pipeline_docker_restart():
    """Simulate a Docker container crash-loop and verify the system detects it."""
    inferra = InferraTestHarness()
    await inferra.start()

    # Feed synthetic Docker log events
    await inferra.inject_raw_events([
        RawEvent(source_type="docker", raw_payload="ERROR: segfault at 0x0", ...),
        RawEvent(source_type="docker", raw_payload="Container exited with code 139", ...),
        RawEvent(source_type="docker", raw_payload="Container started", ...),
        RawEvent(source_type="docker", raw_payload="ERROR: segfault at 0x0", ...),
        RawEvent(source_type="docker", raw_payload="Container exited with code 139", ...),
        RawEvent(source_type="docker", raw_payload="Container started", ...),
    ])

    # Wait for analysis tick
    await inferra.wait_for_analysis()

    incidents = await inferra.get_incidents()
    assert len(incidents) == 1

    hypotheses = await inferra.get_hypotheses(incidents[0].incident_id)
    assert any(h.cause_subtype == "crash_loop" for h in hypotheses)

    await inferra.stop()
```

---

### 5. Performance Tests

Verify that the system meets its latency budgets under load.

```python
def test_normalization_under_5ms():
    """Single event normalization must complete within 5ms at p99."""
    events = [generate_random_raw_event() for _ in range(10000)]
    latencies = []
    for event in events:
        start = time.monotonic_ns()
        normalize(event)
        latencies.append((time.monotonic_ns() - start) / 1e6)  # to ms
    p99 = sorted(latencies)[int(len(latencies) * 0.99)]
    assert p99 < 5.0

def test_analysis_tick_under_200ms():
    """Full analysis tick with 1000 events must complete within 200ms."""
    events = generate_incident_events(count=1000)
    store = InMemoryEventStore(events)
    engine = AnalysisEngine(store, ServiceGraph(...))
    start = time.monotonic_ns()
    clusters = engine.run_tick()
    elapsed_ms = (time.monotonic_ns() - start) / 1e6
    assert elapsed_ms < 200

def test_inference_graph_under_budget():
    """Inference graph construction must complete within configured budget."""
    events = generate_incident_events(count=500)
    start = time.monotonic_ns()
    graph = build_inference_graph(events, service_graph, budget_ms=100)
    elapsed_ms = (time.monotonic_ns() - start) / 1e6
    assert elapsed_ms < 120  # 20% tolerance
```

---

### 6. Regression Tests for Known Bugs

Every bug fix comes with a test that reproduces the bug and verifies the fix:

```python
def test_regression_dedup_severity_escalation():
    """Bug: severity escalation within dedup window was suppressed.
    Fix: split window on severity increase."""
    tracker = DedupTracker(window_seconds=60, severity_escalation_splits=True)
    warn = make_event(fingerprint="abc", severity=WARN, timestamp="14:30:00")
    error = make_event(fingerprint="abc", severity=ERROR, timestamp="14:30:05")
    assert tracker.check(warn) == DedupDecision.STORE
    assert tracker.check(error) == DedupDecision.STORE  # NOT suppressed
```

---

### 7. Weight Tuning Tests

Verify that the feedback-driven weight tuning behaves correctly:

```python
def test_weight_tuning_rewards_correct_ranking():
    """If the system ranked correctly, discriminating components get slightly more weight."""
    state = WeightState(weights=dict(DEFAULT_WEIGHTS), ...)
    original = dict(state.weights)

    # Simulate: correct hypothesis was rank 1, its temporal_alignment was highest
    feedback = IncidentFeedback(correct_hypothesis_id="h1", feedback_type="confirmed")
    hypotheses = [make_scored(id="h1", rank=1, temporal=0.9, correlation=0.5)]

    update_weights(state, feedback, hypotheses)
    assert state.weights["temporal_alignment"] > original["temporal_alignment"]

def test_weight_tuning_bounded():
    """Weights cannot drift beyond MAX_DRIFT from defaults."""
    state = WeightState(weights=dict(DEFAULT_WEIGHTS), ...)
    for _ in range(1000):  # many updates pushing temporal_alignment up
        update_weights(state, always_correct_temporal_feedback, ...)

    max_allowed = DEFAULT_WEIGHTS["temporal_alignment"] * (1.0 + MAX_DRIFT)
    assert state.weights["temporal_alignment"] <= max_allowed

def test_weight_tuning_normalizes():
    """Weights always sum to 1.0 after any update."""
    state = WeightState(weights=dict(DEFAULT_WEIGHTS), ...)
    update_weights(state, some_feedback, some_hypotheses)
    assert abs(sum(state.weights.values()) - 1.0) < 1e-9
```

---

## Test Infrastructure

### Test Helpers

```python
def make_event(service="test-service", timestamp="14:30:00", severity=ERROR,
               message="test error", tags=frozenset(), fingerprint=None, **kwargs):
    """Construct a NormalizedEvent for testing."""
    ...

def make_hypothesis(root=None, effects=None, cause_type=CauseType.UNKNOWN, **kwargs):
    """Construct a Hypothesis for testing."""
    ...

def generate_incident_events(count: int, services: int = 5, severity_mix=None):
    """Generate a synthetic incident with N events across M services."""
    ...

class InferraTestHarness:
    """Full pipeline test harness with in-memory storage."""
    ...
```

### CI Integration

```yaml
# .github/workflows/test.yml
jobs:
  test:
    steps:
      - run: pytest tests/unit/ -v
      - run: pytest tests/integration/ -v
      - run: pytest tests/determinism/ -v
      - run: pytest tests/performance/ -v --benchmark
      - run: python tests/benchmarks/benchmark_runner.py --report
```

---

## Fixture Development Process

1. When a new failure mode is identified, create a fixture by capturing real events (sanitized) or constructing synthetic events.
2. Run the fixture through the pipeline manually and verify the output is correct.
3. Store the output as the snapshot.
4. Add the fixture to the labeled dataset with the correct hypothesis.
5. Add to CI.

**Goal**: Build a library of 50+ fixtures covering all failure taxonomy categories, edge cases, and known-difficult scenarios.

---

## What This Does Not Test

- LLM explanation quality (non-deterministic by nature; tested manually)
- UI rendering (separate frontend test suite)
- Collector reliability under real Docker workloads (requires live environment)
- Long-running baseline accuracy (requires weeks of data; tracked via calibration metrics instead)

These are acknowledged gaps. The testing strategy focuses on what is deterministic and automatable.
