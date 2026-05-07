from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timedelta
from enum import Enum
from typing import Any

from .enums import (
    CauseType,
    CollectorState,
    EventType,
    IncidentState,
    InferenceEdgeType,
    ServiceHealthState,
    Severity,
)


@dataclass
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

    compute_quality = staticmethod(lambda event, parse_meta: compute_quality(event, parse_meta))


@dataclass(frozen=True)
class NormalizedEvent:
    event_id: str
    timestamp: datetime
    timestamp_source: str
    service_id: str
    host_id: str
    source_ref: SourceRef
    severity: Severity
    event_type: EventType
    message: str
    structured_data: dict[str, Any]
    tags: frozenset[str]
    fingerprint: str
    quality: DataQuality
    schema_version: int = 1


def compute_quality(event: NormalizedEvent, parse_meta: Any) -> DataQuality:
    method = _meta_value(parse_meta, "method", "freetext")
    service_source = _meta_value(parse_meta, "service_source", "unknown")
    severity_explicit = bool(_meta_value(parse_meta, "severity_explicit", False))
    ts_conf = 1.0 if event.timestamp_source == "parsed" else 0.5 if event.timestamp_source == "collected_at" else 0.3
    parse_conf = {"json": 1.0, "syslog": 0.9, "kv": 0.8, "regex": 0.7, "grok": 0.7, "freetext": 0.3}.get(
        method, 0.3
    )
    identity_conf = (
        1.0
        if service_source == "config"
        else 0.8
        if service_source == "docker_label"
        else 0.7
        if service_source == "container_name"
        else 0.5
        if service_source == "journald_unit"
        else 0.4
    )
    populated = sum(
        1
        for value in [
            event.service_id != "unknown",
            event.host_id != "unknown",
            event.severity != Severity.INFO or severity_explicit,
            len(event.structured_data) > 0,
            len(event.tags) > 0,
        ]
        if value
    ) / 5.0
    overall = 0.3 * ts_conf + 0.3 * parse_conf + 0.2 * identity_conf + 0.2 * populated
    return DataQuality(overall, ts_conf, parse_conf, identity_conf, populated)


def _meta_value(meta: Any, key: str, default: Any) -> Any:
    if isinstance(meta, dict):
        return meta.get(key, default)
    return getattr(meta, key, default)


@dataclass
class EventFilter:
    service_ids: set[str] | None = None
    host_ids: set[str] | None = None
    severities: set[Severity] | None = None
    event_types: set[EventType] | None = None
    tags: set[str] | None = None
    message_contains: str | None = None


@dataclass
class CorrelationEdge:
    source_event_id: str
    target_event_id: str
    edge_type: str
    weight: float
    evidence: str


@dataclass
class EventCluster:
    cluster_id: str
    events: list[str]
    time_range: tuple[datetime, datetime]
    affected_services: set[str]
    primary_severity: Severity
    trigger_event_id: str
    correlation_edges: list[CorrelationEdge] = field(default_factory=list)
    anomaly_scores: dict[str, float] = field(default_factory=dict)


@dataclass
class Incident:
    incident_id: str
    state: IncidentState
    created_at: datetime
    updated_at: datetime
    clusters: list[str]
    events: list[str]
    affected_services: set[str]
    primary_service: str | None
    time_range: tuple[datetime, datetime]
    severity: Severity
    runtime_context: RuntimeContext | None = None
    inference_graph: InferenceGraph | None = None


@dataclass
class InferenceNode:
    event_id: str
    service_id: str
    timestamp: datetime
    severity: Severity
    summary: str
    node_type: str
    in_degree: int = 0
    out_degree: int = 0


@dataclass
class InferenceEdge:
    source_event_id: str
    target_event_id: str
    edge_type: InferenceEdgeType
    plausibility: float
    latency_ms: float
    evidence: str
    requires: list[str] = field(default_factory=list)


