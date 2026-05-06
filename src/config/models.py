from __future__ import annotations

from dataclasses import field
from pathlib import Path
from typing import Literal

from pydantic import ConfigDict, Field, field_validator
from pydantic.dataclasses import dataclass


_MODEL_CONFIG = ConfigDict(extra="forbid", validate_assignment=True, arbitrary_types_allowed=True)
_SEVERITY = Literal["DEBUG", "INFO", "WARN", "ERROR", "CRITICAL"]


def _config_dataclass(cls: type[object]) -> type[object]:
    return dataclass(cls, config=_MODEL_CONFIG)


@_config_dataclass
class ServerConfig:
    host: str = Field(default="127.0.0.1", min_length=1)
    port: int = Field(default=7433, ge=1, le=65535)
    cors_origins: list[str] = field(default_factory=lambda: ["*"])
    auth_token_env: str = Field(default="", pattern=r"^$|^[A-Za-z_][A-Za-z0-9_]*$")
    require_loopback: bool = True
    expose_prometheus_metrics: bool = False
    rate_limit_chat_tokens_per_minute: float = Field(default=30.0, gt=0)
    rate_limit_explain_tokens_per_minute: float = Field(default=15.0, gt=0)


@_config_dataclass
class StorageConfig:
    data_dir: Path = Path("./data")
    events_db: str = Field(default="events.db", min_length=1, pattern=r"^[^/\\]+\.db$")
    incidents_db: str = Field(default="incidents.db", min_length=1, pattern=r"^[^/\\]+\.db$")
    retention_hours: int = Field(default=72, ge=1)
    batch_size: int = Field(default=100, ge=1)
    flush_interval_ms: int = Field(default=500, ge=1)
    wal_mode: bool = True
    prune_interval_seconds: int = Field(default=60, ge=1)
    enable_mmap: bool = True
    mmap_size_mb: int = Field(default=256, ge=0)

    @field_validator("data_dir", mode="before")
    @classmethod
    def _coerce_data_dir(cls, value: object) -> Path:
        return Path(value) if not isinstance(value, Path) else value


@_config_dataclass
class DockerCollectorConfig:
    enabled: bool = True
    socket: str = Field(default="/var/run/docker.sock", min_length=1)
    include_names: list[str] = field(default_factory=list)
    include_labels: list[str] = field(default_factory=list)
    exclude_names: list[str] = field(default_factory=lambda: ["inferra-*"])
    include_all: bool = True


@_config_dataclass
class JournaldCollectorConfig:
    enabled: bool = True
    units: tuple[str, ...] = ()
    exclude_units: tuple[str, ...] = ("systemd-resolved.service", "systemd-timesyncd.service")
    min_priority: int = Field(default=6, ge=0, le=7)
    poll_interval_seconds: float = Field(default=5.0, gt=0)
    since: str = Field(default="-1 hour", min_length=1)
    limit: int = Field(default=200, ge=1)


@_config_dataclass
class FileCollectorEntry:
    path: str | None = None
    glob: str | None = None
    service_id: str | None = None
    service_id_from_filename: bool = False
    multiline_pattern: str | None = None


@_config_dataclass
class FileCollectorConfig:
    enabled: bool = True
    paths: tuple[str, ...] = ()
    poll_interval_seconds: float = Field(default=1.0, gt=0)
    start_at_end: bool = False
    entries: list[FileCollectorEntry] = field(default_factory=list)


@_config_dataclass
class ProcfsCollectorConfig:
    enabled: bool = True
    poll_interval_seconds: float = Field(default=10.0, gt=0)
    watch_processes: list[str] = field(default_factory=lambda: ["nginx", "postgres", "python", "node", "java"])
    watch_pids: list[int] = field(default_factory=list)
    disk_paths: list[str] = field(default_factory=lambda: ["/", "/var/log", "/data"])
    top_n: int = Field(default=20, ge=1)
    min_cpu_percent: float = Field(default=75.0, ge=0, le=100)
    min_memory_mb: float = Field(default=512.0, ge=0)


@_config_dataclass
class AppCollectorConfig:
    enabled: bool = True
    listen: str = Field(default="127.0.0.1:9876", min_length=1)
    max_payload_bytes: int = Field(default=65536, ge=1)
    shared_token: str = ""
    mount_path: str = Field(default="/api/ingest", min_length=1)
    enable_main_api: bool = True
    enable_standalone: bool = False


@_config_dataclass
class WindowsEventLogCollectorConfig:
    enabled: bool = True
    channels: tuple[str, ...] = ("Application", "System")
    poll_interval_seconds: float = Field(default=5.0, gt=0)


