# Anomaly Detection

## Purpose

The anomaly detector identifies when system behavior deviates from established baselines. It answers: "Is what I'm seeing right now abnormal?"

Unlike the correlation engine (which groups events) or the inference graph (which orders them into plausible sequences), the anomaly detector compares current observations against historical norms. A spike in error rate, an unusual volume of log output, or a service producing events at a time it normally doesn't — these are anomaly signals that feed into correlation strength and hypothesis scoring.

---

## Design Principles

1. **Statistical, not ML-based**: Baselines are computed from rolling statistics (mean, standard deviation, percentiles). No trained models. This is intentional — Inferra is local-first and must work on a fresh install with zero training data.
2. **Graceful cold start**: With no historical data, the detector operates in learning mode (no anomalies reported for the first `cold_start_hours`). After that, baselines are progressively refined.
3. **Seasonal awareness**: Baselines are segmented by hour-of-week (168 buckets) to capture daily and weekly patterns.
4. **Conservative**: The system prefers false negatives (missing a real anomaly) over false positives (alerting on normal behavior). Operators build trust through accuracy.

---

## Metrics Tracked

For each `service_id`, the detector maintains rolling baselines for:

| Metric | Definition | Source |
|---|---|---|
| `event_volume` | Total events per 5-minute bucket | Event store count query |
| `error_rate` | Fraction of events with severity >= ERROR | Event store count query |
| `warn_rate` | Fraction of events with severity >= WARN | Event store count query |
| `unique_fingerprints` | Count of distinct fingerprints per bucket | Dedup tracker |
| `new_fingerprint_rate` | Fingerprints seen for the first time | Dedup tracker |
| `restart_count` | Events tagged `restart` per bucket | Tag count query |
| `mean_severity` | Average severity level per bucket | Computed |

System-wide (not per-service):

| Metric | Definition | Source |
|---|---|---|
| `total_event_volume` | Total events across all services per bucket | Event store |
| `active_services` | Count of services emitting events | Event store |
| `cross_service_error_rate` | Fraction of services with error rate > threshold | Computed |

---

## Baseline Model

### Structure

Each metric has a baseline consisting of 168 hourly buckets (7 days × 24 hours), representing the expected value for each hour of the week.

```python
@dataclass
class BaselineMetric:
    metric_name: str
    service_id: str
    buckets: list[float]           # 168 values: expected metric value per hour-of-week
    stddev: list[float]            # 168 values: expected standard deviation per bucket
    sample_counts: list[int]       # how many observations contributed to each bucket
    min_samples_for_confidence: int = 4  # need this many samples before baseline is trusted
    last_updated: datetime | None = None
```

### Bucket Indexing

```python
def hour_of_week_index(dt: datetime) -> int:
    """Map a datetime to a 0-167 bucket index."""
    return dt.weekday() * 24 + dt.hour
```

### Baseline Update (Exponential Moving Average)

Baselines are updated every hour with the latest observed data:

```python
def update_baseline(baseline: BaselineMetric, bucket_idx: int,
                     observed_value: float, alpha: float = 0.1) -> None:
    """Update baseline using EMA. Alpha controls how fast old data decays."""
    if baseline.sample_counts[bucket_idx] == 0:
        # First observation: initialize directly
        baseline.buckets[bucket_idx] = observed_value
        baseline.stddev[bucket_idx] = 0.0
    else:
        old_mean = baseline.buckets[bucket_idx]
        new_mean = alpha * observed_value + (1 - alpha) * old_mean
        # Update stddev using Welford-like approach
        deviation = abs(observed_value - old_mean)
        old_std = baseline.stddev[bucket_idx]
        new_std = alpha * deviation + (1 - alpha) * old_std

        baseline.buckets[bucket_idx] = new_mean
        baseline.stddev[bucket_idx] = new_std

    baseline.sample_counts[bucket_idx] += 1
    baseline.last_updated = datetime.utcnow()
```