@dataclass
class InferenceGraph:
    nodes: dict[str, InferenceNode] = field(default_factory=dict)
    edges: list[InferenceEdge] = field(default_factory=list)
    root_candidates: list[str] = field(default_factory=list)
    leaf_nodes: list[str] = field(default_factory=list)

    def descendants(self, event_id: str) -> set[str]:
        result: set[str] = set()
        stack = [event_id]
        while stack:
            current = stack.pop()
            for edge in self.edges:
                if edge.source_event_id == current and edge.target_event_id not in result:
                    result.add(edge.target_event_id)
                    stack.append(edge.target_event_id)
        return result

    def ancestors(self, event_id: str) -> set[str]:
        result: set[str] = set()
        stack = [event_id]
        while stack:
            current = stack.pop()
            for edge in self.edges:
                if edge.target_event_id == current and edge.source_event_id not in result:
                    result.add(edge.source_event_id)
                    stack.append(edge.source_event_id)
        return result

    def paths_from_origin(self, origin_event_id: str) -> list[list[str]]:
        paths: list[list[str]] = []

        def walk(node: str, path: list[str]) -> None:
            children = [edge.target_event_id for edge in self.edges if edge.source_event_id == node]
            if not children:
                paths.append(path)
                return
            for child in children:
                if child not in path:
                    walk(child, path + [child])

        walk(origin_event_id, [origin_event_id])
        return paths

    def get_edge(self, source: str, target: str) -> InferenceEdge | None:
        return next((edge for edge in self.edges if edge.source_event_id == source and edge.target_event_id == target), None)

    def strongest_path(self, source: str, target: str) -> tuple[list[str], float]:
        candidates = [path for path in self.paths_from_origin(source) if path[-1] == target or target in path]
        best_path: list[str] = []
        best_score = 0.0
        for path in candidates:
            if target in path:
                path = path[: path.index(target) + 1]
            scores = [
                edge.plausibility
                for idx in range(len(path) - 1)
                if (edge := self.get_edge(path[idx], path[idx + 1])) is not None
            ]
            score = min(scores) if scores else 0.0
            if score > best_score:
                best_path, best_score = path, score
        return best_path, best_score

    def origin_impact_score(self, origin_event_id: str) -> float:
        reachable = len(self.descendants(origin_event_id) & set(self.leaf_nodes))
        return reachable / max(len(self.leaf_nodes), 1)

    def assumption_set(self, path: list[str]) -> list[str]:
        assumptions: list[str] = []
        for idx in range(len(path) - 1):
            edge = self.get_edge(path[idx], path[idx + 1])
            if edge:
                assumptions.extend(edge.requires)
        return assumptions


@dataclass
class Signal:
    signal_type: str
    service_id: str | None
    severity: str
    description: str
    evidence_event_ids: list[str]
    metadata: dict[str, Any] = field(default_factory=dict)
    detector: str = ""


@dataclass
class CompositionRule:
    name: str
    requires: list[str]
    cause_type: CauseType
    cause_subtype: str
    title_template: str
    confidence: float
    requires_same_service: bool = False
    requires_temporal_order: bool = False


@dataclass
class Hypothesis:
    hypothesis_id: str
    cause_type: CauseType
    cause_subtype: str
    title: str
    description: str
    root_cause_event_id: str | None
    affected_services: list[str]
    supporting_events: list[str]
    contradicting_events: list[str]
    evidence_chain: list[str]
    suggested_checks: list[str]
    generation_rule: str
    generation_confidence: float


@dataclass
class ScoreBreakdown:
    temporal_alignment: float
    correlation_strength: float
    frequency_weight: float
    dependency_proximity: float
    evidence_coverage: float
    anomaly_severity: float


@dataclass
class ScoredHypothesis:
    hypothesis_id: str
    rank: int
    cause_type: CauseType
    description: str
    total_score: float
    score_breakdown: ScoreBreakdown
    supporting_events: list[str]
    contradicting_events: list[str]
    affected_services: list[str]
    suggested_checks: list[str]
    confidence_label: str
    is_valid: bool
    invalidation_reasons: list[str]


class ContradictionSeverity(str, Enum):
    STRONG = "strong"
    WEAK = "weak"
    INFORMATIONAL = "informational"


@dataclass
class Contradiction:
    hypothesis_id: str
    contradicting_event_id: str
    contradiction_type: str
    explanation: str
    severity: ContradictionSeverity = ContradictionSeverity.WEAK


