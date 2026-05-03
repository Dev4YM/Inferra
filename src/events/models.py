from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Any

from core.enums import EventType, Severity


@dataclass(frozen=True)
class RawEvent:
    source_type: str
    source_id: str
    raw_payload: str
    collected_at: datetime
    metadata: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True)
class SourceRef:
    source_type: str
    source_id: str
    raw_offset: int | None
    collected_at: datetime


@dataclass(frozen=True)
class DataQuality:
    overall: float
    timestamp_confidence: float
    parse_confidence: float
    identity_confidence: float
    completeness: float


@dataclass(frozen=True)
class NormalizedEvent:
    event_id: str
    timestamp: datetime
    timestamp_source: str
    service_id: str
    host_id: str
    severity: Severity
    event_type: EventType
    message: str
    structured_data: dict[str, Any]
    tags: frozenset[str]
    fingerprint: str
    quality: DataQuality
    source_ref: SourceRef
    schema_version: int = 1


@dataclass(frozen=True)
class EventFilter:
    service_ids: set[str] | None = None
    host_ids: set[str] | None = None
    severities: set[Severity] | None = None
    event_types: set[EventType] | None = None
    tags: set[str] | None = None
    message_contains: str | None = None
