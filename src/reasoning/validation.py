from __future__ import annotations

from dataclasses import asdict, fields
from datetime import timedelta
from typing import Any

from config.models import InferraConfig
from core.enums import Severity
from core.models import ScoreBreakdown
from events.models import NormalizedEvent
from reasoning.contradiction import ContradictionHandler


_SCORE_KEYS = tuple(field.name for field in fields(ScoreBreakdown))


class HypothesisValidator:
    def __init__(self, config: InferraConfig) -> None:
        self._config = config
        self._handler = ContradictionHandler(config.contradiction_handling)

    def validate(self, hypothesis: dict[str, Any], events: list[NormalizedEvent]) -> dict[str, Any]:
        by_id = {event.event_id: event for event in events}
        val_cfg = self._config.hypothesis_validation
        records = self._handler.detect(hypothesis, events)
        penalty = self._handler.penalty_multiplier(records)
        supporting = [by_id[event_id] for event_id in hypothesis.get("supporting_events", []) if event_id in by_id]
        contradicting_ids = sorted({item.event_id for item in records if item.event_id})
        contradiction_ratio = len(contradicting_ids) / max(1, len(supporting) + len(contradicting_ids))

        invalidation_reasons = list(hypothesis.get("invalidation_reasons", []))
        invalidation_reasons.extend(item.explanation for item in records)

        validation_statuses: list[str] = []
        if not val_cfg.enabled:
            warnings = 0
            temporal_ok = True
            root_severity_ok = True
            ratio_ok = True
        else:
            temporal_ok, temporal_msg = self._temporal_consistency(hypothesis, supporting, by_id)
            if temporal_msg:
                invalidation_reasons.append(temporal_msg)
                validation_statuses.append("WARN" if temporal_ok else "FAIL")

            root_severity_ok, root_msg = self._root_cause_severity(hypothesis, by_id)
            if root_msg:
                invalidation_reasons.append(root_msg)
                validation_statuses.append("FAIL" if not root_severity_ok else "WARN")

            ratio_ok, ratio_msg = self._contradiction_ratio(contradiction_ratio)
            if ratio_msg:
                invalidation_reasons.append(ratio_msg)
                validation_statuses.append("FAIL" if not ratio_ok else "WARN")

            warnings = sum(1 for item in validation_statuses if item == "WARN")

        base_score = float(hypothesis.get("total_score") or 0.0)
        base_score = round(base_score * penalty, 4)
        warn_mult = max(0.0, 1.0 - warnings * float(val_cfg.confidence_reduction_per_warning))
        score = round(base_score * warn_mult, 4) if val_cfg.enabled else base_score

        is_valid = bool(supporting) and temporal_ok and root_severity_ok and ratio_ok
        if not supporting:
            invalidation_reasons.append("No supporting evidence exists for this hypothesis.")

        updated = dict(hypothesis)
        updated["total_score"] = score
        updated["contradicting_events"] = contradicting_ids
        updated["is_valid"] = is_valid
        updated["invalidation_reasons"] = invalidation_reasons
        updated["score_breakdown"] = _six_component_breakdown(hypothesis.get("score_breakdown"))

        if not is_valid:
            updated["total_score"] = 0.0
        return updated

    def _temporal_consistency(
        self,
        hypothesis: dict[str, Any],
        supporting: list[NormalizedEvent],
        by_id: dict[str, NormalizedEvent],
    ) -> tuple[bool, str]:
        root_id = hypothesis.get("root_cause_event_id")
        if not root_id or root_id not in by_id:
            return True, ""
        root = by_id[root_id]
        tol = timedelta(seconds=float(self._config.contradiction_handling.timeline_tolerance_seconds))
        effects = [event for event in supporting if event.event_id != root_id]
        if not effects:
            return True, ""
        before = sum(1 for event in effects if event.timestamp + tol < root.timestamp)
        ratio = before / len(effects)
        thr = float(self._config.hypothesis_validation.temporal_consistency_threshold)
        warn_thr = float(self._config.hypothesis_validation.temporal_consistency_warn)
        if ratio > thr:
            return False, f"Temporal consistency failed: {before}/{len(effects)} effects precede the root beyond tolerance."
        if ratio > warn_thr:
            return True, f"Temporal consistency warning: {before}/{len(effects)} effects precede the root (possible clock skew)."
        return True, ""

    def _root_cause_severity(self, hypothesis: dict[str, Any], by_id: dict[str, NormalizedEvent]) -> tuple[bool, str]:
        root_id = hypothesis.get("root_cause_event_id")
        if not root_id or root_id not in by_id:
            return True, ""
        root = by_id[root_id]
        minimum = _severity_from_name(self._config.hypothesis_validation.min_root_cause_severity)
        if root.severity < minimum:
            return False, f"Root cause severity {root.severity.name} is below required {minimum.name}."
        return True, ""

    def _contradiction_ratio(self, ratio: float) -> tuple[bool, str]:
        fail_at = float(self._config.hypothesis_validation.contradiction_ratio_fail)
        warn_at = float(self._config.hypothesis_validation.contradiction_ratio_warn)
        if ratio > fail_at:
            return False, f"Contradiction ratio {ratio:.0%} exceeds failure threshold {fail_at:.0%}."
        if ratio > warn_at:
            return True, f"Contradiction ratio {ratio:.0%} exceeds warning threshold {warn_at:.0%}."
        return True, ""


def _six_component_breakdown(score_breakdown: Any) -> dict[str, float]:
    if isinstance(score_breakdown, ScoreBreakdown):
        raw = asdict(score_breakdown)
    elif isinstance(score_breakdown, dict):
        raw = score_breakdown
    else:
        raw = {}
    return {key: round(float(raw.get(key, 0.0)), 4) for key in _SCORE_KEYS}


def _severity_from_name(name: str) -> Severity:
    upper = name.strip().upper()
    for item in Severity:
        if item.name == upper:
            return item
    return Severity.WARN