@_config_dataclass
class HostMetricsCollectorConfig:
    enabled: bool = True
    poll_interval_seconds: float = Field(default=10.0, gt=0)
    warn_cpu_percent: float = Field(default=85.0, ge=0, le=100)
    warn_memory_percent: float = Field(default=85.0, ge=0, le=100)
    warn_disk_percent: float = Field(default=90.0, ge=0, le=100)


@_config_dataclass
class WindowsServiceCollectorConfig:
    enabled: bool = True
    poll_interval_seconds: float = Field(default=30.0, gt=0)
    include_stopped: bool = False
    names: tuple[str, ...] = ()


@_config_dataclass
class LinuxSyslogCollectorConfig:
    enabled: bool = True
    paths: tuple[str, ...] = ("/var/log/syslog", "/var/log/messages")
    poll_interval_seconds: float = Field(default=2.0, gt=0)
    start_at_end: bool = True


@_config_dataclass
class KubernetesCollectorConfig:
    enabled: bool = False
    poll_interval_seconds: float = Field(default=15.0, gt=0)
    namespaces: tuple[str, ...] = ()
    all_namespaces: bool = True
    label_selector: str = ""
    limit: int = Field(default=200, ge=1)
    include_pods: bool = True
    include_events: bool = True


@_config_dataclass
class CollectorsConfig:
    auto_start: bool = False
    retry_initial_seconds: float = Field(default=1.0, gt=0)
    retry_max_seconds: float = Field(default=60.0, gt=0)
    docker: DockerCollectorConfig = field(default_factory=DockerCollectorConfig)
    journald: JournaldCollectorConfig = field(default_factory=JournaldCollectorConfig)
    file: FileCollectorConfig = field(default_factory=FileCollectorConfig)
    process: ProcfsCollectorConfig = field(default_factory=ProcfsCollectorConfig)
    app: AppCollectorConfig = field(default_factory=AppCollectorConfig)
    windows_eventlog: WindowsEventLogCollectorConfig = field(default_factory=WindowsEventLogCollectorConfig)
    host_metrics: HostMetricsCollectorConfig = field(default_factory=HostMetricsCollectorConfig)
    windows_service: WindowsServiceCollectorConfig = field(default_factory=WindowsServiceCollectorConfig)
    linux_syslog: LinuxSyslogCollectorConfig = field(default_factory=LinuxSyslogCollectorConfig)
    kubernetes: KubernetesCollectorConfig = field(default_factory=KubernetesCollectorConfig)


@_config_dataclass
class LogFormatConfig:
    name: str = Field(default="", max_length=64)
    pattern: str = ""
    timestamp_field: str = Field(default="timestamp", min_length=1)
    severity_field: str = Field(default="level", min_length=1)
    message_field: str = Field(default="message", min_length=1)


@_config_dataclass
class TagRuleConfig:
    pattern: str = ""
    tags: list[str] = field(default_factory=list)


@_config_dataclass
class ServiceMappingConfig:
    pattern: str = ""
    service_id: str = ""


@_config_dataclass
class NormalizationConfig:
    host_id: str | None = None
    max_message_length: int = Field(default=1024, ge=1)
    max_structured_data_bytes: int = Field(default=32768, ge=0)
    timestamp_future_tolerance_seconds: int = Field(default=60, ge=0)
    fingerprint_hash: Literal["sha256"] = "sha256"
    fingerprint_length: int = Field(default=32, ge=4, le=32)
    log_formats: list[LogFormatConfig] = field(default_factory=list)
    tag_rules: list[TagRuleConfig] = field(default_factory=list)
    service_mappings: list[ServiceMappingConfig] = field(default_factory=list)


@_config_dataclass
class DeduplicationConfig:
    enabled: bool = True
    window_seconds: int = Field(default=60, ge=1)
    max_tracked_fingerprints: int = Field(default=10000, ge=1)
    periodic_summary_interval_seconds: int = Field(default=60, ge=1)
    severity_escalation_splits: bool = True


@_config_dataclass
class NoiseBlocklistConfig:
    pattern: str = ""
    service_id: str | None = None
    severity_max: _SEVERITY = "INFO"
    reason: str = ""


@_config_dataclass
class NoiseAllowlistConfig:
    pattern: str = ""
    severity_min: _SEVERITY = "ERROR"
    tags: list[str] = field(default_factory=list)
    reason: str = ""


