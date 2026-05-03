# System Limitations

This document is an honest accounting of what Inferra cannot do, where it will fail, and what assumptions it makes that may not hold. Every limitation listed here is a known trade-off, not a bug.

---

## 1. Scale Limitations

### 1.1 Single-Machine Only
Inferra is designed for a single machine with up to ~50 services. It cannot monitor distributed clusters spanning multiple hosts.

**Why**: Local-first design. Adding multi-host support would require a networking layer, consensus protocol, and distributed storage — fundamentally changing the architecture. However, the architecture preserves extensibility: all inter-module communication uses typed protocols, storage is abstracted behind interfaces, and no module holds unrecoverable in-memory state. See architecture_overview.md "Scaling Path" section.

**Workaround**: Run one Inferra instance per host. Cross-host correlation is not supported.

### 1.2 Event Throughput Ceiling
Designed for 1–100 events/second sustained. At >100 events/second, sampling kicks in and some events are dropped.

**Why**: Single-process Python architecture with SQLite storage. These choices optimize for simplicity and local-first operation, not throughput.

**Impact**: In high-volume environments (>50 services, verbose logging), the system may miss low-severity events during peak periods. ERROR/CRITICAL events are always preserved.

### 1.3 Incident Count Ceiling
Maximum 200 concurrent active incidents. Beyond this, new incidents are rejected with a warning.

**Why**: Each incident requires analysis, hypothesis generation, and scoring. 200 incidents × 50 hypotheses × scoring passes would exceed the CPU budget.

---

## 2. Analytical Limitations

### 2.1 No Cross-Host Correlation
Inferra running on Host 1 only sees Host 1's perspective. If Service A on Host 1 fails because Service B on Host 2 is down, Inferra can observe A's symptoms but cannot trace the root cause to B.

### 2.2 No Distributed Tracing
Inferra works with logs, not trace spans (OpenTelemetry, Jaeger, Zipkin). Request-level causation ("this specific request caused that specific error") is not available. Correlation is based on temporal proximity, service topology, and pattern matching.

### 2.2.1 Data Quality Varies
Log quality directly determines analysis quality. Each event carries a `DataQuality` score (0.0–1.0) computed during normalization, reflecting confidence in parsed fields (timestamp, service identity, severity, structured data). Low-quality events (score < 0.5) contribute at a 50% discount to hypothesis evidence. Events with quality < 0.3 are excluded from correlation edge generation entirely. This prevents garbage-parsed data from corrupting the analysis pipeline — but it also means that services with poorly structured logs are underrepresented in hypotheses.

### 2.3 Inference, Not Causation
The inference graph constructs plausible event sequences based on temporal ordering and service topology. **This is not causal inference.** "A happened before B on a connected service" is evidence for, but not proof of, A causing B. Common pitfalls:
- Coincidental temporal proximity on unrelated services
- Reverse causation when clock skew exceeds 5 seconds
- Confounding from a shared root cause not in the observation window

The system names its constructs honestly: `InferenceGraph`, `InferenceEdge`, `plausibility` scores. The operator should treat these as structured starting points for investigation.

### 2.4 Service Graph Accuracy
The service dependency graph must be configured explicitly for reliable analysis. Inferra offers auto-detection via Docker Compose labels and log pattern inference, but:
- Docker Compose detection only works in Compose environments
- Docker network membership tells you containers *can* talk, not that they *do*
- Log pattern inference (disabled by default) produces false positives

**Impact**: 4+ signal detectors and the dependency_proximity scoring component depend on the service graph. An inaccurate graph is worse than no graph. Without explicit configuration, dependency-based analysis is disabled and the user is prompted to configure topology.

### 2.5 Cold Start Period
For the first 6–168 hours, anomaly detection has no baseline. During this period:
- Anomaly scores are unreliable
- Confidence labels are conservative
- The anomaly_severity scoring component contributes nothing (10% of score weight)

The UI shows "Baseline: learning (X hours remaining)" during this period.

### 2.6 No Root Cause Guarantee
Inferra identifies the most plausible root cause based on available evidence. It can be wrong. Common failure modes:
- The actual root cause occurred before the observation window
- The root cause did not produce log output (silent failure — see `failure_model.md` type 4)
- The root cause is in a system Inferra doesn't observe (external API, hardware)
- Multiple simultaneous failures are misattributed to a single cause

### 2.7 Failure Detection Coverage
Inferra explicitly models 6 failure types (see `failure_model.md`): hard failure, partial failure, degraded performance, silent failure, cascading failure, and intermittent/flapping failure. Detection confidence varies significantly:
- **Hard failures, cascading failures**: Well-detected (loud signals, clear patterns)
- **Partial failures, intermittent failures**: Moderately detected (contradiction handler models these, but edge cases exist)
- **Degraded performance**: Detectable through anomaly baselines if metric data exists, but may be invisible if the degraded service produces no error logs
- **Silent failures**: Mostly undetectable — the biggest honest gap. Inferra can notice downstream symptoms but cannot verify that a service's output is correct

### 2.8 Hypothesis Interaction
Hypotheses for the same incident are not independent. The scoring engine applies evidence overlap penalties: if hypothesis A (rank 1) and hypothesis B (rank 2) share 80% of their evidence, B's score is reduced because it adds little explanatory value beyond A. Competing root cause explanations for the same downstream symptoms also penalize the lower-scored alternative. This prevents redundant hypotheses from cluttering results but may occasionally suppress a valid alternative that happens to share evidence with a stronger hypothesis.

