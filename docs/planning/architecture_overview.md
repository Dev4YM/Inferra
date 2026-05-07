# Architecture Overview

Implementation mapping: see [implementation index](implementation_index.md) for planning documents linked to the Rust crates and the remaining Python packages. Migration ownership is tracked in [rust_migration_matrix.md](rust_migration_matrix.md).

## Design Philosophy

Inferra is a local-first runtime debugging system that operates on a strict separation of concerns.

The current target architecture is a Rust-primary control plane with Python
retained only behind an internal AI/analysis worker boundary. Rust owns the
operator-facing runtime shell, public API, Windows service shell, packaging, and
hot-path systems logic. Python remains responsible only for the retained
AI/explanation execution path while that logic is not yet ported.

- **Reproducible reasoning layer**: Correlation, hypothesis generation, scoring, and ranking are performed by rule-based algorithms with explicitly defined heuristics. Given the same input **and** the same internal state (baselines, scoring weights), the system produces the same output. However, several components involve bounded uncertainty: anomaly detection baselines shift over time, log normalization uses heuristic parsers that may misclassify ambiguous input, and deduplication relies on template-based fingerprinting that is imperfect for novel log formats. The system tracks and surfaces this uncertainty through data quality scores (see below) rather than pretending inputs are always clean.
- **Probabilistic shell**: An LLM layer exists solely to translate structured, already-decided results into human-readable explanations. It never influences decisions, rankings, or scores.

This separation exists because debugging trust requires auditability. An operator must be able to trace *why* the system ranked hypothesis A above hypothesis B without encountering a black-box step. Where the reasoning layer relies on heuristics (pattern matching, temporal proximity thresholds, fingerprint templates), those heuristics are documented, configurable, and their uncertainty is propagated to downstream consumers.

### Where Determinism Does and Does Not Hold


| Component                   | Deterministic?                              | Source of Uncertainty                                           |
| --------------------------- | ------------------------------------------- | --------------------------------------------------------------- |
| Log parsing / normalization | Reproducible but heuristic                  | Ambiguous formats, missing fields, encoding errors              |
| Deduplication               | Reproducible given same fingerprint logic   | Template may not correctly generalize novel log patterns        |
| Anomaly detection baselines | Stateful — changes over time                | EMA-based learning means baselines shift with incoming data     |
| Correlation edge creation   | Reproducible given same inputs + thresholds | Thresholds are heuristic; false correlations possible           |
| Signal detection            | Reproducible                                | Tag-based pattern matching may miss unusual phrasing            |
| Hypothesis composition      | Reproducible                                | Rule coverage is finite; novel failure modes may not match      |
| Scoring                     | Reproducible given same weights             | Weights are tunable via feedback; default weights are heuristic |
| Ranking                     | Fully deterministic                         | Pure sort — no uncertainty                                      |
| LLM explanation             | Non-deterministic                           | Temperature > 0; different runs produce different text          |


---

## System Boundary

Inferra runs entirely on the operator's machine (or within a single-node deployment). It has:

- **No outbound network dependency** for core operation (LLM can optionally call a local model or a remote API, but core logic never requires it).
- **No write access** to observed systems. Inferra is read-only with respect to the infrastructure it monitors.
- **No persistent external state**. All state is local: SQLite for structured data, filesystem for logs and indexes.

---

## High-Level Pipeline

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                           COLLECTION LAYER                                  │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐         │
│  │ Docker   │ │ Journald │ │ File     │ │ Proc/Sys │ │ App      │         │
│  │ Collector│ │ Collector│ │ Collector│ │ Collector│ │ Collector│         │
│  └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘         │
│       └─────────────┴─────────────┴─────────────┴─────────────┘             │
│                                   │                                         │
│                          Backpressure Buffer                                │
└──────────────────────────────────┬──────────────────────────────────────────┘
                                   │ raw events
                                   ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                        NORMALIZATION LAYER                                   │
│  ┌────────────────┐  ┌────────────────┐  ┌────────────────┐                 │
│  │ Log Parser      │  │ Schema Mapper  │  │ Field Enricher │                 │
│  │ (regex/grok)    │  │ (canonical     │  │ (service_id,   │                 │
│  │                 │  │  event schema) │  │  host, tags)   │                 │
│  └───────┬────────┘  └───────┬────────┘  └───────┬────────┘                 │
│          └───────────────────┴───────────────────┘                           │
│                              │                                               │
│              ┌───────────────┼───────────────┐                               │
│              ▼               ▼               ▼                               │
│     Deduplication     Noise Filter    Validation                             │
└─────────────────────────────┬────────────────────────────────────────────────┘
                              │ normalized events
                              ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                          STORAGE LAYER                                       │
