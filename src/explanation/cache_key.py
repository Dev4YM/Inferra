from __future__ import annotations

import hashlib
import json
from typing import Any

from core.time import to_iso
from events.models import NormalizedEvent


def hypotheses_hash(hypotheses: list[dict[str, Any]]) -> str:
    normalized: list[dict[str, Any]] = []
    for hyp in sorted(hypotheses, key=lambda item: (int(item.get("rank") or 0), str(item.get("hypothesis_id") or ""))):
        normalized.append({key: hyp[key] for key in sorted(hyp.keys())})
    blob = json.dumps(normalized, sort_keys=True, separators=(",", ":"), default=str)
    return hashlib.sha256(blob.encode("utf-8")).hexdigest()


def events_hash_head(events: list[NormalizedEvent]) -> str:
    ordered = sorted(events, key=lambda item: (item.timestamp, item.event_id))
    lines = [
        f"{event.event_id}\t{to_iso(event.timestamp)}\t{event.service_id}\t{event.fingerprint}"
        for event in ordered[:200]
    ]
    return hashlib.sha256("\n".join(lines).encode("utf-8")).hexdigest()


def explanation_cache_key_hashes(
    hypotheses: list[dict[str, Any]],
    events: list[NormalizedEvent],
) -> tuple[str, str]:
    return hypotheses_hash(hypotheses), events_hash_head(events)


def stable_template_explanation_id(incident_id: str, hypotheses: list[dict[str, Any]], events: list[NormalizedEvent]) -> str:
    hyp_h, evt_h = explanation_cache_key_hashes(hypotheses, events)
    inner = hashlib.sha256(f"{incident_id}|{hyp_h}|{evt_h}".encode("utf-8")).hexdigest()
    return f"exp-tpl-{inner[:24]}"
