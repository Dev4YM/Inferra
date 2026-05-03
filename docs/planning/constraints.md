# System Constraints

## 1. Operational Constraints

### 1.1 Read-Only Observation
Inferra MUST NOT write to, modify, restart, or otherwise mutate any observed system. This includes:
- No sending signals to processes
- No writing to observed log files
- No modifying container state (no `docker stop`, `docker restart`)
- No executing commands on observed hosts
- No modifying configuration files of observed services

**Rationale**: Trust. An operator must know that enabling Inferra cannot cause or worsen an outage. This is non-negotiable.

### 1.2 No Autonomous Actions
Inferra MUST NOT take remediation actions, even if it has high confidence in a root cause. It may:
- Suggest actions (in the explanation layer)
- Display what it would recommend

It must never execute those suggestions. The human operator is always the actor.

### 1.3 No Cloud Dependency for Core Logic
The correlation engine, hypothesis generator, scoring engine, and incident builder MUST operate without any network access. The system must function identically on an air-gapped machine (minus the LLM explanation layer, which degrades gracefully to structured output).

**Exception**: The explanation layer MAY call a remote LLM API if configured, but this is optional. A local LLM (e.g., ollama) or template-based fallback must always be available.

---

## 2. Resource Constraints

### 2.1 Memory Budget
- **Target**: <500MB RSS for core process under normal load (up to 50 events/second)
- **Hard ceiling**: 1GB RSS. If memory exceeds this, the system must shed load (drop oldest events from ringbuffer, refuse new collector registrations).
- **Event store**: Bounded by SQLite file. Old events are pruned by retention policy (default: 72 hours).
- **In-memory structures**: Service graph, active incidents, baselines. Each bounded:
  - Service graph: max 500 nodes, 5000 edges
  - Active incidents: max 200 concurrent
  - Baselines: max 100 metric series × 168 hourly buckets = ~67K data points

### 2.2 CPU Budget
- Normalization: <5ms per event
- Analysis tick (correlation + anomaly detection): <200ms for up to 1000 events in window
- Hypothesis generation per incident: <100ms
- Scoring per hypothesis set: <50ms
- Full pipeline latency (event ingested → incident updated): <2 seconds at p99

### 2.3 Disk Budget
- SQLite event store: governed by retention policy. At 50 events/sec with average 500-byte events, 72-hour retention ≈ 6GB.
- Baseline data: <10MB
- Incident store: <100MB (incidents are much smaller than raw events)
- Total expected: <10GB under default settings

### 2.4 Throughput Limits
- **Designed for**: 1–100 events/second sustained
- **Burst tolerance**: up to 500 events/second for 30 seconds (backpressure buffer absorbs)
- **Not designed for**: >100 events/second sustained. At this scale, Inferra will sample and log a warning. This is a local debugging tool, not a log aggregation platform.

---

## 3. Data Constraints

### 3.1 Event Size Limits
- Maximum single event payload: 64KB (events exceeding this are truncated with a `truncated: true` flag)
- Maximum message field: 16KB
- Maximum metadata JSON: 32KB

### 3.2 Temporal Constraints
- Events with timestamps older than `retention_window` (default 72h) are rejected at ingestion
- Events with future timestamps (>60s ahead of system clock) are flagged as `clock_skew` and stored with both original and ingestion timestamp
- Clock skew between observed services is assumed to be <5 seconds. Beyond this, temporal correlation accuracy degrades and a warning is emitted.

### 3.3 Cardinality Constraints
- Maximum unique `service_id` values: 500
- Maximum unique `host_id` values: 200
- Maximum events per incident: 10,000 (excess events are summarized as counts)
- Maximum hypotheses per incident: 50

---

## 4. Correctness Constraints

### 4.1 Determinism
Given identical input events in identical order, the correlation engine, hypothesis engine, and scoring engine MUST produce identical outputs. This is tested via snapshot tests in CI.

**Implication**: No random number generation in core logic. No time-dependent behavior in scoring (timestamps are inputs, not read from clock). No floating-point non-determinism (use `decimal.Decimal` for scores where reproducibility matters).

### 4.2 Evidence Grounding
Every hypothesis MUST reference at least one concrete event as evidence. Hypotheses without evidence are rejected by the validator.

Every score component MUST trace back to measurable event properties (timestamps, counts, field values). No score component may be derived from LLM output.

### 4.3 Uncertainty Representation
Scores are NOT probabilities. They are relative rankings within an incident's hypothesis set. The system must never present a score as "90% likely" — it presents "hypothesis A scored 0.82 vs hypothesis B scored 0.45, meaning A is better supported by evidence."

Confidence calibration is a separate subsystem that tracks historical accuracy to provide calibrated reliability estimates.

### 4.4 No Hallucination Propagation
The explanation layer receives structured data and produces text. If the LLM generates a claim not supported by the input data, the guardrail system must detect and strip it. Specifically:
- Every service name mentioned must exist in the input
- Every timestamp mentioned must correspond to an actual event
- Every causal claim must map to a hypothesis that was scored

---

## 5. Privacy and Security Constraints

### 5.1 Data Locality
All event data, incidents, and analysis results remain on the local filesystem. No telemetry, no usage reporting, no external data transmission (except optional LLM API calls, which send only sanitized hypothesis structures, never raw log content).

### 5.2 Log Content Handling
When sending data to a remote LLM, the system MUST:
- Strip IP addresses, hostnames, and paths (replace with service_id references)
- Strip environment variable values
- Strip any field tagged as `sensitive` in the event schema
- Send only the structured hypothesis and evidence summary, not raw log lines

### 5.3 No Credential Storage
Inferra stores no credentials. LLM API keys are read from environment variables or a local config file with filesystem permissions as the only protection.

---

## 6. Failure Mode Constraints

### 6.1 Collector Failure Isolation
If one collector crashes or hangs, all other collectors MUST continue operating. A failed collector is restarted with exponential backoff (1s, 2s, 4s, ..., max 60s).

### 6.2 Analysis Failure Isolation
If the analysis engine encounters an error on one incident, it MUST NOT affect other incidents. The failed incident is marked as `analysis_error` and retried on the next tick.

### 6.3 Storage Failure Handling
If SQLite writes fail (disk full, corruption):
- The system continues operating in degraded mode with in-memory-only event storage (bounded by memory budget)
- A persistent warning is displayed in the UI
- The system attempts recovery on each tick

### 6.4 LLM Failure Graceful Degradation
If the LLM is unavailable:
- The system falls back to template-based explanations (structured but less natural)
- All scoring, ranking, and incident management continue unaffected
- The UI displays "explanation unavailable — showing structured summary" instead of a rendered explanation