│  ┌──────────────┐  ┌──────────────────┐  ┌────────────────┐                 │
│  │ Event Store   │  │ Time-Series      │  │ Service Graph  │                 │
│  │ (SQLite +     │  │ Ringbuffer       │  │ (in-memory     │                 │
│  │  append log)  │  │ (metric windows) │  │  adjacency)    │                 │
│  └──────┬───────┘  └───────┬──────────┘  └───────┬────────┘                 │
└─────────┼──────────────────┼──────────────────────┼──────────────────────────┘
          │                  │                      │
          ▼                  ▼                      ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                         ANALYSIS LAYER                                       │
│                                                                              │
│  ┌─────────────────┐     ┌──────────────────┐     ┌──────────────────┐      │
│  │ Anomaly          │     │ Correlation       │     │ Runtime Context  │      │
│  │ Detector         │────▶│ Engine            │◀────│ Builder          │      │
│  │                  │     │                   │     │                  │      │
│  │ - baseline model │     │ - temporal join   │     │ - resource state │      │
│  │ - deviation calc │     │ - service graph   │     │ - topology       │      │
│  │ - spike detect   │     │   traversal       │     │ - config snap    │      │
│  └─────────────────┘     │ - inference graph │     └──────────────────┘      │
│                           │   construction    │                               │
│                           └────────┬─────────┘                               │
│                                    │ incident clusters                       │
│                                    ▼                                         │
│                     ┌──────────────────────────┐                             │
│                     │ Incident Builder          │                             │
│                     │ - cluster → incident      │                             │
│                     │ - scope assignment        │                             │
│                     │ - lifecycle management    │                             │
│                     └────────────┬─────────────┘                             │
└──────────────────────────────────┼───────────────────────────────────────────┘
                                   │ incidents
                                   ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                        REASONING LAYER                                       │
│                                                                              │
│  ┌──────────────────┐    ┌──────────────────┐    ┌──────────────────┐       │
│  │ Hypothesis        │    │ Hypothesis        │    │ Scoring          │       │
│  │ Generator         │───▶│ Validator         │───▶│ Engine           │       │
│  │                   │    │                   │    │                  │       │
│  │ - signal detect   │    │ - evidence check  │    │ - weighted sum   │       │
│  │ - signal compose  │    │ - contradiction   │    │ - feedback tune  │       │
│  │ - graph traversal │    │   detection       │    │ - calibration    │       │
│  └──────────────────┘    │ - invalidation    │    └────────┬─────────┘       │
│                           └──────────────────┘             │                 │
│                                                            │ ranked list     │
│                    ┌───────────────────────────┐           │                 │
│                    │ Contradiction Handler      │◀──────────┘                 │
│                    │ - conflicting evidence     │                             │
│                    │ - ambiguity flagging       │                             │
│                    └───────────────────────────┘                             │
└──────────────────────────────────┬───────────────────────────────────────────┘
                                   │ scored hypotheses + evidence
                                   ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                       PRESENTATION LAYER                                     │
│                                                                              │
│  ┌──────────────────┐    ┌──────────────────┐    ┌──────────────────┐       │
│  │ Explanation       │    │ Incident          │    │ UI Server        │       │
│  │ Layer (LLM)       │    │ Lifecycle Mgr     │    │ (local web)      │       │
│  │                   │    │                   │    │                  │       │
│  │ - prompt build    │    │ - state machine   │    │ - dashboard      │       │
│  │ - guardrails      │    │ - merge/split     │    │ - incident view  │       │
│  │ - citation check  │    │ - resolution      │    │ - timeline       │       │
│  └──────────────────┘    └──────────────────┘    │ - graph view     │       │
│                                                   └──────────────────┘       │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## Module Boundaries

Each subsystem communicates through typed data contracts (see `data_flow_contracts.md`). No module holds a direct reference to another module's internals. Communication is via:

1. **Function call with typed structs** for synchronous in-process calls (collectors → normalization → storage).
2. **Internal event bus** for asynchronous decoupled flow (storage writes → analysis triggers).
3. **Query interface** for on-demand data access (hypothesis engine querying event store).

### Module Dependency Rules

- Collection layer depends on: nothing (leaf inputs)
- Normalization layer depends on: event model schema
- Storage layer depends on: event model schema
- Analysis layer depends on: storage layer (read), event model
- Reasoning layer depends on: analysis layer output, storage layer (read)
- Presentation layer depends on: reasoning layer output, storage layer (read)

Circular dependencies are forbidden. Data flows strictly downward through the pipeline.

---

## Concurrency Model

Inferra uses a local-first, small-footprint runtime with a single Rust primary
process:

- **Collectors and runtime services** run as Rust async tasks under Tokio.
- **Normalization** is a synchronous pipeline invoked per-event (low latency requirement: <5ms per event).
- **Analysis** runs on a periodic tick (configurable, default 5 seconds) or is triggered when event volume exceeds a threshold.
- **Reasoning** is triggered per-incident (on incident creation or update).
- **Explanation / AI investigation** is triggered on-demand through native Rust provider integration when enabled.

