from __future__ import annotations

import json
import math
import threading
from datetime import datetime
from pathlib import Path

from core.time import parse_datetime, to_iso


class MetricRingbuffer:
    """Fixed-size circular buffer for one metric series."""

    def __init__(
        self,
        *,
        service_id: str = "",
        metric_name: str = "",
        capacity: int = 720,
    ) -> None:
        self.service_id = service_id
        self.metric_name = metric_name
        self.capacity = capacity
        self.values: list[float | None] = [None] * capacity
        self.timestamps: list[datetime | None] = [None] * capacity
        self.head = 0
        self.count = 0
        self._lock = threading.RLock()

    def append(self, timestamp: datetime, value: float) -> None:
        with self._lock:
            self.timestamps[self.head] = timestamp
            self.values[self.head] = float(value)
            self.head = (self.head + 1) % self.capacity
            self.count = min(self.count + 1, self.capacity)

    def query_range(self, start: datetime, end: datetime) -> list[tuple[datetime, float]]:
        return [
            (timestamp, value)
            for timestamp, value in self._ordered_items()
            if start <= timestamp <= end
        ]

    def last_n(self, n: int) -> list[tuple[datetime, float]]:
        if n <= 0:
            return []
        return self._ordered_items()[-n:]

    def mean_per_minute(self) -> float:
        values = [value for _, value in self._ordered_items()]
        if not values:
            return 0.0
        # Metric buckets are 5 minutes wide, so convert average bucket value to per-minute rate.
        return sum(values) / len(values) / 5.0

    def coefficient_of_variation(self) -> float:
        values = [value for _, value in self._ordered_items()]
        if len(values) < 2:
            return 0.0
        mean = sum(values) / len(values)
        if abs(mean) < 1e-12:
            return 0.0
        variance = sum((value - mean) ** 2 for value in values) / len(values)
        return math.sqrt(variance) / abs(mean)

    def save_to_json(self, path: str | Path | None = None) -> Path:
        target = Path(path) if path is not None else self.default_path()
        target.parent.mkdir(parents=True, exist_ok=True)
        temp_path = target.with_suffix(target.suffix + ".tmp")
        payload = {
            "service_id": self.service_id,
            "metric_name": self.metric_name,
            "capacity": self.capacity,
            "head": self.head,
            "count": self.count,
            "entries": [
                {"timestamp": to_iso(timestamp), "value": value}
                for timestamp, value in self._ordered_items()
            ],
        }
        temp_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
        temp_path.replace(target)
        return target

    @classmethod
    def load_from_json(cls, path: str | Path) -> "MetricRingbuffer":
        source = Path(path)
        if not source.exists():
            return cls()
        data = json.loads(source.read_text(encoding="utf-8"))
        ringbuffer = cls(
            service_id=data.get("service_id", ""),
            metric_name=data.get("metric_name", ""),
            capacity=int(data.get("capacity", 720)),
        )
        for entry in data.get("entries", []):
            timestamp = parse_datetime(entry["timestamp"])
            if timestamp is None:
                continue
            ringbuffer.append(timestamp, float(entry["value"]))
        return ringbuffer

    def default_path(self) -> Path:
        filename = f"{self.service_id}_{self.metric_name}.json".strip("_")
        return Path("./data/metrics") / filename

    def _ordered_items(self) -> list[tuple[datetime, float]]:
        with self._lock:
            if self.count == 0:
                return []
            start = (self.head - self.count) % self.capacity
            items: list[tuple[datetime, float]] = []
            for index in range(self.count):
                slot = (start + index) % self.capacity
                timestamp = self.timestamps[slot]
                value = self.values[slot]
                if timestamp is None or value is None:
                    continue
                items.append((timestamp, value))
            return items
