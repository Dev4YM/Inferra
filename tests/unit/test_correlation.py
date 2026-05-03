from core.time import utc_now
from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline
from analysis.correlation import CorrelationEngine


def test_correlation_clusters_shared_failure_tags():
    pipeline = NormalizationPipeline()
    now = utc_now()
    first = pipeline.normalize(
        RawEvent(
            source_type="app",
            source_id="test",
            raw_payload='{"service":"api","level":"error","message":"timeout calling postgres"}',
            collected_at=now,
            metadata={},
        )
    )
    second = pipeline.normalize(
        RawEvent(
            source_type="app",
            source_id="test",
            raw_payload='{"service":"worker","level":"error","message":"timeout calling postgres"}',
            collected_at=now,
            metadata={},
        )
    )

    clusters = CorrelationEngine().build_clusters([first, second])

    assert len(clusters) == 1
    assert clusters[0].affected_services == {"api", "worker"}
    assert clusters[0].correlation_edges[0].edge_type == "co_occurrence"