The public control plane remains local and self-contained. Operator-facing
runtime behavior stays inside the native Rust process so packaging, service
hosting, and API delivery no longer depend on deprecated Python code.

---

## Deployment Topology

```
Single machine:
┌─────────────────────────────────────────┐
│  Inferra Rust Process                   │
│  ├── Collector tasks                    │
│  ├── Normalization / storage            │
│  ├── Analysis / runtime context         │
│  ├── Public API + Web UI host           │
│  └── Service / CLI shell                │
│                                         │
│  Optional internal worker:              │
│  └── Python AI / explanation service    │
│                                         │
│  Storage:                               │
│  ├── ./data/events.db    (SQLite)       │
│  ├── ./data/incidents.db (SQLite)       │
│  ├── ./data/metrics/     (ringbuffers)  │
│  └── ./data/baselines/   (JSON)         │
└─────────────────────────────────────────┘
```

No message queues. No container orchestration. The intentional runtime boundary
is one local Rust control-plane process plus an optional local Python worker for
AI/analysis.

---

## Scaling Path (Not Now, But Not Never)

The single-machine design is the correct MVP architecture. But it must not become a cage. The following design decisions preserve future extensibility without adding current complexity:

### What's Already Extensible


| Aspect          | Current                | Growth Path                                                                                                           |
| --------------- | ---------------------- | --------------------------------------------------------------------------------------------------------------------- |
| Collectors      | In-process async tasks | Can become separate processes feeding a local queue (Unix socket, named pipe)                                         |
| Event storage   | SQLite                 | Schema is compatible with PostgreSQL migration; query interface is abstracted behind `EventStore` protocol            |
| Analysis engine | Single periodic tick   | Can be split into independent workers per strategy (correlation, anomaly, inference) communicating via shared storage |
| Service graph   | In-memory NetworkX     | Graph structure is serializable; can move to a graph database if node count exceeds ~500                              |
| Scoring weights | Local JSON file        | Can become a shared config store if multiple instances need synchronized weights                                      |


### What Would Require Architectural Changes


| Aspect                    | Current Limitation            | Required for Growth                                                                                |
| ------------------------- | ----------------------------- | -------------------------------------------------------------------------------------------------- |
| Multi-host observation    | Single machine only           | Would need an event relay agent on each host, plus a central correlation instance                  |
| Event throughput >100/s   | Python single-process ceiling | Would need Rust/Go collector sidecar writing to shared storage, Python processing off the hot path |
| >200 concurrent incidents | CPU budget exceeded           | Would need incident prioritization or sharding by service group                                    |
| Multi-user access         | No auth, localhost only       | Would need auth layer, RBAC, and possibly a shared backend                                         |


### Design Rules That Preserve Growth

1. **All inter-module communication uses typed contracts** (see `data_flow_contracts.md`). Modules can be replaced or relocated independently.
2. **Storage is accessed through protocol interfaces**, not direct SQL. Swapping SQLite for PostgreSQL requires implementing the same protocol.
3. **No module holds in-memory state that isn't recoverable from storage.** Crash → restart → resume. This is the foundation for any future distribution.
4. **Configuration is external** (`inferra.toml`), not compiled. Adding a new collector, changing storage backend, or switching LLM provider is a config change.

The honest position: Inferra v1 is a single-machine tool. It will remain performant and valuable at that scale. If it needs to grow, the abstractions are in place to grow it without a rewrite — but the growth itself requires engineering work that isn't free.

---

## Technology Stack


| Component        | Technology                            | Rationale                                         |
| ---------------- | ------------------------------------- | ------------------------------------------------- |
| Language         | Python 3.11+                          | Ecosystem for log parsing, async, data processing |
| Storage          | SQLite (WAL mode)                     | Local-first, zero-config, concurrent reads        |
| Event bus        | Internal asyncio queues               | No external dependencies                          |
| Web UI           | FastAPI + static frontend             | Minimal footprint, WebSocket for live updates     |
| LLM integration  | Pluggable (local ollama / remote API) | User choice, not a hard dependency                |
| Graph operations | NetworkX (in-memory)                  | Sufficient for single-machine scale               |
| Metric storage   | Custom ringbuffer                     | Fixed memory, no unbounded growth                 |


---

## Startup Sequence

1. Load configuration (`inferra.toml`)
2. Initialize storage (create SQLite tables if missing, verify schema version)
3. Load baseline models from `./data/baselines/` (or initialize empty)
4. Discover and start collectors (based on config: which Docker socket, which log paths, etc.)
5. Start normalization pipeline consumer
6. Start analysis engine tick loop
7. Start web UI server
8. Log: `Inferra ready. Observing.`

---

## Shutdown Sequence

1. Signal collectors to stop (drain in-flight events)
2. Flush normalization pipeline
3. Persist any in-memory baselines to disk
4. Commit pending SQLite transactions
5. Close web UI server
6. Exit

