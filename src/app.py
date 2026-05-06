from __future__ import annotations

import asyncio
import shutil
import sqlite3
import threading
from pathlib import Path
from typing import Any

from analysis import IncidentLifecycleManager
from collectors import AppHttpCollector, Collector, CollectorSupervisor, build_collectors
from config import InferraConfig, load_config
from core.enums import Severity
from core.logging import get_logger
from core.time import utc_now
from events.models import NormalizedEvent, RawEvent
from normalization import NormalizationPipeline
from normalization.dedup import DedupTracker
from normalization.noise import NoiseFilter
from runtime import ServiceGraph
from storage import BaselineStore, CalibrationStore, EventStore, IncidentStore, ServiceGraphStore, WeightStore
from storage import initialize_storage
from web.live_hub import LiveHub

_log = get_logger(__name__)


class InferraRuntime:
    def __init__(self, config: InferraConfig | None = None) -> None:
        self.config = config or load_config()
        self.event_store: EventStore
        self.incident_store: IncidentStore
        self.baseline_store: BaselineStore
        self.service_graph_store: ServiceGraphStore
        self.weight_store: WeightStore
        self.calibration_store: CalibrationStore
        mmap_bytes = (self.config.storage.mmap_size_mb * 1024 * 1024) if self.config.storage.enable_mmap else 0
        (
            self.event_store,
            self.incident_store,
            self.baseline_store,
            self.service_graph_store,
            self.weight_store,
            self.calibration_store,
        ) = initialize_storage(
            self.config.storage.data_dir,
            events_db_name=self.config.storage.events_db,
            incidents_db_name=self.config.storage.incidents_db,
            retention_hours=self.config.storage.retention_hours,
            prune_interval_seconds=self.config.storage.prune_interval_seconds,
            wal_mode=self.config.storage.wal_mode,
            mmap_size_bytes=mmap_bytes,
            archive_after_days=self.config.incident_lifecycle.archive_after_days,
        )
        self.service_graph = ServiceGraph.load(self.config.storage.data_dir / "service_graph.json")
        self.pipeline = NormalizationPipeline(self.config.normalization)
        self.dedup = DedupTracker(self.config.deduplication)
        self.noise_filter = NoiseFilter(self.config.noise_filter, data_dir=self.config.storage.data_dir)
        self.lifecycle = IncidentLifecycleManager(
            self.event_store,
            self.incident_store,
            self.service_graph,
            config=self.config,
            baseline_store=self.baseline_store,
            anomaly_detection=self.config.anomaly_detection,
            weight_store=self.weight_store,
            calibration_store=self.calibration_store,
            live_notify=self._live_enqueue,
        )
        self.raw_queue: asyncio.Queue[RawEvent] = asyncio.Queue(maxsize=10000)
        self.collectors = build_collectors(self.config, state_store=self.event_store)
        self._live_hub: LiveHub | None = None
        self._live_queue: asyncio.Queue[tuple[str, dict[str, Any]]] | None = None
        self._live_forward_task: asyncio.Task[None] | None = None
        for collector in self.collectors:
            if isinstance(collector, AppHttpCollector):
                collector.attach_queue(self.raw_queue)
        self.collector_supervisor = CollectorSupervisor(
            self.collectors,
            self.raw_queue,
            retry_initial_seconds=self.config.collectors.retry_initial_seconds,
            retry_max_seconds=self.config.collectors.retry_max_seconds,
        )
        self._consumer_task: asyncio.Task[None] | None = None
        self._running = False
        self._degraded_lock = threading.Lock()
        self._degraded_reasons: set[str] = set()
        self._storage_writes_ok = True
        self._storage_pause_scheduled = False
        self._asyncio_loop: asyncio.AbstractEventLoop | None = None

    def attach_live_hub(self, hub: LiveHub) -> None:
        self._live_hub = hub
        self._live_queue = asyncio.Queue(maxsize=4000)

    def _live_enqueue(self, kind: str, payload: dict[str, Any]) -> None:
        queue = self._live_queue
        if queue is None:
            return
        try:
            queue.put_nowait((kind, payload))
        except asyncio.QueueFull:
            _log.warning("live event queue saturated", extra={"kind": kind})

    async def _forward_live_events(self) -> None:
        hub = self._live_hub
        queue = self._live_queue
        if hub is None or queue is None:
            return
        while self._running:
            try:
                kind, payload = await asyncio.wait_for(queue.get(), timeout=1.0)
            except asyncio.TimeoutError:
                continue
            try:
                await hub.broadcast(kind, payload)
            except Exception as exc:
                _log.warning("live broadcast failed", extra={"kind": kind, "error": str(exc)})
            finally:
                queue.task_done()

    async def start(self, start_collectors: bool | None = None) -> None:
        self._running = True
        self._asyncio_loop = asyncio.get_running_loop()
        if self._live_queue is not None and self._live_hub is not None:
            self._live_forward_task = asyncio.create_task(self._forward_live_events())
        self._consumer_task = asyncio.create_task(self._consume_raw_events())
        should_start_collectors = self.config.collectors.auto_start if start_collectors is None else start_collectors
        if should_start_collectors:
            await self.collector_supervisor.start()

    async def stop(self) -> None:
        self._running = False
        await self.collector_supervisor.stop()
        await self.raw_queue.join()
        if self._live_forward_task:
            self._live_forward_task.cancel()
            try:
                await self._live_forward_task
            except asyncio.CancelledError:
                pass
            self._live_forward_task = None
        if self._consumer_task:
            self._consumer_task.cancel()
            try:
                await self._consumer_task
            except asyncio.CancelledError:
                pass
        self.noise_filter.persist_registry()
        self.event_store.close()
        self.incident_store.close()
        self._asyncio_loop = None

    async def ingest_raw(self, raw: RawEvent) -> str | None:
        event = self.pipeline.normalize(raw)

        if self.config.deduplication.enabled:
            result = self.dedup.check(event)
            for summary in result.summary_events:
                if not self._store_event(summary):
                    return None
            if result.decision == "suppress":
                self.noise_filter.record_event(event)
                return None

        event = self.noise_filter.annotate(event)
        self.noise_filter.record_event(event)

        if self.config.noise_filter.enabled and not self.noise_filter.should_store(event):
            return None

        if not self._store_event(event):
            return None
        return event.event_id

    def _note_storage_operational_error(self, exc: sqlite3.OperationalError) -> None:
        msg = str(exc).lower()
        schedule_pause = False
        with self._degraded_lock:
            self._storage_writes_ok = False
            if "readonly" in msg or "read-only" in msg:
                self._degraded_reasons.add("storage_readonly")
                want_pause = True
            elif "disk" in msg or "full" in msg:
                self._degraded_reasons.add("disk_full")
                want_pause = True
            else:
                self._degraded_reasons.add("sqlite_operational")
                want_pause = False
            if want_pause and not self._storage_pause_scheduled:
                self._storage_pause_scheduled = True
                schedule_pause = True
        if schedule_pause:
            self._schedule_collectors_pause_after_storage_loss()

    def _schedule_collectors_pause_after_storage_loss(self) -> None:
        loop = self._asyncio_loop
        if loop is None:
            return

        async def _pause() -> None:
            try:
                await self.collector_supervisor.stop()
            except Exception as exc:
                _log.warning("collector stop after storage failure failed", extra={"error": str(exc)})

        def _kick() -> None:
            asyncio.create_task(_pause())

        loop.call_soon_threadsafe(_kick)

    def degradation_snapshot(self) -> dict[str, Any]:
        data_dir = Path(self.config.storage.data_dir)
        free: int | None = None
        try:
            free = int(shutil.disk_usage(str(data_dir.resolve())).free)
        except OSError:
            pass
        with self._degraded_lock:
            reasons = sorted(self._degraded_reasons)
            writes_ok = self._storage_writes_ok
        q = self.raw_queue
        saturated = q.maxsize > 0 and q.qsize() >= max(1, int(q.maxsize * 0.9))
        merged = sorted(set(reasons))
        if free is not None and free < 64 * 1024 * 1024:
            merged = sorted(set([*merged, "disk_space_low"]))
        if saturated:
            merged = sorted(set([*merged, "raw_queue_saturated"]))
        degraded = bool(merged) or not writes_ok
        return {
            "degraded": degraded,
            "degraded_reasons": merged,
            "storage_writes_ok": writes_ok,
            "data_dir_bytes_free": free,
            "raw_queue_depth": q.qsize(),
            "raw_queue_maxsize": q.maxsize,
        }

    def _store_event(self, event: NormalizedEvent) -> bool:
        try:
            self.event_store.add_event(event)
        except sqlite3.OperationalError as exc:
            _log.error(
                "event store write failed",
                extra={"event_id": event.event_id, "error": str(exc)},
            )
            self._note_storage_operational_error(exc)
            return False
        self._live_enqueue(
            "event_count",
            {"total": self.event_store.count_events(), "last_event_id": event.event_id},
        )
        if event.severity >= Severity.WARN:
            self.lifecycle.analyze_recent()
        return True

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
        self.lifecycle.analyze_recent(window_seconds=3600)
        return max(0, count_after - count_before)

    def add_topology_relation(self, source: str, target: str, relation_type: str = "depends_on") -> None:
        self.service_graph.add_relation(source, target, relation_type)
        self.service_graph.save(self.config.storage.data_dir / "service_graph.json")

    def collector_health(self) -> list[dict]:
        rows = self.collector_supervisor.health()
        queue_depth = self.raw_queue.qsize()
        for row in rows:
            row["queue_depth"] = max(int(row.get("queue_depth", 0)), queue_depth)
        return rows

    def app_http_collectors(self) -> list[AppHttpCollector]:
        return [collector for collector in self.collectors if isinstance(collector, AppHttpCollector)]

    def collectors_for_source_type(self, source_type: str) -> list[Collector]:
        normalized = source_type.strip().lower()
        return [collector for collector in self.collectors if getattr(collector, "source_type", "").strip().lower() == normalized]

    async def start_collectors(self) -> None:
        await self.collector_supervisor.start()

    async def stop_collectors(self) -> None:
        await self.collector_supervisor.stop()

    async def start_collector(self, collector_id: str) -> bool:
        return await self.collector_supervisor.start_collector(collector_id)

    async def stop_collector(self, collector_id: str) -> bool:
        return await self.collector_supervisor.stop_collector(collector_id)

    async def collect_source_once(self, source_type: str) -> dict[str, int | list[str] | str]:
        if not self._running:
            raise RuntimeError("InferraRuntime must be started before running one-shot collection")

        collectors = self.collectors_for_source_type(source_type)
        if not collectors:
            raise ValueError(f"No collectors configured for source_type={source_type!r}")

        count_before = self.event_store.count_events()
        emitted = 0
        for collector in collectors:
            collect_once = getattr(collector, "collect_once", None)
            if not callable(collect_once):
                raise TypeError(f"Collector {collector.collector_id} does not support one-shot collection")
            emitted += int(await collect_once(self.raw_queue))
        await self.raw_queue.join()
        count_after = self.event_store.count_events()
        return {
            "source_type": source_type,
            "collector_count": len(collectors),
            "collector_ids": [collector.collector_id for collector in collectors],
            "raw_events_emitted": emitted,
            "events_stored": max(0, count_after - count_before),
        }

    async def _consume_raw_events(self) -> None:
        while self._running:
            raw = await self.raw_queue.get()
            try:
                await self.ingest_raw(raw)
            finally:
                self.raw_queue.task_done()
