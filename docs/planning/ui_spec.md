# UI Specification

## Design Goal

The UI exists to answer one question: **"Why is my system failing?"**

Everything else is secondary. If the operator has to dig through logs, filter events, or mentally reconstruct timelines, the UI has failed.

---

## Primary Interaction Loop

This is the exact sequence the operator follows. Every screen, component, and layout decision serves this loop.

```
Step 1: GLANCE (Dashboard, <3 seconds)
  "Is something wrong?"
  → Green header = no. Stop here.
  → Red/yellow card = yes. Click it.

Step 2: UNDERSTAND (Incident Detail, <30 seconds)
  "What does the system think happened?"
  → Read the #1 hypothesis title and score.
  → Glance at the evidence timeline.
  → Check contradictions (if any).

Step 3: VERIFY (Evidence + Suggested Checks, 1-5 minutes)
  "Do I believe this hypothesis?"
  → Review the score breakdown — which signals are strong, which are weak?
  → Expand the inference graph — does the causal chain make sense?
  → Copy-paste suggested diagnostic commands and run them.
  → Read alternative hypotheses — is #2 or #3 more plausible?

Step 4: ACT (Resolution, <30 seconds)
  "What do I do about it?"
  → If hypothesis was right: click "Resolve: Correct" (feeds scoring calibration).
  → If a different hypothesis was right: select it and click "Resolve: This One".
  → If none were right: click "Resolve: None Correct" (no calibration, incident archived).
  → Optionally add a note about the actual root cause (free text, stored for future reference).
```

**Critical metric**: Step 1 → Step 3 should take <60 seconds for a straightforward incident. If it takes longer, the UI is too complex or the hypothesis is too vague.

### What This Means for UI Design

| Principle | Implementation |
|---|---|
| No log digging by default | Events are pre-filtered, sorted, and annotated by the hypothesis engine. Raw logs are a tab, not the default view. |
| Answer first, evidence second | The hypothesis title and score are visible before any evidence. The operator reads the conclusion first, then evaluates the evidence. |
| Copy-paste debugging | Every `suggested_check` is in a code block with a one-click copy button. |
| Feedback is 2 clicks | Resolution buttons are always visible at the bottom of the incident view. No modals, no multi-step forms. |
| Progressive disclosure | Dashboard → Incident → Hypothesis → Evidence → Raw Event. Each click adds one level of detail. |

---

## Architecture

The UI is a local web application served by Inferra's built-in HTTP server.

| Component | Technology | Rationale |
|---|---|---|
| Server | FastAPI (Python) | Consistent with backend; async WebSocket support |
| Frontend | React/Vite source under `src/web/frontend`, built to `src/web/ui_dist` | One official frontend source with packaged static output |
| Reactivity | WebSocket for live updates, REST for initial load | Incidents and events update in real-time |
| Styling | Product-specific CSS and component structure | Dense operator UI with room for developer detail |
| Charts | Lightweight charting (uPlot or Chart.js) | Time-series visualization for metrics/timelines |

**Access**: `http://localhost:{port}` (default port: 7433). No authentication (local-only tool).

---

## Page Structure