@dataclass
class TimelineEntry:
    timestamp: datetime
    service_id: str
    severity: Severity
    summary: str
    is_key_event: bool


@dataclass
class ServiceRelation:
    source: str
    target: str
    relation_type: str


@dataclass
class ExplanationRequest:
    incident_id: str
    top_hypotheses: list[ScoredHypothesis]
    timeline: list[TimelineEntry]
    service_topology: list[ServiceRelation]
    runtime_context_summary: dict[str, Any]
    contradictions: list[Contradiction]
    output_format: str


@dataclass
class ExplanationResult:
    incident_id: str
    summary: str
    primary_hypothesis_text: str
    evidence_narrative: str
    timeline_narrative: str
    alternative_explanations: list[str]
    suggested_actions: list[str]
    uncertainty_notes: list[str]
    generation_model: str
    guardrail_violations: list[str]
    explanation_id: str = ""
    hypotheses_hash: str = ""
    events_hash_head: str = ""
    schema_version: int = 1
    quality: str = "ok"


@dataclass(frozen=True)
class IncidentAiTrace:
    trace_id: str
    incident_id: str
    trace_kind: str
    sanitized_system_prompt: str
    sanitized_user_prompt: str
    allowed_fields: tuple[str, ...]
    blocked_fields: tuple[str, ...]
    raw_logs_sent: bool
    schema_version: int
    created_at: str | None = None


@dataclass(frozen=True)
class IncidentChatMessage:
    message_id: str
    incident_id: str
    role: str
    content: str
    schema_version: int
    created_at: str | None = None


@dataclass
class IncidentFeedback:
    incident_id: str
    resolved_at: datetime
    correct_hypothesis_id: str | None
    feedback_type: str
    operator_notes: str = ""


@dataclass
class CalibrationBucket:
    score_lower: float
    score_upper: float
    total_predictions: int
    correct_predictions: int
    accuracy: float
    sample_confidence: str


def _default_calibration_buckets() -> list[CalibrationBucket]:
    return [
        CalibrationBucket(0.0, 0.2, 0, 0, 0.0, "insufficient"),
        CalibrationBucket(0.2, 0.4, 0, 0, 0.0, "insufficient"),
        CalibrationBucket(0.4, 0.6, 0, 0, 0.0, "insufficient"),
        CalibrationBucket(0.6, 0.8, 0, 0, 0.0, "insufficient"),
        CalibrationBucket(0.8, 1.0, 0, 0, 0.0, "insufficient"),
    ]


@dataclass
class CalibrationModel:
    schema_version: int = 1
    buckets: list[CalibrationBucket] = field(default_factory=_default_calibration_buckets)
    last_updated: datetime | None = None
    total_feedback_count: int = 0
    overall_accuracy: float = 0.0


@dataclass
class BaselineMetric:
    metric_name: str
    service_id: str
    buckets: list[float]
    stddev: list[float]
    sample_counts: list[int]
    min_samples_for_confidence: int = 4
    last_updated: datetime | None = None


@dataclass
class AnomalyResult:
    score: float
    confidence: str
    z_score: float
    expected: float = 0.0
    observed: float = 0.0
    std: float = 0.0


def determine_health_state(
    service_id: str,
    anomaly_score: float,
    recent_events: list[NormalizedEvent],
    health_checks: list[NormalizedEvent],
    expected_interval_seconds: float,
    baseline_error_rate: float,
    now: datetime | None = None,
) -> ServiceHealthState:
    del service_id, baseline_error_rate
    now = now or datetime.utcnow()
    if recent_events:
        last_event_age = (now - max(event.timestamp for event in recent_events)).total_seconds()
        if last_event_age > expected_interval_seconds * 3:
            return ServiceHealthState.UNREACHABLE
    else:
        return ServiceHealthState.UNKNOWN
    recent_hc = [event for event in health_checks if (now - event.timestamp).total_seconds() < 120]
    if recent_hc:
        latest_hc = max(recent_hc, key=lambda event: event.timestamp)
        if "fail" in latest_hc.message.lower():
            return ServiceHealthState.FAILING
    error_events = [
        event
        for event in recent_events
        if event.severity >= Severity.ERROR and (now - event.timestamp).total_seconds() < 120
    ]
    if anomaly_score > 0.6 and len(error_events) >= 2:
        return ServiceHealthState.FAILING
    if anomaly_score > 0.3 or len(error_events) >= 1:
        return ServiceHealthState.DEGRADED
    return ServiceHealthState.HEALTHY


