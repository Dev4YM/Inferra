from __future__ import annotations

import asyncio
import random
import time
from abc import ABC, abstractmethod
from dataclasses import dataclass
from datetime import datetime
from typing import Protocol

from core.logging import get_logger
from core.time import utc_now
from events.models import RawEvent


class CollectorStateStore(Protocol):
    def get_collector_state(self, collector_id: str, state_key: str) -> str | None: ...

    def set_collector_state(self, collector_id: str, state_key: str, state_value: str) -> None: ...


@dataclass(frozen=True)
class CollectorHealth:
    collector_id: str
    source_type: str
    is_running: bool
    events_emitted: int
    events_per_second: float = 0.0
    last_event_at: datetime | None = None
    error_count: int = 0
    last_error: str | None = None
    lag_seconds: float | None = None
    dropped_events: int = 0
    queue_depth: int = 0


class Collector(ABC):
    def __init__(
        self,
        *,
        state_store: CollectorStateStore | None = None,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        self.state_store = state_store
        self.emit_timeout_seconds = emit_timeout_seconds
        self._logger = get_logger(__name__)
        self._running = False
        self._stop_event = asyncio.Event()
        self._events_emitted = 0
        self._error_count = 0
        self._last_error: str | None = None
        self._last_event_at: datetime | None = None
        self._last_queue_depth = 0
        self._dropped_events = 0
        self._started_monotonic: float | None = None

    @property
    @abstractmethod
    def collector_id(self) -> str:
        raise NotImplementedError

    @property
    @abstractmethod
    def source_type(self) -> str:
        raise NotImplementedError

    @abstractmethod
    async def run(self, queue: asyncio.Queue[RawEvent]) -> None:
        raise NotImplementedError

    async def start(self, queue: asyncio.Queue[RawEvent]) -> None:
        await self.run(queue)

    async def stop(self) -> None:
        self._stop_event.set()

    def health(self) -> CollectorHealth:
        elapsed = 0.0
        if self._started_monotonic is not None:
            elapsed = max(0.0, time.monotonic() - self._started_monotonic)
        return CollectorHealth(
            collector_id=self.collector_id,
            source_type=self.source_type,
            is_running=self._running,
            events_emitted=self._events_emitted,
            events_per_second=(self._events_emitted / elapsed) if elapsed > 0 else 0.0,
            last_event_at=self._last_event_at,
            error_count=self._error_count,
            last_error=self._last_error,
            lag_seconds=(utc_now() - self._last_event_at).total_seconds() if self._last_event_at else None,
            dropped_events=self._dropped_events,
            queue_depth=self._last_queue_depth,
        )

    def health_check(self) -> CollectorHealth:
        return self.health()

    def checkpoint_load(self, state_key: str, default: str | None = None) -> str | None:
        if self.state_store is None:
            return default
        value = self.state_store.get_collector_state(self.collector_id, state_key)
        return value if value is not None else default

    def checkpoint_save(self, state_key: str, state_value: str) -> None:
        if self.state_store is None:
            return
        self.state_store.set_collector_state(self.collector_id, state_key, state_value)

    async def emit(self, queue: asyncio.Queue[RawEvent], raw: RawEvent) -> bool:
        self._last_queue_depth = queue.qsize()
        try:
            if queue.full():
                await asyncio.sleep(random.uniform(0.0, 0.05))
                await asyncio.wait_for(queue.put(raw), timeout=self.emit_timeout_seconds)
            else:
                queue.put_nowait(raw)
        except asyncio.QueueFull:
            try:
                await asyncio.sleep(random.uniform(0.0, 0.05))
                await asyncio.wait_for(queue.put(raw), timeout=self.emit_timeout_seconds)
            except TimeoutError:
                self._record_drop(queue)
                return False
        except TimeoutError:
            self._record_drop(queue)
            return False
        self._events_emitted += 1
        self._last_event_at = raw.collected_at
        self._last_queue_depth = queue.qsize()
        return True

    async def _mark_running(self) -> None:
        self._stop_event = asyncio.Event()
        self._running = True
        self._started_monotonic = time.monotonic()

    async def _mark_stopped(self) -> None:
        self._running = False

    def _record_error(self, error: Exception | str) -> None:
        self._error_count += 1
        self._last_error = str(error)
        self._logger.warning(
            "Collector cycle failed",
            extra={"collector_id": self.collector_id, "source_type": self.source_type, "error": self._last_error},
        )

    def _record_drop(self, queue: asyncio.Queue[RawEvent]) -> None:
        self._dropped_events += 1
        self._last_queue_depth = queue.qsize()
        self._last_error = "queue put timed out"
        self._logger.warning(
            "Collector dropped event under backpressure",
            extra={
                "collector_id": self.collector_id,
                "source_type": self.source_type,
                "queue_depth": self._last_queue_depth,
                "emit_timeout_seconds": self.emit_timeout_seconds,
            },
        )

    def _should_stop(self) -> bool:
        return self._stop_event.is_set()


class PollingCollector(Collector, ABC):
    def __init__(
        self,
        *,
        poll_interval_seconds: float,
        state_store: CollectorStateStore | None = None,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(state_store=state_store, emit_timeout_seconds=emit_timeout_seconds)
        self.poll_interval_seconds = poll_interval_seconds

    @abstractmethod
    async def collect_once(self, queue: asyncio.Queue[RawEvent]) -> int:
        raise NotImplementedError

    async def run(self, queue: asyncio.Queue[RawEvent]) -> None:
        await self._mark_running()
        try:
            while not self._should_stop():
                try:
                    await self.collect_once(queue)
                except asyncio.CancelledError:
                    raise
                except Exception as exc:
                    self._record_error(exc)
                try:
                    await asyncio.wait_for(self._stop_event.wait(), timeout=self.poll_interval_seconds)
                except TimeoutError:
                    continue
        finally:
            await self._mark_stopped()
