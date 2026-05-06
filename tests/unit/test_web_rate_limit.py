from __future__ import annotations

from web.rate_limit import HostRateLimiter, TokenBucket


def test_token_bucket_allows_burst_then_blocks() -> None:
    bucket = TokenBucket(capacity=2.0, refill_per_second=0.0)
    assert bucket.consume(1.0) is True
    assert bucket.consume(1.0) is True
    assert bucket.consume(1.0) is False


def test_host_rate_limiter_tracks_keys_independently() -> None:
    limiter = HostRateLimiter(tokens_per_minute=600.0, burst=10.0)
    assert limiter.consume("a") is True
    assert limiter.consume("b") is True
