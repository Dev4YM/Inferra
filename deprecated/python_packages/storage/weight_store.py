from __future__ import annotations

import json
from dataclasses import asdict
from pathlib import Path

from config.models import ScoringTuningConfig
from core.models import IncidentFeedback, ScoredHypothesis, WeightSnapshot, WeightState
from core.time import parse_datetime, to_iso, utc_now


DEFAULT_WEIGHTS = {
    "temporal_alignment": 0.25,
    "correlation_strength": 0.20,
    "frequency_weight": 0.15,
    "dependency_proximity": 0.15,
    "evidence_coverage": 0.15,
    "anomaly_severity": 0.10,
}
LEARNING_RATE = 0.05
MAX_DRIFT = 0.5
MIN_WEIGHT = 0.03


class WeightStore:
    def __init__(
        self,
        path: str | Path = "./data/scoring_weights.json",
        history_path: str | Path = "./data/weight_history.jsonl",
    ) -> None:
        self.path = Path(path)
        self.history_path = Path(history_path)

    def load(self) -> WeightState:
        if not self.path.exists():
            return WeightState(
                weights=dict(DEFAULT_WEIGHTS),
                default_weights=dict(DEFAULT_WEIGHTS),
                history=self._load_history(),
            )
        data = json.loads(self.path.read_text(encoding="utf-8"))
        return WeightState(
            weights=dict(data.get("weights") or DEFAULT_WEIGHTS),
            default_weights=dict(data.get("default_weights") or DEFAULT_WEIGHTS),
            update_count=int(data.get("update_count", 0)),
            history=self._load_history(),
        )

    def save(self, state: WeightState) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        payload = {
            "schema_version": 1,
            "weights": state.weights,
            "default_weights": state.default_weights,
            "update_count": state.update_count,
            "last_updated": to_iso(state.history[-1].timestamp) if state.history else None,
        }
        temp_path = self.path.with_suffix(self.path.suffix + ".tmp")
        temp_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
        temp_path.replace(self.path)
        self._append_missing_history(state.history)

    def append_history(self, snapshot: WeightSnapshot) -> None:
        self.history_path.parent.mkdir(parents=True, exist_ok=True)
        line = self._snapshot_line(snapshot)
        with self.history_path.open("a", encoding="utf-8") as handle:
            handle.write(line + "\n")

    def _load_history(self) -> list[WeightSnapshot]:
        if not self.history_path.exists():
            return []
        history: list[WeightSnapshot] = []
        for line in self.history_path.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            item = json.loads(line)
            history.append(
                WeightSnapshot(
                    timestamp=parse_datetime(item["timestamp"]) or utc_now(),
                    weights_before=dict(item["weights_before"]),
                    weights_after=dict(item["weights_after"]),
                    trigger_incident_id=item["trigger_incident_id"],
                    reason=item["reason"],
                )
            )
        return history

    def _append_missing_history(self, snapshots: list[WeightSnapshot]) -> None:
        if not snapshots:
            return
        existing = set()
        if self.history_path.exists():
            existing = {line for line in self.history_path.read_text(encoding="utf-8").splitlines() if line.strip()}
        missing = [self._snapshot_line(snapshot) for snapshot in snapshots]
        new_lines = [line for line in missing if line not in existing]
        if not new_lines:
            return
        self.history_path.parent.mkdir(parents=True, exist_ok=True)
        with self.history_path.open("a", encoding="utf-8") as handle:
            for line in new_lines:
                handle.write(line + "\n")

    def _snapshot_line(self, snapshot: WeightSnapshot) -> str:
        return json.dumps(
            {
                "timestamp": to_iso(snapshot.timestamp),
                "weights_before": snapshot.weights_before,
                "weights_after": snapshot.weights_after,
                "trigger_incident_id": snapshot.trigger_incident_id,
                "reason": snapshot.reason,
            },
            sort_keys=True,
        )


