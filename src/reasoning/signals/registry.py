from __future__ import annotations

from .detectors import DETECTOR_FUNCS
from .types import Signal, SignalContext


def collect_signals(ctx: SignalContext) -> tuple[Signal, ...]:
    found: list[Signal] = []
    for fn in DETECTOR_FUNCS:
        found.extend(fn(ctx))
    merged: dict[tuple[str, str | None, tuple[str, ...]], Signal] = {}
    for s in sorted(found, key=lambda x: (x.name, x.service_id or "", x.evidence_event_ids, -x.confidence)):
        key = (s.name, s.service_id, s.evidence_event_ids)
        prev = merged.get(key)
        if prev is None or s.confidence > prev.confidence:
            merged[key] = s
    return tuple(sorted(merged.values(), key=lambda x: (x.name, x.service_id or "", x.evidence_event_ids)))