```
┌─────────────────────────────────────────────────────────────────┐
│ HEADER                                                           │
│ Inferra ▪ Status: Observing ▪ Events: 142/sec ▪ Active: 3      │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│ ┌─── NAV ───┐ ┌────────────── MAIN CONTENT ─────────────────┐  │
│ │ Dashboard  │ │                                              │  │
│ │ Incidents  │ │  (varies by selected page)                   │  │
│ │ Services   │ │                                              │  │
│ │ Timeline   │ │                                              │  │
│ │ Settings   │ │                                              │  │
│ │            │ │                                              │  │
│ │ ── Health ─│ │                                              │  │
│ │ Collectors │ │                                              │  │
│ │ Baselines  │ │                                              │  │
│ │ Calibration│ │                                              │  │
│ └────────────┘ └──────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

---

## Page 1: Dashboard

The landing page. Answers "Is something wrong?" at a glance.

### Components

**1. Status Bar** (always visible in header)
- System state: `Observing` (green) | `Degraded` (yellow) | `Collector Error` (red)
- Event throughput: `142 events/sec`
- Active incidents count: `3 active`
- Baseline status: `Learning (4h remaining)` or `Stable`

**2. Incident Summary Cards**
```
┌────────────────────────────────────────────────┐
│ 🔴 CRITICAL  api-gateway, postgres             │
│ "postgres failure cascaded to 3 services"      │
│ Started 5 min ago │ 47 events │ Score: 0.84    │
│ [View Details →]                                │
├────────────────────────────────────────────────┤
│ 🟡 WARN  payment-service                       │
│ "Elevated error rate on payment-service"        │
│ Started 12 min ago │ 15 events │ Score: 0.52   │
│ [View Details →]                                │
└────────────────────────────────────────────────┘
```

Cards are sorted by severity, then by score. Maximum 10 cards on dashboard; older incidents accessible via Incidents page.

**3. Service Health Grid**
A grid showing all known services with color-coded health:
- Green: normal (anomaly score < 0.3)
- Yellow: elevated (anomaly score 0.3–0.6)
- Red: anomalous (anomaly score > 0.6)
- Gray: no data / learning

```
┌──────────┬──────────┬──────────┬──────────┐
│ api-gw   │ users    │ postgres │ redis    │
│  ● OK    │  ● OK    │  ● CRIT  │  ● WARN  │
│  2 ev/s  │  0.5/s   │  12/s    │  3/s     │
└──────────┴──────────┴──────────┴──────────┘
```

Clicking a service navigates to the Service Detail page.

**4. Event Rate Chart**
A time-series chart showing total event volume (stacked by severity) over the last 60 minutes. Anomalous regions are highlighted with a subtle red background.

---

## Page 2: Incident Detail

The primary analysis view. Answers "Why is it wrong?"

### Layout

```
┌──────────────────────────────────────────────────────────┐
│ INCIDENT: postgres failure cascaded to 3 services        │
│ Status: INVESTIGATING │ Severity: CRITICAL │ 47 events   │
├──────────────────────────────────────────────────────────┤
│                                                           │
│ ┌─── HYPOTHESES PANEL (left, 40%) ─────────────────────┐ │
│ │                                                       │ │
│ │ ▼ #1 (Score: 0.84, High Confidence)                  │ │
│ │   postgres failure cascaded to 3 services             │ │
│ │   Type: dependency_failure                            │ │
│ │                                                       │ │
│ │   Score Breakdown:                                    │ │
│ │   ████████░░ temporal_alignment: 0.92                 │ │
│ │   ███████░░░ correlation_strength: 0.78               │ │
│ │   █████░░░░░ frequency_weight: 0.65                   │ │
│ │   ████████░░ dependency_proximity: 0.88               │ │
│ │   ██████░░░░ evidence_coverage: 0.71                  │ │
│ │   ████░░░░░░ anomaly_severity: 0.55                   │ │
│ │                                                       │ │
│ │   ⚠ 1 contradiction: health check passed at 14:32    │ │
│ │                                                       │ │
│ │   Suggested checks:                                   │ │
│ │   $ docker logs postgres_container --tail 100         │ │
│ │   $ docker inspect postgres_container                 │ │
│ │                                                       │ │
│ │ ▷ #2 (Score: 0.52, Medium Confidence)                │ │
│ │   Memory exhaustion on host                           │ │
│ │                                                       │ │
│ │ ▷ #3 (Score: 0.31, Low Confidence)                   │ │
│ │   Configuration change on api-gateway                 │ │
│ └───────────────────────────────────────────────────────┘ │
│                                                           │
│ ┌─── EVIDENCE PANEL (right, 60%) ──────────────────────┐ │
│ │                                                       │ │
│ │ [Timeline] [Logs] [Graph] [Explanation]  ← tabs       │ │
│ │                                                       │ │
│ │ (content varies by selected tab)                      │ │
│ │                                                       │ │
│ └───────────────────────────────────────────────────────┘ │
│                                                           │
│ ┌─── ACTIONS ──────────────────────────────────────────┐  │
│ │ [Resolve: Correct ▾] [Resolve: None Correct] [Skip]  │  │
│ └───────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────┘
```

### Evidence Panel Tabs

**Timeline Tab**
A vertical timeline showing events in chronological order. Key events (flagged by hypothesis) are highlighted.

```
14:30:02  ● ERROR  postgres:     connection refused: too many clients
14:30:03  ● ERROR  postgres:     connection refused: too many clients  (×12)
14:30:05  ● ERROR  api-gateway:  upstream timeout: postgres
14:30:06  ● ERROR  user-service: database connection failed
14:30:08  ● WARN   api-gateway:  circuit breaker tripped for postgres
14:30:10  ● ERROR  api-gateway:  HTTP 503 returned to client
14:30:15  ○ INFO   postgres:     health check passed ⚠ (contradiction)
```

Each entry is clickable to expand the full event detail.

**Logs Tab**
Raw event messages for the incident, filterable by service and severity.

```
Service: [All ▾]  Severity: [WARN+ ▾]  Search: [________]

14:30:02 ERROR postgres     connection refused: too many clients already
14:30:05 ERROR api-gateway  upstream service timeout after 5000ms: postgres:5432
14:30:06 ERROR user-service  sqlalchemy.exc.OperationalError: could not connect to server
...
```

**Graph Tab**
Visual representation of the inference graph. Nodes are events (colored by severity), edges are inferred sequences (thickness = plausibility). Each edge shows its assumptions on hover.

```
[postgres: conn refused] ──(0.92)──▶ [api-gw: timeout] ──(0.85)──▶ [api-gw: 503]
                         ──(0.88)──▶ [user-svc: db conn failed]
```

Implementation: SVG-based graph rendered client-side. For incidents with >50 events, only the top-20 events by severity are shown in the graph (with an "expand" option).

**Explanation Tab**
The LLM-generated (or template-generated) explanation, rendered as formatted text.

```
## Summary
A connection limit exhaustion on postgres caused cascading failures across
api-gateway and user-service. The root cause appears to be an elevated number
of database connections, likely from a connection leak or traffic spike.