@_config_dataclass
class NoiseFilterConfig:
    enabled: bool = True
    blocklist_enabled: bool = True
    allowlist_enabled: bool = True
    adaptive_enabled: bool = True
    frequency_window_minutes: int = Field(default=5, ge=1)
    high_rate_threshold_per_minute: int = Field(default=100, ge=1)
    stability_threshold_cv: float = Field(default=0.2, ge=0)
    routine_sample_target_per_minute: int = Field(default=5, ge=0)
    relevance_scoring_enabled: bool = True
    noise_threshold: float = Field(default=0.7, ge=0, le=1)
    high_volume_events_per_second: int = Field(default=200, ge=1)
    registry_enabled: bool = True
    registry_expiry_days: int = Field(default=14, ge=1)
    blocklist: list[NoiseBlocklistConfig] = field(default_factory=list)
    allowlist: list[NoiseAllowlistConfig] = field(default_factory=list)
    always_keep_severity: _SEVERITY = "ERROR"


@_config_dataclass
class AnomalyWeightsConfig:
    error_rate: float = Field(default=0.35, ge=0, le=1)
    event_volume: float = Field(default=0.20, ge=0, le=1)
    new_fingerprint_rate: float = Field(default=0.20, ge=0, le=1)
    restart_count: float = Field(default=0.15, ge=0, le=1)
    warn_rate: float = Field(default=0.10, ge=0, le=1)


@_config_dataclass
class AnomalyEventScoreWeightsConfig:
    severity: float = Field(default=0.12, ge=0, le=1)
    fingerprint_anomaly: float = Field(default=0.13, ge=0, le=1)
    resource_tag: float = Field(default=0.75, ge=0, le=1)


@_config_dataclass
class AnomalyDetectionConfig:
    enabled: bool = True
    bucket_interval_minutes: int = Field(default=5, ge=1)
    baseline_update_interval_hours: int = Field(default=1, ge=1)
    baseline_alpha: float = Field(default=0.1, gt=0, le=1)
    cold_start_hours: int = Field(default=6, ge=0)
    min_samples_for_confidence: int = Field(default=4, ge=1)
    spike_z_threshold: float = Field(default=3.0, gt=0)
    sustained_lookback_buckets: int = Field(default=6, ge=1)
    absence_sensitivity: float = Field(default=0.1, ge=0, le=1)
    weights: AnomalyWeightsConfig = field(default_factory=AnomalyWeightsConfig)
    event_score_weights: AnomalyEventScoreWeightsConfig = field(default_factory=AnomalyEventScoreWeightsConfig)
    expected_heartbeats: dict[str, list[str]] = field(default_factory=dict)


@_config_dataclass
class CorrelationConfig:
    analysis_interval_seconds: int = Field(default=5, ge=1)
    analysis_window_seconds: int = Field(default=60, ge=1)
    temporal_lookback_seconds: int = Field(default=30, ge=0)
    temporal_lookahead_seconds: int = Field(default=10, ge=0)
    temporal_half_life_seconds: float = Field(default=10.0, gt=0)
    max_hop_distance: int = Field(default=3, ge=1)
    dependency_weight_decay: Literal["inverse", "linear", "none"] = "inverse"
    cooccurrence_bucket_seconds: int = Field(default=5, ge=1)
    cascade_window_seconds: int = Field(default=30, ge=1)
    cluster_min_edge_weight: float = Field(default=0.15, ge=0, le=1)
    cluster_min_events: int = Field(default=2, ge=1)
    merge_on_shared_service_and_time: bool = True


@_config_dataclass
class InferenceGraphStrategiesConfig:
    dependency_propagation: bool = True
    same_service_escalation: bool = True
    resource_preceded_error: bool = True
    config_preceded_error: bool = True
    restart_preceded_disconnection: bool = True
    shared_fate: bool = True
    timeout_chain: bool = True


@_config_dataclass
class InferenceGraphConfig:
    budget_ms: int = Field(default=100, ge=1)
    max_events_for_graph: int = Field(default=500, ge=1)
    plausibility_threshold: float = Field(default=0.15, ge=0, le=1)
    max_edges_per_node: int = Field(default=10, ge=1)
    strategies: InferenceGraphStrategiesConfig = field(default_factory=InferenceGraphStrategiesConfig)


@_config_dataclass
class CustomHypothesisRuleConfig:
    name: str = ""
    requires: list[str] = field(default_factory=list)
    requires_same_service: bool = False
    requires_temporal_order: bool = False
    cause_type: str = Field(default="unknown", min_length=1)
    cause_subtype: str = Field(default="anomaly_detected", min_length=1)
    title_template: str = ""
    confidence: float = Field(default=0.5, ge=0, le=1)


