# Confidence Calibration

## Purpose

Scores from the scoring engine are relative rankings, not probability estimates. A score of 0.8 does not mean "80% likely to be correct." Confidence calibration bridges this gap by tracking how often highly-scored hypotheses turned out to be correct (when the operator provides feedback), and using that history to produce meaningful confidence labels.

---

## Problem Statement

Without calibration, the system cannot answer: "When I say 'high confidence,' how often am I right?"

A well-calibrated system means:
- Hypotheses labeled "high confidence" are correct ~80% of the time
- Hypotheses labeled "medium confidence" are correct ~50% of the time
- Hypotheses labeled "low confidence" are correct ~20% of the time

Calibration requires feedback data. Until enough feedback is collected, the system uses conservative defaults.

---

## Feedback Mechanism

When an operator resolves an incident, they can optionally indicate which hypothesis (if any) was correct:

```python
@dataclass
class IncidentFeedback:
    incident_id: str
    resolved_at: datetime
    correct_hypothesis_id: str | None     # None = "none were correct" or "I don't know"
    feedback_type: str                    # "confirmed" | "none_correct" | "skipped"
    operator_notes: str = ""              # free-text
```

This feedback is stored in the incident store and used to update calibration data.

---

## Calibration Model

### Score-to-Accuracy Mapping

The calibration model divides the score range [0.0, 1.0] into buckets and tracks the accuracy rate per bucket:

```python
@dataclass
class CalibrationBucket:
    score_lower: float          # inclusive
    score_upper: float          # exclusive
    total_predictions: int      # hypotheses that fell in this score range
    correct_predictions: int    # of those, how many were confirmed correct
    accuracy: float             # correct / total (0.0 if total == 0)
    sample_confidence: str      # "sufficient" | "insufficient"

@dataclass
class CalibrationModel:
    schema_version: int = 1
    buckets: list[CalibrationBucket] = field(default_factory=lambda: [
        CalibrationBucket(0.0, 0.2, 0, 0, 0.0, "insufficient"),
        CalibrationBucket(0.2, 0.4, 0, 0, 0.0, "insufficient"),
        CalibrationBucket(0.4, 0.6, 0, 0, 0.0, "insufficient"),
        CalibrationBucket(0.6, 0.8, 0, 0, 0.0, "insufficient"),
        CalibrationBucket(0.8, 1.0, 0, 0, 0.0, "insufficient"),
    ])
    last_updated: datetime | None = None
    total_feedback_count: int = 0
    overall_accuracy: float = 0.0   # across all buckets
```

### Update Logic

```python
def update_calibration(model: CalibrationModel, feedback: IncidentFeedback,
                        hypotheses: list[ScoredHypothesis]) -> None:
    if feedback.feedback_type == "skipped":
        return  # no data to learn from

    model.total_feedback_count += 1

    for h in hypotheses:
        bucket = find_bucket(model.buckets, h.total_score)
        bucket.total_predictions += 1

        if feedback.feedback_type == "confirmed" and h.hypothesis_id == feedback.correct_hypothesis_id:
            bucket.correct_predictions += 1
        # If feedback is "none_correct", no bucket gets a correct increment

        if bucket.total_predictions >= MIN_SAMPLES:
            bucket.accuracy = bucket.correct_predictions / bucket.total_predictions
            bucket.sample_confidence = "sufficient"

    # Recalculate overall accuracy
    total_correct = sum(b.correct_predictions for b in model.buckets)
    total_pred = sum(b.total_predictions for b in model.buckets)
    model.overall_accuracy = total_correct / max(total_pred, 1)
    model.last_updated = datetime.utcnow()
```

### Minimum Samples

A bucket needs at least `MIN_SAMPLES = 10` feedback entries before its accuracy is considered reliable. Below this threshold, the bucket uses a default accuracy estimate.

---

## Confidence Label Assignment

```python
def assign_confidence_label(score: float, calibration: CalibrationModel) -> str:
    """Map a hypothesis score to a confidence label using calibration data."""
    bucket = find_bucket(calibration.buckets, score)

    if bucket.sample_confidence == "insufficient":
        # Not enough feedback data: use conservative defaults
        return default_confidence_label(score)

    accuracy = bucket.accuracy

    if accuracy >= 0.7:
        return "high"
    elif accuracy >= 0.4:
        return "medium"
    else:
        return "low"

def default_confidence_label(score: float) -> str:
    """Conservative defaults before calibration data is available."""
    # Intentionally conservative: even high scores get "medium" without calibration
    if score >= 0.75:
        return "medium"
    elif score >= 0.4:
        return "low"
    else:
        return "low"
```

