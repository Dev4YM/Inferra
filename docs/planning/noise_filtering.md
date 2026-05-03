# Noise Filtering

## Problem

Not all events are diagnostically useful. Many systems emit high volumes of routine log lines that obscure actual failure signals. Examples:

- Periodic health check passes ("GET /healthz 200 OK" every 5 seconds)
- Debug-level logging left enabled in production
- Informational startup banners repeated on each restart
- Scheduled job completion messages ("Cron job XYZ completed successfully")
- Metric heartbeats

Without filtering, these events:

1. Waste storage and query performance
2. Dilute correlation signals (noise co-occurs with everything)
3. Inflate frequency counts, distorting hypothesis scoring
4. Clutter the timeline view for operators

---

## Filtering Strategy

Noise filtering operates at three levels, applied in order after deduplication:

```
NormalizedEvent (post-dedup)
    │
    ├── Level 1: Static Rules (blocklist / allowlist)
    │
    ├── Level 2: Adaptive Frequency Filter
    │
    ├── Level 3: Diagnostic Relevance Scorer
    │
    ▼
Event stored (with noise_score metadata)
   or
Event suppressed (counted but not stored)
```

---

## Level 1: Static Rules

User-defined and built-in rules that unconditionally include or exclude events.

### Blocklist

Events matching blocklist patterns are suppressed. Suppressed events are counted (for volume metrics) but not stored or analyzed.

```toml
[[noise_filter.blocklist]]
pattern = "GET /healthz"
reason = "routine health check"

[[noise_filter.blocklist]]
pattern = "^\\s*$"
reason = "empty log lines"

[[noise_filter.blocklist]]
service_id = "envoy"
severity_max = "DEBUG"
reason = "envoy debug logging too verbose"
```

### Allowlist

Events matching allowlist patterns are always stored, regardless of other filtering. This prevents important events from being accidentally filtered.

```toml
[[noise_filter.allowlist]]
severity_min = "ERROR"
reason = "always keep errors and above"

[[noise_filter.allowlist]]
tags = ["oom", "crash", "restart"]
reason = "always keep critical system events"

[[noise_filter.allowlist]]
pattern = "FATAL|panic|segfault"
reason = "always keep fatal signals"
```

**Rule evaluation order**:

1. Check allowlist first. If any allowlist rule matches → STORE unconditionally.
2. Check blocklist. If any blocklist rule matches → SUPPRESS.
3. Proceed to Level 2.

---

## Level 2: Adaptive Frequency Filter

Filters events that appear at a predictably high, stable rate — they are routine, not anomalous.

### Mechanism

For each fingerprint, the system tracks a rolling rate (events per minute over the last 30 minutes). If the rate is:

- **Stable** (coefficient of variation < 0.3) AND
- **High** (above a configurable threshold, default: 10 per minute)

Then the fingerprint is classified as **routine noise**. Subsequent events with this fingerprint are sampled rather than stored individually:

- 1 in N events is stored (where N = rate / target_rate, and target_rate default = 1/min)
- Stored events are tagged `_noise_sampled: true` and `_noise_sample_rate: N`

### Rate Tracking

```python
class FrequencyTracker:
    """Track per-fingerprint event rates using sliding windows."""

    def __init__(self, window_minutes: int = 30, bucket_seconds: int = 60):
        self.window_minutes = window_minutes
        self.bucket_seconds = bucket_seconds
        self._rates: dict[str, RateWindow] = {}

    def record(self, fingerprint: str, timestamp: datetime) -> None:
        """Record an event occurrence."""
        ...

    def is_routine(self, fingerprint: str) -> tuple[bool, float]:
        """Returns (is_routine, rate_per_minute)."""
        rate = self._rates.get(fingerprint)
        if rate is None:
            return False, 0.0

        mean_rate = rate.mean_per_minute()
        cv = rate.coefficient_of_variation()

        if mean_rate > self.high_rate_threshold and cv < self.stability_threshold:
            return True, mean_rate
        return False, mean_rate
```

### Adaptation Behavior

- New fingerprints are never classified as routine (need at least `window_minutes` of data).
- If a previously routine fingerprint's rate spikes (CV exceeds threshold), it is reclassified as non-routine. This catches anomalous bursts of normally-routine events.
- If a previously routine fingerprint stops appearing, its tracker expires after 2× `window_minutes` and is removed.

---

## Level 3: Diagnostic Relevance Scorer

For events that pass Levels 1 and 2, a relevance score is computed. This score does not suppress events, but it annotates them with a `noise_score` in structured_data (0.0 = pure noise, 1.0 = highly relevant). Downstream systems (correlation, scoring) can use this to weight events.