def update_weights(
    state: WeightState,
    feedback: IncidentFeedback,
    hypotheses: list[ScoredHypothesis],
    *,
    tuning: ScoringTuningConfig | None = None,
) -> None:
    if tuning is not None and not tuning.enabled:
        return
    if feedback.feedback_type == "skipped":
        return
    if feedback.correct_hypothesis_id is None and feedback.feedback_type == "none_correct":
        return

    correct = next((item for item in hypotheses if item.hypothesis_id == feedback.correct_hypothesis_id), None)
    if correct is None:
        return

    lr = float(tuning.learning_rate) if tuning is not None else LEARNING_RATE
    max_drift = float(tuning.max_drift_from_default) if tuning is not None else MAX_DRIFT
    min_w = float(tuning.min_weight) if tuning is not None else MIN_WEIGHT

    snapshot_before = dict(state.weights)
    if correct.rank == 1:
        _reward_discriminating_components(state, correct, hypotheses, lr)
    else:
        top_wrong = hypotheses[0]
        _penalize_misleading_components(state, correct, top_wrong, lr)

    for _ in range(32):
        _enforce_bounds(state, max_drift=max_drift, min_weight=min_w)
        total = sum(state.weights.values())
        if total <= 0:
            break
        for key in state.weights:
            state.weights[key] /= total
        if all(
            max(min_w, state.default_weights[k] * (1.0 - max_drift)) - 1e-12
            <= state.weights[k]
            <= state.default_weights[k] * (1.0 + max_drift) + 1e-12
            for k in state.weights
        ):
            break

    state.update_count += 1
    state.history.append(
        WeightSnapshot(
            timestamp=utc_now(),
            weights_before=snapshot_before,
            weights_after=dict(state.weights),
            trigger_incident_id=feedback.incident_id,
            reason=f"rank_1={'correct' if correct.rank == 1 else 'incorrect'}",
        )
    )


def _reward_discriminating_components(
    state: WeightState,
    correct: ScoredHypothesis,
    hypotheses: list[ScoredHypothesis],
    learning_rate: float,
) -> None:
    averages = _average_breakdown(hypotheses)
    for component in state.weights:
        if getattr(correct.score_breakdown, component) > averages[component]:
            state.weights[component] *= 1.0 + learning_rate * 0.5


def _penalize_misleading_components(
    state: WeightState,
    correct: ScoredHypothesis,
    top_wrong: ScoredHypothesis,
    learning_rate: float,
) -> None:
    for component in state.weights:
        correct_value = getattr(correct.score_breakdown, component)
        wrong_value = getattr(top_wrong.score_breakdown, component)
        if wrong_value > correct_value:
            state.weights[component] *= 1.0 - learning_rate
        elif correct_value > wrong_value:
            state.weights[component] *= 1.0 + learning_rate


def _enforce_bounds(state: WeightState, *, max_drift: float = MAX_DRIFT, min_weight: float = MIN_WEIGHT) -> None:
    for component in state.weights:
        default = state.default_weights[component]
        lower = max(min_weight, default * (1.0 - max_drift))
        upper = default * (1.0 + max_drift)
        state.weights[component] = max(lower, min(upper, state.weights[component]))


def reset_weights(state: WeightState) -> None:
    weights_before = dict(state.weights)
    state.weights = dict(state.default_weights)
    state.history.append(
        WeightSnapshot(
            timestamp=utc_now(),
            weights_before=weights_before,
            weights_after=dict(state.default_weights),
            trigger_incident_id="manual_reset",
            reason="operator reset to defaults",
        )
    )


def _average_breakdown(hypotheses: list[ScoredHypothesis]) -> dict[str, float]:
    if not hypotheses:
        return {key: 0.0 for key in DEFAULT_WEIGHTS}
    totals = {key: 0.0 for key in DEFAULT_WEIGHTS}
    for hypothesis in hypotheses:
        breakdown = asdict(hypothesis.score_breakdown)
        for key in totals:
            totals[key] += float(breakdown[key])
    return {key: totals[key] / len(hypotheses) for key in totals}