---

## 3. Data Limitations

### 3.1 Log-Dependent
Inferra's analysis quality is directly proportional to log quality. If a service:
- Emits no logs → invisible to Inferra
- Emits unstructured, inconsistent logs → poor normalization, weak fingerprinting
- Emits only DEBUG/INFO with no severity differentiation → severity-based analysis is useless

### 3.2 No Binary Protocol Inspection
Inferra reads text logs and metrics. It cannot inspect binary protocols (gRPC, Thrift), read database query plans, inspect message queue payloads, or analyze network packet captures.

### 3.3 Retention-Limited History
Default 72-hour event retention. Failures with longer root cause timelines (slow memory leak over days, gradual config drift over weeks) may not be diagnosable because early events have been pruned.

---

## 4. Reasoning Limitations

### 4.1 Signal-Based, Not Learned
Hypothesis generation uses a composable signal detector + composition rule system (15 detectors, 12+ composition rules). This covers significantly more ground than a fixed template set, but:
- Failure modes that produce no recognizable signals are missed
- Novel signal combinations not covered by any composition rule fall through to standalone hypotheses (less useful)
- Custom detectors require Python code; custom composition rules can be defined in TOML but the operator needs to understand the signal model

### 4.2 Feedback Improves Scoring, Not Detection
Operator feedback adjusts scoring weights (bounded, auditable, reversible) so the system ranks hypotheses more accurately over time. However, feedback does NOT change:
- Signal detector logic (what patterns are recognized)
- Composition rules (how signals combine into hypotheses)
- Inference graph construction rules

This means: if a signal detector misses a pattern, no amount of feedback will teach it to recognize that pattern. The detection layer is static. The ranking layer adapts.

**Trade-off**: This preserves determinism and auditability in the detection layer while allowing improvement in the ranking layer. It limits long-term learning but prevents the system from drifting into unpredictable behavior.

### 4.3 Scoring Weights Are Heuristic
The default scoring weights (temporal_alignment=0.25, correlation_strength=0.20, etc.) are starting guesses, not empirically derived values. The feedback-driven weight tuning mechanism can improve them over time (bounded by ±50% of defaults), but:
- Tuning requires operator feedback (optional, may never be provided)
- Small sample sizes mean weights may not converge for months
- Selection bias: operators may only provide feedback on easy incidents

### 4.4 Contradiction Detection Is Improved but Not Complete
The contradiction handler now models intermittent failures (health checks passing between failure bursts get "weak" severity instead of "strong"). However, it still misses:
- Race conditions where cause and effect are nearly simultaneous
- Contradictions that require domain-specific knowledge
- Contradictions in multi-cause incidents

---

## 5. LLM Limitations

### 5.1 Guardrails Catch ~50-60% of Hallucinations
The guardrail pipeline catches:
- Hallucinated service names
- Fabricated timestamps
- Obvious overconfidence language
- Fabricated metric values and event counts

It does NOT reliably catch:
- Plausible-sounding technical details using correct entity names
- Correct names used in wrong context
- Subtle causal fabrications

**Recommendation**: For production incidents, use the "Structured Summary" mode (template fallback) which contains only data that actually exists. Use LLM mode for exploration and learning.

### 5.2 Explanation Quality Varies by Model
Smaller local models (7B parameters) produce noticeably worse explanations than larger models. The system works with any instruction-tuned model but quality correlates with model capability.

### 5.3 Latency
LLM inference adds 2–15 seconds per explanation. This is asynchronous and does not block analysis, but it's noticeable in the UI.

---

## 6. Operational Limitations

### 6.1 No High Availability
Single process. If it crashes, monitoring stops. Run under systemd/supervisor for automatic restart.

### 6.2 No Authentication
The web UI has no login. Appropriate for single-user local use. Not appropriate for shared environments.

### 6.3 No Alerting
Inferra does not send notifications. It is a passive observation tool that must be actively viewed.

### 6.4 No Data Export
No built-in export to SIEM, ticketing, or dashboard systems. Data is accessible via the REST API.

---

## 7. Platform Limitations

### 7.1 Linux-First
Full functionality requires Linux: procfs for system metrics, systemd journal for journald collector, Docker Unix socket for container monitoring. macOS and Windows support is partial via `psutil` and Docker Desktop, with reduced metric granularity and no journald/procfs collectors. The system is functional but limited on non-Linux platforms.

### 7.2 Docker-Centric
Container monitoring assumes Docker. Podman and containerd are not natively supported (their logs can be collected via the file collector, but container-level metrics and topology discovery require Docker API).

---

## Summary: What Inferra Is and Is Not

| Inferra IS | Inferra IS NOT |
|---|---|
| A local debugging assistant for devs and small teams | A production monitoring platform (use Datadog/Grafana for that) |
| A structured evidence presenter with plausible sequence inference | An oracle that always finds the root cause |
| A pattern matcher for common failure modes with composable signals | A general-purpose AI reasoning engine |
| Improvable via feedback (scoring weights adapt) | Self-learning (detection rules don't change) |
| Deterministic and auditable at any point in time | Magically intelligent |
| A starting point for investigation | The final word on an outage |
| Free, open-source, fully local, zero cloud dependency | Enterprise software |

The goal is to get the operator to the right hypothesis faster than manual log triage. Not to replace the operator.
