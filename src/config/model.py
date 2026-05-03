from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path


@dataclass(frozen=True)
class ServerConfig:
    host: str = "127.0.0.1"
    port: int = 7433


@dataclass(frozen=True)
class StorageConfig:
    data_dir: Path = Path("./data")
    retention_hours: int = 72


@dataclass(frozen=True)
class AIConfig:
    enabled: bool = False
    provider: str = "ollama"
    base_url: str = "http://127.0.0.1:11434"
    model: str = "gemma4:e4b"
    token_env: str = ""
    timeout_seconds: float = 30.0
    temperature: float = 1.0
    top_p: float = 0.95
    top_k: int = 64
    max_context_events: int = 30
    redact_raw_logs: bool = True
    allow_remote: bool = False


@dataclass(frozen=True)
class FileCollectorConfig:
    enabled: bool = True
    paths: tuple[str, ...] = ()
    poll_interval_seconds: float = 1.0
    start_at_end: bool = False


@dataclass(frozen=True)
class AppCollectorConfig:
    enabled: bool = True
    max_payload_bytes: int = 65536


@dataclass(frozen=True)
class WindowsEventLogCollectorConfig:
    enabled: bool = True
    channels: tuple[str, ...] = ("Application", "System")
    poll_interval_seconds: float = 5.0


@dataclass(frozen=True)
class HostMetricsCollectorConfig:
    enabled: bool = True
    poll_interval_seconds: float = 10.0
    warn_cpu_percent: float = 85.0
    warn_memory_percent: float = 85.0
    warn_disk_percent: float = 90.0


@dataclass(frozen=True)
class WindowsServiceCollectorConfig:
    enabled: bool = True
    poll_interval_seconds: float = 30.0
    include_stopped: bool = False
    names: tuple[str, ...] = ()


@dataclass(frozen=True)
class LinuxSyslogCollectorConfig:
    enabled: bool = True
    paths: tuple[str, ...] = ("/var/log/syslog", "/var/log/messages")
    poll_interval_seconds: float = 2.0
    start_at_end: bool = True


@dataclass(frozen=True)
class JournaldCollectorConfig:
    enabled: bool = True
    poll_interval_seconds: float = 5.0
    units: tuple[str, ...] = ()
    since: str = "-1 hour"
    limit: int = 200


@dataclass(frozen=True)
class KubernetesCollectorConfig:
    enabled: bool = False
    poll_interval_seconds: float = 15.0
    namespaces: tuple[str, ...] = ()
    all_namespaces: bool = True
    limit: int = 200
    include_pods: bool = True
    include_events: bool = True


@dataclass(frozen=True)
class ProcessCollectorConfig:
    enabled: bool = True
    poll_interval_seconds: float = 15.0
    top_n: int = 20
    min_cpu_percent: float = 75.0
    min_memory_mb: float = 512.0


@dataclass(frozen=True)
class CollectorsConfig:
    auto_start: bool = False
    retry_initial_seconds: float = 1.0
    retry_max_seconds: float = 60.0
    file: FileCollectorConfig = field(default_factory=FileCollectorConfig)
    app: AppCollectorConfig = field(default_factory=AppCollectorConfig)
    windows_eventlog: WindowsEventLogCollectorConfig = field(default_factory=WindowsEventLogCollectorConfig)
    host_metrics: HostMetricsCollectorConfig = field(default_factory=HostMetricsCollectorConfig)
    windows_service: WindowsServiceCollectorConfig = field(default_factory=WindowsServiceCollectorConfig)
    linux_syslog: LinuxSyslogCollectorConfig = field(default_factory=LinuxSyslogCollectorConfig)
    journald: JournaldCollectorConfig = field(default_factory=JournaldCollectorConfig)
    kubernetes: KubernetesCollectorConfig = field(default_factory=KubernetesCollectorConfig)
    process: ProcessCollectorConfig = field(default_factory=ProcessCollectorConfig)


@dataclass(frozen=True)
class NormalizationConfig:
    max_message_length: int = 1024
    max_structured_data_bytes: int = 32768
    timestamp_future_tolerance_seconds: int = 60
    fingerprint_length: int = 32


@dataclass(frozen=True)
class DeduplicationConfig:
    enabled: bool = True
    window_seconds: int = 60
    max_tracked_fingerprints: int = 10000


@dataclass(frozen=True)
class NoiseFilterConfig:
    enabled: bool = True
    always_keep_severity: str = "ERROR"


@dataclass(frozen=True)
class InferraConfig:
    server: ServerConfig = field(default_factory=ServerConfig)
    storage: StorageConfig = field(default_factory=StorageConfig)
    ai: AIConfig = field(default_factory=AIConfig)
    collectors: CollectorsConfig = field(default_factory=CollectorsConfig)
    normalization: NormalizationConfig = field(default_factory=NormalizationConfig)
    deduplication: DeduplicationConfig = field(default_factory=DeduplicationConfig)
    noise_filter: NoiseFilterConfig = field(default_factory=NoiseFilterConfig)