**Design choice**: Before calibration, even the highest-scoring hypotheses are labeled "medium" rather than "high." This prevents the system from expressing unwarranted confidence. Earning the "high" label requires demonstrated accuracy through feedback.

---

## Calibration Persistence

Stored in `./data/calibration.json`:

```json
{
    "schema_version": 1,
    "last_updated": "2026-05-01T14:00:00Z",
    "total_feedback_count": 47,
    "overall_accuracy": 0.62,
    "buckets": [
        {"score_lower": 0.0, "score_upper": 0.2, "total": 12, "correct": 1, "accuracy": 0.08},
        {"score_lower": 0.2, "score_upper": 0.4, "total": 18, "correct": 5, "accuracy": 0.28},
        {"score_lower": 0.4, "score_upper": 0.6, "total": 25, "correct": 12, "accuracy": 0.48},
        {"score_lower": 0.6, "score_upper": 0.8, "total": 30, "correct": 21, "accuracy": 0.70},
        {"score_lower": 0.8, "score_upper": 1.0, "total": 15, "correct": 13, "accuracy": 0.87}
    ]
}
```

Loaded at startup. Updated on every feedback submission. Persisted after update.

---

## Calibration Drift Detection

Over time, the system's scoring characteristics may change (new rule templates, different workloads). The calibration model includes a staleness check:

```python
def check_calibration_staleness(model: CalibrationModel) -> str:
    """Detect if calibration data is stale or unreliable."""
    if model.total_feedback_count < 20:
        return "insufficient_data"

    if model.last_updated and (datetime.utcnow() - model.last_updated).days > 30:
        return "stale"

    # Check if recent feedback contradicts calibration
    # (This would require tracking recent vs. historical accuracy,
    # which we do via a separate rolling window)
    return "current"
```

If calibration is stale:
- The system continues using the stored model but marks labels with "(calibration outdated)"
- A notification in the UI suggests the operator provide more feedback

---

## UI Integration

The confidence label affects how hypotheses are presented:

| Label | UI Treatment |
|---|---|
| "high" | Green confidence badge, shown first, expanded by default |
| "medium" | Yellow confidence badge, normal display |
| "low" | Gray confidence badge, collapsed by default |
| "low (uncalibrated)" | Gray badge with info tooltip: "Not enough feedback data to calibrate confidence" |

The calibration accuracy chart is available in the UI under Settings > System Health:
- Shows accuracy by score bucket
- Shows total feedback count
- Indicates whether calibration has sufficient data

---

## Bootstrapping Strategy

Since calibration requires feedback, and feedback requires incidents, there's a chicken-and-egg problem at first use.

**Approach**:
1. First 20 incidents: all hypotheses labeled "low" or "medium" (conservative defaults)
2. The UI prominently nudges the operator: "Help improve Inferra's accuracy — tell us which hypothesis was correct"
3. After 20 feedback entries: initial calibration applies, labels begin reflecting actual accuracy
4. Continuous refinement: every additional feedback entry improves calibration

The system never claims high confidence until it has evidence of being accurate.

---

## Configuration

```toml
[calibration]
enabled = true
min_samples_per_bucket = 10
bucket_count = 5                    # divides [0, 1] into 5 equal ranges
staleness_threshold_days = 30
persistence_file = "./data/calibration.json"

# Default labels (used before calibration)
[calibration.defaults]
high_threshold = 0.75        # score >= this → "medium" (not "high" — conservative)
medium_threshold = 0.40      # score >= this → "low"
```

---

## Limitations

1. **Feedback is optional**: Operators may never provide feedback, in which case calibration never improves beyond defaults. The system must be useful without calibration.
2. **Selection bias**: Operators may only provide feedback for easy incidents (where the correct hypothesis is obvious), skewing calibration toward incidents where the system already performs well.
3. **Small sample sizes**: Even with regular feedback, individual buckets may have <10 samples for months. The system handles this gracefully with conservative defaults.
4. **Feedback accuracy**: The operator's identification of the "correct" hypothesis may itself be wrong. The calibration model assumes operator feedback is ground truth.