**Alpha = 0.1**: Recent data contributes 10% of the new baseline, giving an effective memory of ~10 update cycles (~10 weeks for weekly patterns). This means the baseline adapts to gradual changes but isn't swayed by one-time anomalies.

---

## Anomaly Scoring

For each metric observation, the anomaly score represents how many standard deviations the observation is from the baseline:

```python
def compute_anomaly_score(observed: float, baseline: BaselineMetric,
                           bucket_idx: int) -> AnomalyResult:
    expected = baseline.buckets[bucket_idx]
    std = baseline.stddev[bucket_idx]
    samples = baseline.sample_counts[bucket_idx]

    # Not enough data: no anomaly call
    if samples < baseline.min_samples_for_confidence:
        return AnomalyResult(score=0.0, confidence="insufficient_data", z_score=0.0)

    # Avoid division by zero for metrics with no variance
    if std < 1e-6:
        if abs(observed - expected) < 1e-6:
            return AnomalyResult(score=0.0, confidence="normal", z_score=0.0)
        else:
            return AnomalyResult(score=1.0, confidence="high", z_score=float('inf'))

    z_score = (observed - expected) / std

    # Anomaly score: normalized to 0.0-1.0 using sigmoid-like mapping
    # z=0 → 0.0 (normal), z=2 → 0.5, z=4 → 0.88, z=6 → 0.97
    raw_score = 1.0 - 1.0 / (1.0 + (abs(z_score) / 3.0) ** 2)

    confidence = "high" if samples >= 20 else "medium" if samples >= 8 else "low"

    return AnomalyResult(
        score=raw_score,
        confidence=confidence,
        z_score=z_score,
        expected=expected,
        observed=observed,
        std=std,
    )

@dataclass
class AnomalyResult:
    score: float              # 0.0 (normal) to 1.0 (extreme anomaly)
    confidence: str           # "insufficient_data" | "low" | "medium" | "high"
    z_score: float            # raw z-score (can be negative for below-baseline)
    expected: float = 0.0
    observed: float = 0.0
    std: float = 0.0
```

### Anomaly Thresholds

| Z-Score Range | Anomaly Score | Interpretation |
|---|---|---|
| |z| < 1.5 | 0.0 – 0.2 | Normal variation |
| 1.5 ≤ |z| < 3.0 | 0.2 – 0.5 | Mild anomaly |
| 3.0 ≤ |z| < 5.0 | 0.5 – 0.85 | Significant anomaly |
| |z| ≥ 5.0 | 0.85 – 1.0 | Extreme anomaly |

---

## Composite Anomaly Score

Each service gets a composite anomaly score combining all its metrics:

```python
def composite_anomaly_score(service_id: str, metric_results: dict[str, AnomalyResult]) -> float:
    weights = {
        "error_rate": 0.35,
        "event_volume": 0.20,
        "new_fingerprint_rate": 0.20,
        "restart_count": 0.15,
        "warn_rate": 0.10,
    }

    weighted_sum = 0.0
    total_weight = 0.0
    for metric_name, result in metric_results.items():
        if result.confidence == "insufficient_data":
            continue
        w = weights.get(metric_name, 0.1)
        weighted_sum += w * result.score
        total_weight += w

    if total_weight == 0:
        return 0.0
    return weighted_sum / total_weight
```

---

## Detection Modes

### 1. Spike Detection
Sudden increase in metric value (error rate jumps from 1% to 30%).

Detected by: high z-score in the current bucket.

### 2. Sustained Shift Detection
Metric elevated for multiple consecutive buckets (error rate at 5% for the last 3 hours instead of the usual 1%).

```python
def sustained_shift(metric: str, service_id: str, lookback_buckets: int = 6) -> float:
    """Average anomaly score over the last N buckets. High value = sustained anomaly."""
    recent_scores = [compute_anomaly_score(observed, baseline, idx)
                     for observed, idx in last_n_buckets(metric, service_id, lookback_buckets)]
    return mean(s.score for s in recent_scores)
```

### 3. Absence Detection
A service that normally emits events has gone silent.

