# Event Deduplication

## Problem

Many failure scenarios produce repeated, near-identical log entries. A connection retry loop might emit "connection refused" 500 times in 10 seconds. Without deduplication, these flood the event store, degrade query performance, and distort frequency-based scoring (a hypothesis looks artificially strong because the same signal was counted 500 times instead of once).

---

## Goals

1. Reduce event volume by collapsing duplicates into counted occurrences.
2. Preserve the first and last occurrence of each duplicate group (for timeline accuracy).
3. Maintain exact counts (so frequency-based analysis remains accurate).
4. Operate in near-real-time (no batch-mode post-processing).

---

## Fingerprint-Based Deduplication

### Fingerprint Definition

Two events are considered duplicates if they share the same `fingerprint` (see `event_model.md`). The fingerprint is computed from:

```
fingerprint = SHA-256(service_id + "|" + templatized_message + "|" + severity)[:32]
```

The `templatize_message` function replaces variable parts (IPs, timestamps, UUIDs, numbers, paths, quoted strings) with placeholders. This means:

- `"Connection to 10.0.0.5:5432 refused"` and `"Connection to 10.0.0.6:5432 refused"` → same fingerprint
- `"Connection to 10.0.0.5:5432 refused"` and `"Connection to 10.0.0.5:5432 timed out"` → different fingerprints
- `"Error in handler at line 42"` and `"Error in handler at line 87"` → same fingerprint

### Deduplication Window

Deduplication operates within a sliding time window per fingerprint:

```
┌──────────────────────────────────────────────────────┐
│ Dedup Window (default: 60 seconds)                    │
│                                                       │
│  Event 1 (stored)  ...duplicates suppressed...  Event N (stored, with count)
│  ▲ first_seen                                    ▲ last_seen
└──────────────────────────────────────────────────────┘
```

**Window behavior**:
- First event with a new fingerprint: stored normally.
- Subsequent events with the same fingerprint within the window: suppressed (not stored individually).
- The dedup tracker updates a counter and the `last_seen` timestamp.
- When the window expires (no new event with this fingerprint for `window_duration` seconds), the tracker emits a summary:

```python
@dataclass
class DedupSummary:
    fingerprint: str
    first_event_id: str       # the stored first occurrence
    last_event: NormalizedEvent  # the last occurrence (stored)
    suppressed_count: int     # how many were suppressed (total = suppressed + 2)
    window_start: datetime
    window_end: datetime
```

The last event is stored with an additional structured_data field:
```python
structured_data["_dedup_count"] = suppressed_count + 1  # +1 for the last event itself
structured_data["_dedup_first_event_id"] = first_event_id
structured_data["_dedup_window_seconds"] = (window_end - window_start).total_seconds()
```

---

## Implementation: Sliding Window Tracker

```python
class DedupTracker:
    """Tracks active deduplication windows."""

    def __init__(self, window_seconds: int = 60, max_tracked: int = 10000):
        self.window_seconds = window_seconds
        self.max_tracked = max_tracked
        self._windows: dict[str, DedupWindow] = {}  # fingerprint → window

    def check(self, event: NormalizedEvent) -> DedupDecision:
        """Check if event should be stored, suppressed, or closes a window."""
        fp = event.fingerprint
        now = event.timestamp

        if fp in self._windows:
            window = self._windows[fp]
            window.count += 1
            window.last_event = event
            window.last_seen = now
            return DedupDecision.SUPPRESS

        # New fingerprint: start a new window
        if len(self._windows) >= self.max_tracked:
            self._evict_oldest()

        self._windows[fp] = DedupWindow(
            fingerprint=fp,
            first_event=event,
            last_event=event,
            count=1,
            first_seen=now,
            last_seen=now,
        )
        return DedupDecision.STORE

    def expire_windows(self, now: datetime) -> list[DedupSummary]:
        """Called periodically. Returns summaries for expired windows."""
        expired = []
        cutoff = now - timedelta(seconds=self.window_seconds)
        to_remove = []

        for fp, window in self._windows.items():
            if window.last_seen < cutoff:
                to_remove.append(fp)
                if window.count > 1:
                    expired.append(DedupSummary(
                        fingerprint=fp,
                        first_event_id=window.first_event.event_id,
                        last_event=window.last_event,
                        suppressed_count=window.count - 1,
                        window_start=window.first_seen,
                        window_end=window.last_seen,
                    ))

        for fp in to_remove:
            del self._windows[fp]

        return expired

class DedupDecision(Enum):
    STORE = "store"          # new fingerprint, store this event
    SUPPRESS = "suppress"    # duplicate within window, do not store
```

---

## Edge Cases

### 1. Severity Escalation Within a Window

If a duplicate event arrives with a higher severity than the first event (e.g., first was WARN, now ERROR), the window is **split**: the higher-severity event is stored as a new event (it indicates escalation, which is analytically significant), and a new dedup window starts for the higher severity.

### 2. Long-Running Duplicate Storms

If a fingerprint keeps producing events indefinitely (window never expires), periodic summaries are emitted every `window_seconds` to prevent unbounded counting without output:

```
t=0s:   Event stored (first)
t=1-59s: 200 events suppressed
t=60s:  Summary emitted (count=200), last event stored, window resets
t=61-119s: 180 events suppressed
t=120s: Summary emitted (count=180), last event stored, window resets
...
```

### 3. Cross-Service Duplicates

Deduplication is per-fingerprint, which includes `service_id`. The same error message from two different services is NOT deduplicated — this is intentional, because the same error from two services is a correlation signal, not noise.

### 4. Tracker Memory Limits

The tracker holds at most `max_tracked` (default: 10,000) active windows. If this limit is reached:
1. Evict the window with the oldest `last_seen` timestamp.
2. Emit its summary (if count > 1).
3. Insert the new window.

This bounds memory usage to approximately `max_tracked × 200 bytes ≈ 2MB`.

---

## Configuration

```toml
[deduplication]
enabled = true
window_seconds = 60
max_tracked_fingerprints = 10000
periodic_summary_interval_seconds = 60  # for long-running storms
severity_escalation_splits = true       # split window on severity increase
```

---

## Interaction with Downstream Systems

### Scoring Engine
The scoring engine uses `_dedup_count` from structured_data to weight frequency signals:
- `frequency_raw = count of distinct events matching a hypothesis`
- `frequency_weighted = sum of (1 + _dedup_count) for each matching event`

This means a hypothesis supported by one event that represented 500 suppressed duplicates is scored differently than one supported by one singular event.

### Anomaly Detection
Anomaly detection operates on the raw event counts (including duplicates), not on deduplicated counts. The dedup summary's count is used to reconstruct the true volume:
- `true_event_count = stored_events + sum(suppressed_counts)`

This ensures rate-based anomaly detection catches spikes even when individual events are suppressed.

---

## Metrics

| Metric | Description |
|---|---|
| `dedup_events_suppressed_total` | Total events suppressed |
| `dedup_windows_active` | Currently active dedup windows |
| `dedup_summaries_emitted_total` | Summaries emitted on window expiry |
| `dedup_compression_ratio` | `suppressed / (suppressed + stored)` over last 5 minutes |
| `dedup_evictions_total` | Windows evicted due to capacity limit |