## What Happened
At 14:30:02, postgres began rejecting new connections with "too many clients"
errors. Within 3 seconds, api-gateway experienced upstream timeouts and began
returning HTTP 503 errors. user-service also lost its database connection.

## Evidence Assessment
Strong evidence: 12 connection refused events from postgres precede all
downstream errors. The temporal alignment is tight (3 seconds from root to
symptoms). One contradiction noted: a health check passed at 14:30:15, suggesting
the issue may have been intermittent.

## Recommended Actions
1. Check postgres connection count and limits
2. Check for connection leaks in application code
3. Review postgres max_connections setting
```

---

## Page 3: Services

Overview of all discovered services and their current health.

### Components

**Service Table**
| Service | Status | Events/min | Error Rate | Anomaly Score | Last Incident |
|---|---|---|---|---|---|
| api-gateway | Healthy | 120 | 0.5% | 0.12 | 2h ago |
| postgres | Critical | 840 | 45% | 0.94 | Active |
| user-service | Degraded | 45 | 12% | 0.67 | Active |
| redis | Healthy | 30 | 0% | 0.05 | Never |

**Service Detail** (on click)
- Event volume chart (last 24h)
- Error rate chart (last 24h)
- Baseline comparison overlay
- Related incidents list
- Container details (from runtime context)
- Service dependencies (from topology)

---

## Page 4: Timeline

A global timeline view across all services, showing events and incidents on a shared time axis. Useful for understanding cross-service failures.

**Implementation**: Horizontal scrolling timeline with swim lanes (one per service). Events are dots/markers colored by severity. Incidents are shaded regions spanning the affected time range.

```
Time:    14:28    14:29    14:30    14:31    14:32
         │        │        │        │        │
postgres ─────────────●●●●●●●──●●────────────────
api-gw   ─────────────────●●●●●●──●──────────────
user-svc ──────────────────●●●──●─────────────────
redis    ─────────────────────────────────────────

                    └──── Incident #7 ────┘
```

---

## Page 5: Settings

**Configuration sections**:
- Collectors: enable/disable, configure paths/sockets
- Normalization: custom log formats, tag rules
- Analysis: correlation windows, anomaly thresholds
- Explanation: LLM provider, model selection, API key
- Display: theme (light/dark), refresh rate, max timeline events
- About: version, license, system health

---

## WebSocket API

Live updates are pushed to the UI via WebSocket:

```python
# Server → Client messages
{"type": "incident_created", "incident": {...}}
{"type": "incident_updated", "incident_id": "...", "changes": {...}}
{"type": "incident_resolved", "incident_id": "..."}
{"type": "event_count", "events_per_second": 142, "by_severity": {...}}
{"type": "collector_health", "collectors": [...]}
{"type": "explanation_ready", "incident_id": "...", "explanation": {...}}
```

```python
# Client → Server messages
{"type": "resolve_incident", "incident_id": "...", "feedback": {...}}
{"type": "subscribe_incident", "incident_id": "..."}  # get detailed updates for one incident
{"type": "unsubscribe_incident", "incident_id": "..."}
```

---

## REST API

```
GET  /api/incidents                    # list active incidents
GET  /api/incidents/{id}               # full incident detail
GET  /api/incidents/{id}/hypotheses    # hypotheses for incident
GET  /api/incidents/{id}/explanation   # explanation for incident
GET  /api/incidents/{id}/events        # events in incident (paginated)
POST /api/incidents/{id}/resolve       # resolve incident with feedback
GET  /api/services                     # list known services
GET  /api/services/{id}                # service detail with health
GET  /api/services/{id}/events         # events for service (paginated)
GET  /api/events?start=...&end=...     # query events by time range
GET  /api/health                       # system health (collectors, storage, baselines)
GET  /api/config                       # current configuration
PUT  /api/config                       # update configuration (selective)
```

---

## Interaction Principles

1. **No log digging**: The primary path is Dashboard → Incident → Hypothesis → Explanation. The operator should reach understanding without ever opening a terminal.

2. **Progressive disclosure**: Dashboard shows summary. Clicking reveals hypotheses. Expanding reveals evidence. Clicking evidence shows raw event. Each level adds detail without overwhelming.

3. **Copy-ready diagnostics**: Every "suggested check" command is in a `<code>` block with a copy button. The operator can paste directly into a terminal.

4. **Non-blocking feedback**: The "Resolve" action is quick (2 clicks). Detailed feedback is optional. The system must not create friction around resolution.

5. **Accessible offline**: Since Inferra runs locally, the UI must work without any CDN-hosted resources. All CSS, JS, and fonts are bundled with the application.

---

## Performance Targets

| Metric | Target |
|---|---|
| Initial page load | <500ms |
| WebSocket update latency | <100ms (local) |
| Dashboard render with 10 incidents | <200ms |
| Timeline render with 200 events | <500ms |
| Graph render with 50 nodes | <300ms |

---

## Responsive Design

The UI is designed for desktop use (1280px+ width). It degrades gracefully on smaller screens but is not optimized for mobile. This is a professional debugging tool, not a consumer product.