@_config_dataclass
class HypothesisEngineConfig:
    max_hypotheses_per_incident: int = Field(default=50, ge=1)
    min_supporting_events: int = Field(default=1, ge=1)
    min_generation_confidence: float = Field(default=0.1, ge=0, le=1)
    dedup_overlap_threshold: float = Field(default=0.5, ge=0, le=1)
    custom_rules: list[CustomHypothesisRuleConfig] = field(default_factory=list)


@_config_dataclass
class HypothesisValidationConfig:
    enabled: bool = True
    temporal_consistency_threshold: float = Field(default=0.5, ge=0, le=1)
    temporal_consistency_warn: float = Field(default=0.2, ge=0, le=1)
    contradiction_ratio_fail: float = Field(default=0.6, ge=0, le=1)
    contradiction_ratio_warn: float = Field(default=0.3, ge=0, le=1)
    min_root_cause_severity: _SEVERITY = "WARN"
    confidence_reduction_per_warning: float = Field(default=0.2, ge=0, le=1)


@_config_dataclass
class ScoringTuningConfig:
    enabled: bool = True
    learning_rate: float = Field(default=0.05, gt=0, le=1)
    max_drift_from_default: float = Field(default=0.5, ge=0, le=1)
    min_weight: float = Field(default=0.03, ge=0, le=1)
    tiebreak_order: list[str] = field(
        default_factory=lambda: ["evidence_coverage", "contradicting_events_asc", "root_cause_timestamp_asc"]
    )


@_config_dataclass
class ScoringConfig:
    temporal_alignment: float = Field(default=0.25, ge=0, le=1)
    correlation_strength: float = Field(default=0.20, ge=0, le=1)
    frequency_weight: float = Field(default=0.15, ge=0, le=1)
    dependency_proximity: float = Field(default=0.15, ge=0, le=1)
    evidence_coverage: float = Field(default=0.15, ge=0, le=1)
    anomaly_severity: float = Field(default=0.10, ge=0, le=1)
    tuning: ScoringTuningConfig = field(default_factory=ScoringTuningConfig)


@_config_dataclass
class CalibrationDefaultsConfig:
    high_threshold: float = Field(default=0.75, ge=0, le=1)
    medium_threshold: float = Field(default=0.40, ge=0, le=1)


@_config_dataclass
class CalibrationConfig:
    enabled: bool = True
    min_samples_per_bucket: int = Field(default=10, ge=1)
    bucket_count: int = Field(default=5, ge=1)
    staleness_threshold_days: int = Field(default=30, ge=1)
    persistence_file: Path = Path("./data/calibration.json")
    defaults: CalibrationDefaultsConfig = field(default_factory=CalibrationDefaultsConfig)

    @field_validator("persistence_file", mode="before")
    @classmethod
    def _coerce_persistence_file(cls, value: object) -> Path:
        return Path(value) if not isinstance(value, Path) else value


@_config_dataclass
class ContradictionRulesConfig:
    timeline_violation: bool = True
    health_check: bool = True
    resource_state: bool = True
    scope_mismatch: bool = True
    mutual_exclusion: bool = True


@_config_dataclass
class ContradictionHandlingConfig:
    enabled: bool = True
    timeline_tolerance_seconds: float = Field(default=5.0, ge=0)
    strong_penalty_per_contradiction: float = Field(default=0.15, ge=0, le=1)
    weak_penalty_per_contradiction: float = Field(default=0.05, ge=0, le=1)
    min_penalty_multiplier: float = Field(default=0.5, ge=0, le=1)
    rules: ContradictionRulesConfig = field(default_factory=ContradictionRulesConfig)


@_config_dataclass
class ExplanationSanitizationConfig:
    strip_ips: bool = True
    strip_env_values: bool = True
    strip_paths: bool = True
    keep_service_names: bool = True
    keep_timestamps: bool = True


@_config_dataclass
class ExplanationGuardrailsConfig:
    verify_service_names: bool = True
    verify_timestamps: bool = True
    verify_causal_claims: bool = True
    check_overconfidence: bool = True


@_config_dataclass
class ExplanationConfig:
    provider: Literal["template", "ollama"] = "template"
    fallback: str = Field(default="template_fallback", min_length=1)
    timeout_seconds: float = Field(default=30.0, gt=0)
    max_retries: int = Field(default=2, ge=0)
    temperature: float = Field(default=0.2, ge=0, le=2)
    max_tokens: int = Field(default=2048, ge=1)
    cache_enabled: bool = True
    sanitization: ExplanationSanitizationConfig = field(default_factory=ExplanationSanitizationConfig)
    guardrails: ExplanationGuardrailsConfig = field(default_factory=ExplanationGuardrailsConfig)


