from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class ConfidenceInput:
    score: float
    supporting_count: int
    contradiction_count: int
    dependency_proximity: float


class ConfidenceCalibrator:
    def label(self, item: ConfidenceInput) -> str:
        score = self.adjusted_score(item)
        if score >= 0.85:
            return "high"
        if score >= 0.68:
            return "medium"
        return "low"

    def adjusted_score(self, item: ConfidenceInput) -> float:
        score = item.score
        if item.supporting_count < 2:
            score -= 0.08
        elif item.supporting_count >= 4:
            score += 0.04
        if item.dependency_proximity >= 0.9:
            score += 0.03
        elif item.dependency_proximity <= 0.3:
            score -= 0.04
        score -= min(0.2, item.contradiction_count * 0.06)
        return round(max(0.0, min(1.0, score)), 4)
