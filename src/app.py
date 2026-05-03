from __future__ import annotations

import asyncio
from pathlib import Path

from analysis import SimpleIncidentAnalyzer
from collectors import CollectorSupervisor, build_collectors
from config import InferraConfig, load_config
from core.enums import Severity
from core.time import utc_now
from events.models import RawEvent
from normalization import NormalizationPipeline
from normalization.dedup import DedupDecision, DedupTracker
from normalization.noise import NoiseFilter
from runtime import ServiceGraph
from storage import SQLiteEventStore, SQLiteIncidentStore, initialize_storage


class InferraRuntime:
    def __init__(self, config: InferraConfig | None = None) -> None:
        self.config = config or load_config()
        self.event_store: SQLiteEventStore
        self.incident_store: SQLiteIncidentStore
        self.event_store, self.incident_store = initialize_storage(self.config.storage.data_dir)
        self.service_graph = ServiceGraph.load(self.config.storage.data_dir / "service_graph.json")
        self.pipeline = NormalizationPipeline(self.config.normalization)
        self.dedup = DedupTracker(
            window_seconds=self.config.deduplication.window_seconds,
            max_tracked=self.config.deduplication.max_tracked_fingerprints,
        )
        self.noise_filter = NoiseFilter()
        self.analyzer = SimpleIncidentAnalyzer(self.event_store, self.incident_store, self.service_graph)
        self.raw_queue: asyncio.Queue[RawEvent] = asyncio.Queue(maxsize=10000)
        self.collector_supervisor = CollectorSupervisor(
            build_collectors(self.config, state_store=self.event_store),
            self.raw_queue,
            retry_initial_seconds=self.config.collectors.retry_initial_seconds,
            retry_max_seconds=self.config.collectors.retry_max_seconds,
        )
        self._consumer_task: asyncio.Task[None] | None = None
        self._running = False

    async def start(self, start_collectors: bool | None = None) -> None:
        self._running = True
        self._consumer_task = asyncio.create_task(self._consume_raw_events())
        should_start_collectors = self.config.collectors.auto_start if start_collectors is None else start_collectors
        if should_start_collectors:
            await self.collector_supervisor.start()

    async def stop(self) -> None:
        self._running = False
        await self.collector_supervisor.stop()
        await self.raw_queue.join()
        if self._consumer_task:
            self._consumer_task.cancel()
            try:
                await self._consumer_task
            except asyncio.CancelledError:
                pass
        self.event_store.close()
        self.incident_store.close()

    async def ingest_raw(self, raw: RawEvent) -> str | None:
        event = self.pipeline.normalize(raw)
        if self.config.deduplication.enabled and self.dedup.check(event) == DedupDecision.SUPPRESS:
            return None
        event = self.noise_filter.annotate(event)
        if self.config.noise_filter.enabled and not self.noise_filter.should_store(event):
            return None
        self.event_store.add_event(event)
        if event.severity >= Severity.WARN:
            self.analyzer.analyze_recent()
        return event.event_id

    async def ingest_payload(
        self,
        payload: str,
        source_type: str = "app",
        source_id: str = "app://localhost",
        metadata: dict | None = None,
    ) -> str | None:
        raw = RawEvent(
            source_type=source_type,
            source_id=source_id,
            raw_payload=payload,
            collected_at=utc_now(),
            metadata=metadata or {},
        )
        return await self.ingest_raw(raw)

    async def ingest_file_once(self, path: str | Path, service_id: str | None = None) -> int:
        from collectors.file import FileCollector

        collector = FileCollector(path, service_id=service_id)
        count_before = len(self.event_store.latest_events(limit=100000))
        await collector.collect_existing(self.raw_queue)
        await self.raw_queue.join()
        count_after = len(self.event_store.latest_events(limit=100000))
        self.analyzer.analyze_recent(window_seconds=3600)
        return max(0, count_after - count_before)

    def add_topology_relation(self, source: str, target: str, relation_type: str = "depends_on") -> None:
        self.service_graph.add_relation(source, target, relation_type)
        self.service_graph.save(self.config.storage.data_dir / "service_graph.json")

    def collector_health(self) -> list[dict]:
        return self.collector_supervisor.health()

    async def start_collectors(self) -> None:
        await self.collector_supervisor.start()

    async def stop_collectors(self) -> None:
        await self.collector_supervisor.stop()

    async def _consume_raw_events(self) -> None:
        while self._running:
            raw = await self.raw_queue.get()
            try:
                await self.ingest_raw(raw)
            finally:
                self.raw_queue.task_done()
