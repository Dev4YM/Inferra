from __future__ import annotations

from collections.abc import Mapping
from dataclasses import dataclass, field
from datetime import datetime
from types import MappingProxyType
from typing import Any

from core.enums import EventType, Severity


def freeze_value(value: Any) -> Any:
    if isinstance(value, Mapping):
        return MappingProxyType({str(key): freeze_value(item) for key, item in value.items()})
    if isinstance(value, list | tuple):
        return tuple(freeze_value(item) for item in value)
    if isinstance(value, set | frozenset):
        return frozenset(freeze_value(item) for item in value)
    return value


def thaw_value(value: Any) -> Any:
    if isinstance(value, Mapping):
        return {str(key): thaw_value(item) for key, item in value.items()}
    if isinstance(value, tuple):
        return [thaw_value(item) for item in value]
    if isinstance(value, frozenset):
        return sorted(thaw_value(item) for item in value)
    return value


@dataclass(frozen=True)
class RawEvent:
    source_type: str
    source_id: str
    raw_payload: str
    collected_at: datetime
    metadata: Mapping[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        object.__setattr__(self, "metadata", freeze_value(dict(self.metadata)))


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
    flags: frozenset[str] = frozenset()

    def __post_init__(self) -> None:
        object.__setattr__(self, "flags", frozenset(str(flag) for flag in self.flags if flag))


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
    structured_data: Mapping[str, Any]
    tags: frozenset[str]
    fingerprint: str
    quality: DataQuality
    source_ref: SourceRef
    schema_version: int = 1

    def __post_init__(self) -> None:
        object.__setattr__(self, "structured_data", freeze_value(dict(self.structured_data)))
        object.__setattr__(self, "tags", frozenset(str(tag) for tag in self.tags if tag))


@dataclass(frozen=True)
class EventFilter:
    service_ids: set[str] | None = None
    host_ids: set[str] | None = None
    severities: set[Severity] | None = None
    event_types: set[EventType] | None = None
    tags: set[str] | None = None
    message_contains: str | None = None
