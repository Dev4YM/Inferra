from __future__ import annotations

from datetime import UTC, datetime

from core.enums import EventType, Severity
from events.models import DataQuality, NormalizedEvent, SourceRef
from explanation.cache_key import explanation_cache_key_hashes, hypotheses_hash


def test_hypotheses_hash_sorts_deterministically() -> None:
    hyp_a = {"hypothesis_id": "b", "rank": 2, "total_score": 0.5}
    hyp_b = {"hypothesis_id": "a", "rank": 1, "total_score": 0.9}
    first = hypotheses_hash([hyp_a, hyp_b])
    second = hypotheses_hash([hyp_b, hyp_a])
    assert first == second


def test_events_hash_head_stable_order() -> None:
    ts = datetime(2026, 5, 4, 12, 0, tzinfo=UTC)
    src = SourceRef(source_type="app", source_id="s", raw_offset=None, collected_at=ts)
    dq = DataQuality(1.0, 1.0, 1.0, 1.0, 1.0)
    ev1 = NormalizedEvent(
        event_id="evt-z",
        timestamp=ts,
        timestamp_source="payload",
        service_id="api",
        host_id="h",
        severity=Severity.ERROR,
        event_type=EventType.LOG,
        message="m",
        structured_data={},
        tags=frozenset(),
        fingerprint="fp",
        quality=dq,
        source_ref=src,
    )
    ev2 = NormalizedEvent(
        event_id="evt-a",
        timestamp=ts.replace(second=1),
        timestamp_source="payload",
        service_id="api",
        host_id="h",
        severity=Severity.ERROR,
        event_type=EventType.LOG,
        message="m2",
        structured_data={},
        tags=frozenset(),
        fingerprint="fp2",
        quality=dq,
        source_ref=src,
    )
    h1, _ = explanation_cache_key_hashes([], [ev1, ev2])
    h2, _ = explanation_cache_key_hashes([], [ev2, ev1])
    assert h1 == h2