```python
def absence_detection(service_id: str, expected_volume: float, observed_volume: float) -> float:
    """Score for unexpected silence. Returns 0.0 if volume is normal, up to 1.0 for complete silence."""
    if expected_volume < 1.0:
        return 0.0  # service doesn't normally emit much, silence is normal
    if observed_volume < expected_volume * 0.1:  # less than 10% of expected
        return min(1.0, (expected_volume - observed_volume) / expected_volume)
    return 0.0
```

### 4. Novel Pattern Detection
Appearance of fingerprints never seen before (a new error message).

Detected by: `new_fingerprint_rate` metric exceeding baseline. Novel errors during a failure are strong diagnostic signals.

---

## Cold Start Behavior

On first run (no baselines exist):

1. **Hours 0–6**: Learning mode. All metrics collected, no anomalies reported. Events still stored and visible, but anomaly scores are all 0.0.
2. **Hours 6–24**: Tentative baselines. Anomalies reported with `confidence="low"`. Only extreme deviations (z > 5) trigger anomaly flags.
3. **Hours 24–168**: Building weekly pattern. Confidence increases as more hourly buckets get populated.
4. **After 168 hours (1 week)**: Full operation. All hourly buckets have at least 1 sample. Confidence upgrades as sample counts grow.

The cold start period is a known limitation. The system is transparent about it:
- UI displays "Baseline: learning (X hours remaining)" during cold start.
- Hypothesis confidence is automatically reduced during cold start.

---

## Execution Schedule

```
Every 5 minutes (aligned to clock):
    1. For each service with events in the last 5 minutes:
       a. Compute current metric values from event store
       b. Compute anomaly scores against baselines
       c. Store anomaly scores for use by correlation engine and hypothesis scoring
    2. Update metric ringbuffers with current values

Every 1 hour (aligned to clock):
    1. For each service:
       a. Aggregate the last hour's metrics
       b. Update baseline using EMA
    2. Persist baselines to disk
```

---

## Configuration

```toml
[anomaly_detection]
enabled = true
bucket_interval_minutes = 5
baseline_update_interval_hours = 1
baseline_alpha = 0.1               # EMA smoothing factor
cold_start_hours = 6               # no anomaly reporting during this period
min_samples_for_confidence = 4     # per hourly bucket

# Thresholds
spike_z_threshold = 3.0            # z-score for spike detection
sustained_lookback_buckets = 6     # 30 minutes at 5-min resolution
absence_sensitivity = 0.1          # flag absence below 10% of expected

# Metric weights for composite score
[anomaly_detection.weights]
error_rate = 0.35
event_volume = 0.20
new_fingerprint_rate = 0.20
restart_count = 0.15
warn_rate = 0.10
```

---

## Failure Modes

| Failure | Impact | Mitigation |
|---|---|---|
| Baseline corrupted | All anomaly scores invalid | Rebuild from ringbuffer (60h of data); report degraded confidence |
| Metric series missing | No anomaly detection for that service | Log warning; service excluded from anomaly-based correlation |
| Permanent regime change (new normal) | False anomalies for ~10 update cycles | EMA naturally adapts; operator can manually reset baseline |
| Seasonal pattern too complex | Hourly buckets insufficient | Accept limitation; 168-bucket granularity is a known trade-off |
| Clock change (DST, NTP jump) | Bucket alignment disrupted | Use UTC internally; ignore system-local timezone |

---

## Assumptions

1. Service behavior has detectable patterns at hourly and weekly granularity. Services with truly random event patterns will have high stddev baselines, resulting in fewer anomaly detections.
2. Five-minute metric buckets provide sufficient resolution. Sub-second anomalies (e.g., a 2-second error burst) are captured by the correlation engine's per-event analysis, not by the anomaly detector.
3. EMA with alpha=0.1 provides the right balance between adaptability and stability. Too fast (high alpha) and the baseline chases anomalies. Too slow (low alpha) and genuine baseline shifts take weeks to register.