### Scoring Factors


| Factor           | Weight | Description                                                |
| ---------------- | ------ | ---------------------------------------------------------- |
| Severity         | 0.30   | ERROR=1.0, CRITICAL=1.0, WARN=0.6, INFO=0.2, DEBUG=0.1     |
| Temporal novelty | 0.25   | Is this event occurring at an unusual time? (vs. baseline) |
| Rate anomaly     | 0.20   | Is this fingerprint occurring at an unusual rate?          |
| Tag relevance    | 0.15   | Events with failure-indicating tags score higher           |
| Co-occurrence    | 0.10   | Does this event co-occur with other anomalous events?      |


```python
def compute_noise_score(event: NormalizedEvent, context: FilterContext) -> float:
    severity_score = {
        Severity.DEBUG: 0.1,
        Severity.INFO: 0.2,
        Severity.WARN: 0.6,
        Severity.ERROR: 1.0,
        Severity.CRITICAL: 1.0,
    }[event.severity]

    temporal_score = context.temporal_novelty(event.fingerprint, event.timestamp)
    rate_score = context.rate_anomaly(event.fingerprint)

    FAILURE_TAGS = {"oom", "crash", "restart", "connection_refused", "timeout", "disk_full"}
    tag_score = 1.0 if event.tags & FAILURE_TAGS else 0.2

    cooccurrence_score = context.cooccurrence_anomaly(event.timestamp, event.service_id)

    return (
        0.30 * severity_score
        + 0.25 * temporal_score
        + 0.20 * rate_score
        + 0.15 * tag_score
        + 0.10 * cooccurrence_score
    )
```

### Storage Decision

Events with `noise_score < noise_threshold` (default: 0.15) may be downsampled at the storage layer during high-volume periods. Under normal volume, all events are stored regardless of noise score. The threshold only activates when event volume exceeds `high_volume_threshold` (default: 100 events/sec).

---

## Known Noise Registry

A persistent registry of fingerprints that have been consistently classified as routine noise over multiple sessions. Stored in `./data/noise_registry.json`:

```json
{
    "schema_version": 1,
    "entries": [
        {
            "fingerprint": "a1b2c3d4...",
            "template": "GET <PATH> <NUM> <STR>",
            "service_id": "nginx",
            "mean_rate_per_minute": 12.5,
            "first_seen": "2026-04-28T10:00:00Z",
            "last_confirmed": "2026-05-01T15:00:00Z",
            "confidence": 0.95
        }
    ]
}
```

On startup, the frequency tracker is seeded with registry data, allowing it to immediately classify known-noise fingerprints without waiting for the full window to populate.

The registry is updated hourly. Entries not confirmed in 7 days are removed (the noise pattern may no longer apply).

---

## Configuration

```toml
[noise_filter]
enabled = true

# Level 1
blocklist_enabled = true
allowlist_enabled = true

# Level 2
adaptive_enabled = true
frequency_window_minutes = 30
high_rate_threshold_per_minute = 10.0
stability_threshold_cv = 0.3
routine_sample_target_per_minute = 1.0

# Level 3
relevance_scoring_enabled = true
noise_threshold = 0.15
high_volume_events_per_second = 100

# Registry
registry_enabled = true
registry_expiry_days = 7
```

---

## Failure Modes


| Failure                               | Impact                                                           | Mitigation                                               |
| ------------------------------------- | ---------------------------------------------------------------- | -------------------------------------------------------- |
| Blocklist rule too broad              | Important events suppressed                                      | Allowlist takes precedence; ERROR+ always stored         |
| Adaptive filter misclassifies a burst | Anomalous events filtered as routine                             | CV check reclassifies when rate pattern changes          |
| Noise score miscalculated             | Low-relevance events stored, high-relevance events underweighted | Score is advisory, not suppressive (under normal volume) |
| Registry stale after config change    | Old noise patterns applied to new service                        | 7-day expiry limits staleness                            |


---

## Metrics


| Metric                              | Description                                  |
| ----------------------------------- | -------------------------------------------- |
| `noise_filter_suppressed_total`     | Events suppressed by static rules            |
| `noise_filter_sampled_total`        | Events downsampled by adaptive filter        |
| `noise_filter_passed_total`         | Events that passed all filters               |
| `noise_filter_routine_fingerprints` | Count of fingerprints classified as routine  |
| `noise_score_histogram`             | Distribution of noise scores (p50, p90, p99) |


