from __future__ import annotations

import asyncio
import json

from fastapi.testclient import TestClient

from inferra_legacy.app import InferraRuntime
from config.model import DeduplicationConfig, InferraConfig, StorageConfig
from core.enums import Severity
import pytest

pytestmark = pytest.mark.legacy_runtime


def test_runtime_dedup_collapses_duplicates_and_emits_summaries(tmp_path):
    async def run():
        config = InferraConfig(
            storage=StorageConfig(data_dir=tmp_path),
            deduplication=DeduplicationConfig(
                window_seconds=120,
                periodic_summary_interval_seconds=30,
            ),
        )
        runtime = InferraRuntime(config)
        await runtime.start()
        try:
            for i in range(50):
                await runtime.ingest_payload(
                    json.dumps({"service": "api", "level": "info", "message": "health check passed"}),
                    source_type="app",
                    source_id="app://test",
                )
            stats = runtime.dedup.stats()
            assert stats.total_suppressed >= 49
            events = runtime.event_store.latest_events(limit=1000)
            non_summary = [e for e in events if "dedup_summary" not in e.tags]
            assert len(non_summary) == 1
        finally:
            await runtime.stop()

    asyncio.run(run())


def test_runtime_severity_escalation_splits_dedup_stream(tmp_path):
    async def run():
        config = InferraConfig(
            storage=StorageConfig(data_dir=tmp_path),
            deduplication=DeduplicationConfig(
                window_seconds=120,
                severity_escalation_splits=True,
            ),
        )
        runtime = InferraRuntime(config)
        await runtime.start()
        try:
            for _ in range(10):
                await runtime.ingest_payload(
                    json.dumps({"service": "api", "level": "info", "message": "connection pool saturated"}),
                )
            await runtime.ingest_payload(
                json.dumps({"service": "api", "level": "error", "message": "connection pool saturated"}),
            )
            events = runtime.event_store.latest_events(limit=1000)
            non_summary = [e for e in events if "dedup_summary" not in e.tags]
            stored_severities = {e.severity for e in non_summary}
            assert Severity.INFO in stored_severities
            assert Severity.ERROR in stored_severities
        finally:
            await runtime.stop()

    asyncio.run(run())


def test_dashboard_shows_dedup_and_noise_stats(tmp_path):
    from web import create_app

    config = InferraConfig(
        storage=StorageConfig(data_dir=tmp_path),
        deduplication=DeduplicationConfig(window_seconds=120),
    )
    app = create_app(config)

    with TestClient(app) as client:
        for _ in range(5):
            client.post("/api/ingest", json={"service": "api", "level": "info", "message": "heartbeat ok"})
        client.post("/api/ingest", json={"service": "api", "level": "error", "message": "timeout calling db"})

        dashboard = client.get("/api/dashboard").json()
        assert dashboard["dedup"]["total_suppressed"] >= 0
        assert isinstance(dashboard["dedup"]["tracked_fingerprints"], int)
        assert isinstance(dashboard["noise"]["blocklist_hits"], int)
        assert isinstance(dashboard["noise"]["routine_fingerprints"], int)
        assert isinstance(dashboard["noise"]["total_filtered"], int)
