from __future__ import annotations

from enum import Enum, IntEnum


class Severity(IntEnum):
    DEBUG = 0
    INFO = 1
    WARN = 2
    ERROR = 3
    CRITICAL = 4


class EventType(IntEnum):
    LOG = 0
    METRIC = 1
    STATE_CHANGE = 2
    HEALTH_CHECK = 3


class IncidentState(str, Enum):
    OPEN = "open"
    INVESTIGATING = "investigating"
    EXPLAINED = "explained"
    RESOLVED = "resolved"
    STALE = "stale"


class CauseType(str, Enum):
    DEPENDENCY_FAILURE = "dependency_failure"
    RESOURCE_EXHAUSTION = "resource_exhaustion"
    APPLICATION_BUG = "application_bug"
    INFRASTRUCTURE_FAILURE = "infrastructure_failure"
    CONFIGURATION_ERROR = "configuration_error"
    DATABASE_FAILURE = "database_failure"
    UNKNOWN = "unknown"
