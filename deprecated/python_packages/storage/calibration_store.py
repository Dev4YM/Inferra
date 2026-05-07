from __future__ import annotations

import json
from dataclasses import asdict
from pathlib import Path

from config.models import CalibrationDefaultsConfig
from core.models import CalibrationBucket, CalibrationModel, IncidentFeedback, ScoredHypothesis
from core.time import parse_datetime, to_iso, utc_now

MIN_SAMPLES = 10
BUCKET_RANGES = ((0.0, 0.2), (0.2, 0.4), (0.4, 0.6), (0.6, 0.8), (0.8, 1.0))


class CalibrationStore:
    def __init__(self, path: str | Path = "./data/calibration.json") -> None:
        self.path = Path(path)

    def load(self) -> CalibrationModel:
        if not self.path.exists():
            return CalibrationModel()
        data = json.loads(self.path.read_text(encoding="utf-8"))
        return CalibrationModel(
            schema_version=int(data.get("schema_version", 1)),
            buckets=[
                CalibrationBucket(
                    score_lower=float(bucket["score_lower"]),
                    score_upper=float(bucket["score_upper"]),
                    total_predictions=int(bucket.get("total_predictions", bucket.get("total", 0))),
                    correct_predictions=int(bucket.get("correct_predictions", bucket.get("correct", 0))),
                    accuracy=float(bucket.get("accuracy", 0.0)),
                    sample_confidence=bucket.get("sample_confidence", "insufficient"),
                )
                for bucket in data.get("buckets", [])
            ]
            or CalibrationModel().buckets,
            last_updated=parse_datetime(data["last_updated"]) if data.get("last_updated") else None,
            total_feedback_count=int(data.get("total_feedback_count", 0)),
            overall_accuracy=float(data.get("overall_accuracy", 0.0)),
        )

    def save(self, model: CalibrationModel) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "schema_version": model.schema_version,
            "last_updated": to_iso(model.last_updated) if model.last_updated else None,
            "total_feedback_count": model.total_feedback_count,
            "overall_accuracy": model.overall_accuracy,
            "buckets": [asdict(bucket) for bucket in model.buckets],
        }
        temp_path = self.path.with_suffix(self.path.suffix + ".tmp")
        temp_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
        temp_path.replace(self.path)


def update_calibration(
    model: CalibrationModel,
    feedback: IncidentFeedback,
    hypotheses: list[ScoredHypothesis],
    *,
    min_samples: int = MIN_SAMPLES,
) -> None:
    if feedback.feedback_type == "skipped":
        return
    if feedback.feedback_type == "none_correct":
        return

    model.total_feedback_count += 1
    for hypothesis in hypotheses:
        bucket = find_bucket(model.buckets, hypothesis.total_score)
        bucket.total_predictions += 1
        if feedback.feedback_type == "confirmed" and hypothesis.hypothesis_id == feedback.correct_hypothesis_id:
            bucket.correct_predictions += 1
        bucket.accuracy = bucket.correct_predictions / max(bucket.total_predictions, 1)
        bucket.sample_confidence = "sufficient" if bucket.total_predictions >= min_samples else "insufficient"

    total_correct = sum(bucket.correct_predictions for bucket in model.buckets)
    total_predictions = sum(bucket.total_predictions for bucket in model.buckets)
    model.overall_accuracy = total_correct / max(total_predictions, 1)
    model.last_updated = utc_now()


def assign_confidence_label(
    score: float,
    calibration: CalibrationModel,
    defaults: CalibrationDefaultsConfig | None = None,
    *,
    min_samples: int = MIN_SAMPLES,
    staleness_days: int = 30,
    min_feedback_for_staleness: int = 20,
) -> str:
    defaults = defaults or CalibrationDefaultsConfig()
    stale_state = check_calibration_staleness(
        calibration,
        staleness_days=staleness_days,
        min_feedback=min_feedback_for_staleness,
    )
    if stale_state in ("stale", "insufficient_data"):
        return label_from_score_thresholds(score, defaults)
    bucket = find_bucket(calibration.buckets, score)
    if bucket.total_predictions < min_samples:
        return label_from_score_thresholds(score, defaults)
    if bucket.accuracy >= 0.7:
        return "high"
    if bucket.accuracy >= 0.4:
        return "medium"
    return "low"


def label_from_score_thresholds(score: float, defaults: CalibrationDefaultsConfig) -> str:
    if score >= float(defaults.high_threshold):
        return "high"
    if score >= float(defaults.medium_threshold):
        return "medium"
    return "low"


def check_calibration_staleness(
    model: CalibrationModel,
    *,
    staleness_days: int = 30,
    min_feedback: int = 20,
) -> str:
    if model.total_feedback_count < min_feedback:
        return "insufficient_data"
    if model.last_updated and (utc_now() - model.last_updated).days > staleness_days:
        return "stale"
    return "current"


def find_bucket(buckets: list[CalibrationBucket], score: float) -> CalibrationBucket:
    bounded_score = max(0.0, min(0.999999, score))
    for bucket in buckets:
        if bucket.score_lower <= bounded_score < bucket.score_upper:
            return bucket
    return buckets[-1]