@_config_dataclass
class IncidentLifecycleLimitsConfig:
    max_events_per_incident: int = Field(default=10000, ge=1)
    max_active_incidents: int = Field(default=200, ge=1)
    max_clusters_per_incident: int = Field(default=20, ge=1)


@_config_dataclass
class IncidentLifecycleConfig:
    stale_timeout_seconds: int = Field(default=900, ge=1)
    merge_time_threshold_seconds: int = Field(default=300, ge=1)
    archive_after_days: int = Field(default=7, ge=1)
    enable_auto_split: bool = True
    staleness_check_interval_seconds: int = Field(default=60, ge=1)
    limits: IncidentLifecycleLimitsConfig = field(default_factory=IncidentLifecycleLimitsConfig)


@_config_dataclass
class TopologyEdgeConfig:
    source: str = Field(default="", max_length=256)
    target: str = Field(default="", max_length=256)
    type: str = Field(default="depends_on", min_length=1, max_length=64)


@_config_dataclass
class TopologyConfig:
    edges: list[TopologyEdgeConfig] = field(default_factory=list)


@_config_dataclass
class LoggingConfig:
    level: Literal["DEBUG", "INFO", "WARN", "WARNING", "ERROR", "CRITICAL"] = "INFO"
    module_levels: dict[str, str] = field(default_factory=dict)


@_config_dataclass
class AIConfig:
    enabled: bool = False
    provider: Literal["ollama"] = "ollama"
    base_url: str = Field(default="http://127.0.0.1:11434", pattern=r"^https?://.+")
    model: str = Field(default="gemma4:e4b", min_length=1, pattern=r"^[A-Za-z0-9][A-Za-z0-9._-]*(?::[A-Za-z0-9._-]+)?$")
    token_env: str = Field(default="", pattern=r"^$|^[A-Za-z_][A-Za-z0-9_]*$")
    allow_remote: bool = False
    temperature: float = Field(default=1.0, ge=0, le=2)
    top_p: float = Field(default=0.95, ge=0, le=1)
    top_k: int = Field(default=64, ge=0)
    timeout_seconds: float = Field(default=30.0, gt=0)
    connect_timeout_seconds: float = Field(default=5.0, gt=0)
    read_timeout_seconds: float = Field(default=120.0, gt=0)
    max_retries: int = Field(default=2, ge=0)
    max_tokens: int = Field(default=2048, ge=1)
    stream: bool = True
    cache_enabled: bool = True
    cache_ttl_seconds: int = Field(default=3600, ge=0)
    max_context_events: int = Field(default=30, ge=1)
    redact_raw_logs: bool = True
    nl_search_min_confidence: float = Field(default=0.55, ge=0.0, le=1.0)


@_config_dataclass
class InferraConfig:
    server: ServerConfig = field(default_factory=ServerConfig)
    storage: StorageConfig = field(default_factory=StorageConfig)
    collectors: CollectorsConfig = field(default_factory=CollectorsConfig)
    normalization: NormalizationConfig = field(default_factory=NormalizationConfig)
    deduplication: DeduplicationConfig = field(default_factory=DeduplicationConfig)
    noise_filter: NoiseFilterConfig = field(default_factory=NoiseFilterConfig)
    anomaly_detection: AnomalyDetectionConfig = field(default_factory=AnomalyDetectionConfig)
    correlation: CorrelationConfig = field(default_factory=CorrelationConfig)
    inference_graph: InferenceGraphConfig = field(default_factory=InferenceGraphConfig)
    hypothesis_engine: HypothesisEngineConfig = field(default_factory=HypothesisEngineConfig)
    hypothesis_validation: HypothesisValidationConfig = field(default_factory=HypothesisValidationConfig)
    scoring: ScoringConfig = field(default_factory=ScoringConfig)
    calibration: CalibrationConfig = field(default_factory=CalibrationConfig)
    contradiction_handling: ContradictionHandlingConfig = field(default_factory=ContradictionHandlingConfig)
    explanation: ExplanationConfig = field(default_factory=ExplanationConfig)
    incident_lifecycle: IncidentLifecycleConfig = field(default_factory=IncidentLifecycleConfig)
    topology: TopologyConfig = field(default_factory=TopologyConfig)
    logging: LoggingConfig = field(default_factory=LoggingConfig)
    ai: AIConfig = field(default_factory=AIConfig)
