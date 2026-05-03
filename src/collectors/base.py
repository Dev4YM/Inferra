from __future__ import annotations

import asyncio
from dataclasses import dataclass
from datetime import datetime
from typing import Protocol

from events.models import RawEvent


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


class Collector(Protocol):
    @property
    def collector_id(self) -> str: ...

    @property
    def source_type(self) -> str: ...

    async def start(self, sink: asyncio.Queue[RawEvent]) -> None: ...

    async def stop(self) -> None: ...

    def health_check(self) -> CollectorHealth: ...
