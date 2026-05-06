from __future__ import annotations

import asyncio
import sqlite3
from dataclasses import replace
from datetime import UTC, datetime

import pytest

from app import InferraRuntime
from config.models import InferraConfig
from core.enums import EventType, Severity
from events.models import DataQuality, NormalizedEvent, SourceRef


def _sample_event(event_id: str, ts: datetime) -> NormalizedEvent:
    return NormalizedEvent(
        event_id=event_id,
        timestamp=ts,
        timestamp_source="parsed",
        service_id="api",
        host_id="h1",
        severity=Severity.ERROR,
        event_type=EventType.LOG,
        message="disk chaos",
        structured_data={},
        tags=frozenset(),
        fingerprint=f"fp-{event_id}",
        quality=DataQuality(1.0, 1.0, 1.0, 1.0, 1.0),
        source_ref=SourceRef(
            source_type="app",
            source_id="app://c",
            raw_offset=None,
            collected_at=ts,
        ),
    )


@pytest.mark.chaos
@pytest.mark.asyncio
async def test_disk_full_marks_degraded_and_stops_collectors(tmp_path, monkeypatch: pytest.MonkeyPatch) -> None:
    cfg = replace(
        InferraConfig(),
        storage=replace(InferraConfig().storage, data_dir=tmp_path),
        collectors=replace(InferraConfig().collectors, auto_start=False),
    )
    rt = InferraRuntime(cfg)
    ev = _sample_event("evt-disk", datetime.now(UTC))

    async def run() -> None:
        await rt.start(start_collectors=True)

        def _boom(_events: list[NormalizedEvent]) -> int:
            raise sqlite3.OperationalError("database or disk is full")

        monkeypatch.setattr(rt.event_store, "insert_batch", _boom)
        assert rt._store_event(ev) is False
        await asyncio.sleep(0.35)
        snap = rt.degradation_snapshot()
        assert snap["degraded"] is True
        assert "disk_full" in snap["degraded_reasons"]
        assert snap["storage_writes_ok"] is False
        rows = rt.collector_supervisor.health()
        assert rows and all(str(r.get("status")) == "stopped" for r in rows)
        await rt.stop()

    await run()
