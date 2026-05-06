from __future__ import annotations

import asyncio
import random
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from collectors.base import Collector
from core.time import to_iso, utc_now
from events.models import RawEvent


@dataclass
class CollectorRuntimeState:
    collector: Collector
    status: str = "stopped"
    attempts: int = 0
    last_started_at: datetime | None = None
    last_stopped_at: datetime | None = None
    next_retry_at: datetime | None = None
    last_error: str | None = None
    task: asyncio.Task[None] | None = None


class CollectorSupervisor:
    def __init__(
        self,
        collectors: list[Collector],
        sink: asyncio.Queue[RawEvent],
        retry_initial_seconds: float = 1.0,
        retry_max_seconds: float = 60.0,
    ) -> None:
        self.collectors = collectors
        self.sink = sink
        self.retry_initial_seconds = retry_initial_seconds
        self.retry_max_seconds = retry_max_seconds
        self._states = {collector.collector_id: CollectorRuntimeState(collector=collector) for collector in collectors}
        self._stopping = False

    async def start(self) -> None:
        self._stopping = False
        for state in self._states.values():
            if state.task is None or state.task.done():
                state.task = asyncio.create_task(self._run_collector(state), name=f"inferra:{state.collector.collector_id}")

    async def stop(self) -> None:
        self._stopping = True
        for state in self._states.values():
            await state.collector.stop()
        for state in self._states.values():
            if state.task and not state.task.done():
                state.task.cancel()
        await asyncio.gather(
            *(state.task for state in self._states.values() if state.task is not None),
            return_exceptions=True,
        )
        for state in self._states.values():
            state.status = "stopped"
            state.last_stopped_at = utc_now()

    async def start_collector(self, collector_id: str) -> bool:
        if self._stopping:
            return False
        state = self._states.get(collector_id)
        if state is None:
            return False
        if state.task is not None and not state.task.done():
            return True
        state.task = asyncio.create_task(self._run_collector(state), name=f"inferra:{collector_id}")
        return True

    async def stop_collector(self, collector_id: str) -> bool:
        state = self._states.get(collector_id)
        if state is None:
            return False
        await state.collector.stop()
        task = state.task
        if task is not None and not task.done():
            task.cancel()
            try:
                await task
            except asyncio.CancelledError:
                pass
        state.task = None
        state.status = "stopped"
        state.last_stopped_at = utc_now()
        return True

    def health(self) -> list[dict[str, Any]]:
        rows: list[dict[str, Any]] = []
        for state in sorted(self._states.values(), key=lambda item: item.collector.collector_id):
            collector_health = state.collector.health()
            rows.append(
                {
                    "collector_id": collector_health.collector_id,
                    "source_type": collector_health.source_type,
                    "status": state.status,
                    "is_running": collector_health.is_running,
                    "events_emitted": collector_health.events_emitted,
                    "events_per_second": collector_health.events_per_second,
                    "last_event_at": to_iso(collector_health.last_event_at) if collector_health.last_event_at else None,
                    "error_count": collector_health.error_count,
                    "dropped_events": collector_health.dropped_events,
                    "queue_depth": collector_health.queue_depth,
                    "last_error": state.last_error or collector_health.last_error,
                    "lag_seconds": collector_health.lag_seconds,
                    "attempts": state.attempts,
                    "last_started_at": to_iso(state.last_started_at) if state.last_started_at else None,
                    "last_stopped_at": to_iso(state.last_stopped_at) if state.last_stopped_at else None,
                    "next_retry_at": to_iso(state.next_retry_at) if state.next_retry_at else None,
                }
            )
        return rows

    async def _run_collector(self, state: CollectorRuntimeState) -> None:
        backoff = self.retry_initial_seconds
        while not self._stopping:
            try:
                state.status = "running"
                state.last_started_at = utc_now()
                state.next_retry_at = None
                await state.collector.run(self.sink)
                if not self._stopping:
                    state.status = "retrying"
                    state.last_error = "collector exited unexpectedly"
            except asyncio.CancelledError:
                state.status = "stopped"
                raise
            except Exception as exc:
                state.status = "retrying"
                state.attempts += 1
                state.last_error = str(exc)
            if not self._stopping:
                state.next_retry_at = utc_now()
                jitter = 1.0 + random.random() * 0.25
                await asyncio.sleep(backoff * jitter)
                backoff = min(self.retry_max_seconds, backoff * 2)
        state.status = "stopped"
