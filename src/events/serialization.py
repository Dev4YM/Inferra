from __future__ import annotations

import json
from typing import Any

from core.enums import EventType, Severity
from core.time import parse_datetime, to_iso
from events.models import DataQuality, NormalizedEvent, SourceRef


def event_to_dict(event: NormalizedEvent) -> dict[str, Any]:
    return {
        "event_id": event.event_id,
        "timestamp": to_iso(event.timestamp),
        "timestamp_source": event.timestamp_source,
        "service_id": event.service_id,
        "host_id": event.host_id,
        "severity": int(event.severity),
        "event_type": int(event.event_type),
        "message": event.message,
        "structured_data": event.structured_data,
        "tags": sorted(event.tags),
        "fingerprint": event.fingerprint,
        "quality": {
            "overall": event.quality.overall,
            "timestamp_confidence": event.quality.timestamp_confidence,
            "parse_confidence": event.quality.parse_confidence,
            "identity_confidence": event.quality.identity_confidence,
            "completeness": event.quality.completeness,
        },
        "source_ref": {
            "source_type": event.source_ref.source_type,
            "source_id": event.source_ref.source_id,
            "raw_offset": event.source_ref.raw_offset,
            "collected_at": to_iso(event.source_ref.collected_at),
        },
        "schema_version": event.schema_version,
    }


def event_from_dict(data: dict[str, Any]) -> NormalizedEvent:
    timestamp = parse_datetime(data["timestamp"])
    collected_at = parse_datetime(data["source_ref"]["collected_at"])
    if timestamp is None or collected_at is None:
        raise ValueError("Invalid event timestamp")
    quality_data = data["quality"]
    return NormalizedEvent(
        event_id=data["event_id"],
        timestamp=timestamp,
        timestamp_source=data["timestamp_source"],
        service_id=data["service_id"],
        host_id=data["host_id"],
        severity=Severity(data["severity"]),
        event_type=EventType(data["event_type"]),
        message=data["message"],
        structured_data=dict(data.get("structured_data") or {}),
        tags=frozenset(data.get("tags") or ()),
        fingerprint=data["fingerprint"],
        quality=DataQuality(**quality_data),
        source_ref=SourceRef(
            source_type=data["source_ref"]["source_type"],
            source_id=data["source_ref"]["source_id"],
            raw_offset=data["source_ref"].get("raw_offset"),
            collected_at=collected_at,
        ),
        schema_version=data.get("schema_version", 1),
    )


def json_dumps(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))
