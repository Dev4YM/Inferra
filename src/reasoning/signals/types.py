from __future__ import annotations

from dataclasses import dataclass

from config.models import AnomalyDetectionConfig, InferraConfig
from events.models import NormalizedEvent
from runtime.service_graph import ServiceGraph


@dataclass(frozen=True)
class Signal:
    """Deterministic detector output."""

    name: str
    confidence: float
    evidence_event_ids: tuple[str, ...]
    service_id: str | None = None


@dataclass(slots=True)
class SignalContext:
    events: tuple[NormalizedEvent, ...]
    service_graph: ServiceGraph
    anomaly_scorer: object
    service_scores: dict[str, float]
    expected_heartbeats: dict[str, list[str]]

    @classmethod
    def build(
        cls,
        events: list[NormalizedEvent],
        service_graph: ServiceGraph,
        *,
        anomaly_config: AnomalyDetectionConfig | None = None,
        inferra_config: InferraConfig | None = None,
    ) -> SignalContext:
        from analysis.anomaly import AnomalyScorer

        cfg = inferra_config or InferraConfig()
        scorer = AnomalyScorer(anomaly_config or cfg.anomaly_detection)
        ordered = tuple(sorted(events, key=lambda e: (e.timestamp, e.event_id)))
        scores = scorer.service_scores(list(ordered)) if ordered else {}
        return cls(
            events=ordered,
            service_graph=service_graph,
            anomaly_scorer=scorer,
            service_scores=scores,
            expected_heartbeats=dict(cfg.anomaly_detection.expected_heartbeats),
        )

    def by_id(self) -> dict[str, NormalizedEvent]:
        return {e.event_id: e for e in self.events}