@dataclass
class DedupWindow:
    fingerprint: str
    first_event: NormalizedEvent
    last_event: NormalizedEvent
    count: int
    first_seen: datetime
    last_seen: datetime


@dataclass
class DedupSummary:
    fingerprint: str
    first_event_id: str
    last_event: NormalizedEvent
    suppressed_count: int
    window_start: datetime
    window_end: datetime


@dataclass
class CollectorHealth:
    collector_id: str
    is_running: bool
    events_emitted: int
    events_per_second: float
    last_event_at: datetime | None
    error_count: int
    last_error: str | None
    lag_seconds: float | None
    state: CollectorState = CollectorState.STOPPED


@dataclass
class DiskUsage:
    total_gb: float
    used_gb: float
    free_gb: float
    used_percent: float


@dataclass
class HostContext:
    hostname: str
    os_info: str
    cpu_count: int
    total_memory_mb: int
    load_average: tuple[float, float, float]
    cpu_percent: float
    memory_used_percent: float
    disk_usage: dict[str, DiskUsage]
    uptime_seconds: float
    open_file_descriptors: int
    max_file_descriptors: int


@dataclass
class ContainerContext:
    container_id: str
    container_name: str
    image: str
    state: str
    started_at: datetime | None
    restart_count: int
    cpu_percent: float
    memory_usage_mb: float
    memory_limit_mb: float | None
    memory_percent: float
    network_rx_bytes: int
    network_tx_bytes: int
    pids: int
    health_status: str | None
    labels: dict[str, str]
    environment_keys: list[str]
    ports: list[str]


@dataclass
class ServiceContext:
    service_id: str
    containers: list[str]
    event_rate_current: float
    error_rate_current: float
    anomaly_score: float
    last_restart: datetime | None
    restart_count_24h: int
    active_connections: int | None
    dependency_health: dict[str, str]


@dataclass
class TopologyEdge:
    source_service: str
    target_service: str
    relation_type: str
    inferred_from: str


@dataclass
class TopologySnapshot:
    services: list[str]
    edges: list[TopologyEdge]
    isolated_services: list[str]


@dataclass
class ResourceSummary:
    cpu_pressure: str
    memory_pressure: str
    disk_pressure: str
    container_density: int
    resource_contention_score: float


@dataclass
class RuntimeContext:
    captured_at: datetime
    incident_id: str
    host_context: HostContext
    container_contexts: dict[str, ContainerContext]
    service_contexts: dict[str, ServiceContext]
    topology: TopologySnapshot
    resource_summary: ResourceSummary


@dataclass
class FailureTrend:
    period: str
    cause_type_counts: dict[str, int]
    top_subtypes: list[tuple[str, int]]
    top_services: list[tuple[str, int]]


@dataclass
class WeightSnapshot:
    timestamp: datetime
    weights_before: dict[str, float]
    weights_after: dict[str, float]
    trigger_incident_id: str
    reason: str


@dataclass
class WeightState:
    weights: dict[str, float]
    default_weights: dict[str, float]
    update_count: int = 0
    history: list[WeightSnapshot] = field(default_factory=list)


@dataclass
class ValidationResult:
    status: str
    reason: str = ""


@dataclass
class ValidatedHypothesis:
    hypothesis: Hypothesis
    is_valid: bool
    validation_results: list[ValidationResult]
    invalidation_reasons: list[str]
    adjusted_confidence: float


@dataclass(frozen=True)
class IncidentStateLogEntry:
    log_id: int
    incident_id: str
    old_state: str
    new_state: str
    changed_at: datetime
    reason: str


@dataclass
class ResolutionInfo:
    resolved_by: str
    correct_hypothesis_id: str | None
    feedback_type: str
    notes: str = ""
    resolved_at: datetime = field(default_factory=datetime.utcnow)


def window_expired(window: DedupWindow, now: datetime, window_seconds: int) -> bool:
    return window.last_seen < now - timedelta(seconds=window_seconds)
