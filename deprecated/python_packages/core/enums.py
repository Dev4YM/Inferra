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
    MERGED = "merged"
    ARCHIVED = "archived"


class CauseType(str, Enum):
    DEPENDENCY_FAILURE = "dependency_failure"
    RESOURCE_EXHAUSTION = "resource_exhaustion"
    APPLICATION_BUG = "application_bug"
    INFRASTRUCTURE_FAILURE = "infrastructure_failure"
    CONFIGURATION_ERROR = "configuration_error"
    DATABASE_FAILURE = "database_failure"
    UNKNOWN = "unknown"


class InferenceEdgeType(str, Enum):
    DEPENDENCY_PROPAGATION = "dependency_propagation"
    SAME_SERVICE_ESCALATION = "same_service_escalation"
    RESOURCE_PRECEDED_ERROR = "resource_preceded_error"
    TIMEOUT_CHAIN = "timeout_chain"
    RESTART_PRECEDED_DISCONNECTION = "restart_preceded_disconnection"
    CONFIG_PRECEDED_ERROR = "config_preceded_error"
    SHARED_FATE = "shared_fate"


class ServiceHealthState(str, Enum):
    HEALTHY = "healthy"
    DEGRADED = "degraded"
    FAILING = "failing"
    UNREACHABLE = "unreachable"
    UNKNOWN = "unknown"


class DedupDecision(str, Enum):
    STORE = "store"
    SUPPRESS = "suppress"


class CollectorState(str, Enum):
    DISABLED = "disabled"
    STARTING = "starting"
    RUNNING = "running"
    RETRYING = "retrying"
    FAILED = "failed"
    STOPPED = "stopped"
