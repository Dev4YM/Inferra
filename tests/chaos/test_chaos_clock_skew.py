from __future__ import annotations

import json
from datetime import UTC, datetime

import pytest

from events.models import RawEvent
from normalization.pipeline import NormalizationPipeline

COLLECTED_AT = datetime(2026, 5, 4, 12, 0, tzinfo=UTC)


@pytest.mark.chaos
def test_extreme_future_timestamp_surfaces_clock_skew_flag() -> None:
    raw = RawEvent(
        source_type="file",
        source_id="file://skew.log",
        raw_payload=json.dumps(
            {
                "timestamp": "2026-06-10T10:00:00Z",
                "service": "api",
                "level": "error",
                "message": "skew",
            }
        ),
        collected_at=COLLECTED_AT,
        metadata={"path": "/var/log/skew.log"},
    )
    event = NormalizationPipeline().normalize(raw)
    assert "clock_skew_future" in event.quality.flags
