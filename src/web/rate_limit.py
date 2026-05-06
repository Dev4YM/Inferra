from __future__ import annotations

import threading
import time


class TokenBucket:
    def __init__(self, capacity: float, refill_per_second: float) -> None:
        self.capacity = capacity
        self.refill_per_second = refill_per_second
        self.tokens = capacity
        self.last = time.monotonic()

    def consume(self, cost: float = 1.0) -> bool:
        now = time.monotonic()
        elapsed = now - self.last
        self.last = now
        self.tokens = min(self.capacity, self.tokens + elapsed * self.refill_per_second)
        if self.tokens >= cost:
            self.tokens -= cost
            return True
        return False


class HostRateLimiter:
    def __init__(self, tokens_per_minute: float, burst: float = 8.0) -> None:
        self.refill_per_second = float(tokens_per_minute) / 60.0
        self.capacity = max(float(burst), self.refill_per_second * 3.0)
        self._lock = threading.Lock()
        self._buckets: dict[str, TokenBucket] = {}

    def consume(self, key: str) -> bool:
        with self._lock:
            bucket = self._buckets.setdefault(key, TokenBucket(self.capacity, self.refill_per_second))
            return bucket.consume(1.0)
