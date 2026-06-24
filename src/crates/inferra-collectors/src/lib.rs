//! Native collector runtime and status tracking.

use anyhow::{Context, Result};
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use inferra_core::reconcile_new_events;
use inferra_storage::{
    EventsStore, GovernanceRule, IngestBatchResult, IngestGovernance, NewEventRecord,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::{Disks, ProcessesToUpdate, System};
use time::OffsetDateTime;
use tokio::sync::{watch, RwLock};
use tokio::task::JoinHandle;
use toml::Value as TomlValue;

mod otlp_logs;

pub fn normalize_otlp_logs_protobuf_request(payload: &[u8]) -> Result<serde_json::Value> {
    otlp_logs::normalize_otlp_logs_protobuf_request(payload)
}

static NEXT_EVENT_ID: AtomicU64 = AtomicU64::new(1);
static GLOBAL_QUEUE_DEPTH: AtomicI64 = AtomicI64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorHealth {
    pub collector_id: String,
    pub status: String,
    pub source_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CollectorRuntimeRow {
    pub collector_id: String,
    pub status: String,
    pub source_type: String,
    pub is_running: bool,
    pub events_emitted: u64,
    pub events_per_second: f64,
    pub last_event_at: Option<String>,
    pub error_count: u64,
    pub dropped_events: u64,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
    pub lag_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppIngestResult {
    pub event_id: String,
    pub accepted: bool,
    pub suppressed_duplicates: u64,
    pub suppressed_noise: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtlpLogsIngestResult {
    pub inserted: u64,
    pub rejected_log_records: u64,
    pub suppressed_duplicates: u64,
    pub suppressed_noise: u64,
}

#[derive(Clone, Default)]
pub struct CollectorRuntime {
    inner: Arc<CollectorRuntimeInner>,
}

#[derive(Default)]
struct CollectorRuntimeInner {
    statuses: RwLock<HashMap<String, CollectorRuntimeRow>>,
    stop_sender: Mutex<Option<watch::Sender<bool>>>,
    handles: Mutex<Vec<JoinHandle<()>>>,
    rate_state: Mutex<HashMap<String, CollectorRateState>>,
}

#[derive(Debug, Clone, Copy)]
struct CollectorRateState {
    last_events_emitted: u64,
    last_observed_at_ms: i128,
}

#[derive(Debug, Clone)]
enum CollectorSpec {
    HostMetrics {
        poll_interval: Duration,
        warn_cpu_percent: f32,
        warn_memory_percent: f32,
        warn_disk_percent: f32,
    },
    Process {
        poll_interval: Duration,
        top_n: usize,
        min_cpu_percent: f32,
        min_memory_mb: f64,
        watch_processes: HashSet<String>,
        watch_pids: HashSet<u32>,
    },
    LinuxSyslog {
        poll_interval: Duration,
        paths: Vec<PathBuf>,
        start_at_end: bool,
    },
    File {
        poll_interval: Duration,
        start_at_end: bool,
        targets: Vec<FileTailTarget>,
    },
    Journald {
        poll_interval: Duration,
        units: Vec<String>,
        exclude_units: Vec<String>,
        min_priority: i64,
        since: String,
        limit: usize,
    },
    WindowsEventLog {
        poll_interval: Duration,
        channels: Vec<String>,
    },
    WindowsService {
        poll_interval: Duration,
        include_stopped: bool,
        include_automatic_stopped: bool,
        names: HashSet<String>,
        exclude_names: HashSet<String>,
    },
    Docker {
        poll_interval: Duration,
        socket: String,
        include_names: HashSet<String>,
        include_labels: HashSet<String>,
        exclude_names: HashSet<String>,
        include_all: bool,
    },
    Kubernetes {
        poll_interval: Duration,
        namespaces: Vec<String>,
        all_namespaces: bool,
        label_selector: String,
        limit: usize,
        include_pods: bool,
        include_events: bool,
    },
    AppIngest,
    AppStandalone {
        listen: String,
        mount_path: String,
        shared_token: String,
        max_payload_bytes: usize,
    },
}

#[derive(Debug, Clone)]
struct FileTailTarget {
    path: PathBuf,
    service_id: Option<String>,
    service_id_from_filename: bool,
    multiline_pattern: Option<String>,
}

#[derive(Clone)]
struct AppStandaloneState {
    runtime: CollectorRuntime,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    shared_token: String,
}

struct CollectorTaskContext<'a> {
    runtime: &'a CollectorRuntime,
    events_db: &'a Path,
    incidents_db: &'a Path,
    config: &'a TomlValue,
    collector_id: &'a str,
    source_type: &'a str,
}

#[derive(Debug, Clone, Copy)]
enum CollectorEventKind {
    Log,
    StateChange,
    Metric,
}

impl CollectorEventKind {
    fn event_type(self) -> i64 {
        match self {
            Self::Log => 0,
            Self::StateChange => 1,
            Self::Metric => 2,
        }
    }
}

struct CollectorEventArgs {
    id_prefix: &'static str,
    timestamp: Option<String>,
    service_id: String,
    severity: i64,
    message: String,
    source_type: String,
    source_id: String,
    tags: Vec<String>,
    fingerprint: Option<String>,
    host_id: Option<String>,
    kind: CollectorEventKind,
    quality: &'static str,
    structured_data: Option<serde_json::Value>,
    raw_offset: Option<i64>,
    trace_id: Option<String>,
    span_id: Option<String>,
    signal_kind: &'static str,
    deployment_environment: Option<String>,
    severity_text: Option<String>,
}

fn collector_event(args: CollectorEventArgs) -> NewEventRecord {
    let timestamp = args.timestamp.unwrap_or_else(now_iso);
    let fingerprint = args.fingerprint.unwrap_or_else(|| {
        semantic_fingerprint(
            args.id_prefix,
            &args.service_id,
            &args.source_type,
            &args.message,
            args.trace_id.as_deref(),
        )
    });
    NewEventRecord {
        event_id: next_event_id(args.id_prefix),
        timestamp: timestamp.clone(),
        service_id: args.service_id,
        severity: args.severity,
        message: args.message,
        source_type: args.source_type,
        source_id: args.source_id,
        tags: args.tags,
        fingerprint,
        host_id: args.host_id.unwrap_or_else(host_name),
        event_type: args.kind.event_type(),
        timestamp_source: "collector".into(),
        collected_at: now_iso(),
        quality: Some(args.quality.into()),
        structured_data: args.structured_data,
        raw_offset: args.raw_offset,
        trace_id: args.trace_id,
        span_id: args.span_id,
        signal_kind: args.signal_kind.into(),
        deployment_environment: args.deployment_environment,
        severity_text: args.severity_text,
    }
}

fn host_name() -> String {
    System::host_name().unwrap_or_else(|| "local".into())
}

fn structured_with_attributes(
    attributes: serde_json::Map<String, serde_json::Value>,
    section: &str,
    payload: serde_json::Value,
) -> serde_json::Value {
    let mut root = serde_json::Map::new();
    root.insert("attributes".into(), serde_json::Value::Object(attributes));
    root.insert(section.into(), payload);
    serde_json::Value::Object(root)
}

fn insert_attr(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: serde_json::Value,
) {
    if !key.trim().is_empty() && !value.is_null() {
        attributes.insert(key.to_string(), value);
    }
}

fn insert_scalar_json_attr(
    attributes: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<&serde_json::Value>,
) {
    let Some(value) = value else {
        return;
    };
    match value {
        serde_json::Value::String(s) if !s.is_empty() => {
            insert_attr(attributes, key, serde_json::Value::String(s.clone()));
        }
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            insert_attr(attributes, key, value.clone());
        }
        _ => {}
    }
}

#[derive(Debug, Clone)]
struct ParsedCollectorLog {
    timestamp: Option<String>,
    service_id: Option<String>,
    severity: i64,
    severity_text: Option<String>,
    message: String,
    attributes: serde_json::Map<String, serde_json::Value>,
    parsed_json: Option<serde_json::Value>,
    trace_id: Option<String>,
    span_id: Option<String>,
    deployment_environment: Option<String>,
}

fn parse_collector_log_line(raw: &str) -> ParsedCollectorLog {
    let trimmed = raw.trim();
    let mut parsed = ParsedCollectorLog {
        timestamp: None,
        service_id: None,
        severity: severity_from_text(trimmed),
        severity_text: None,
        message: trimmed.to_string(),
        attributes: serde_json::Map::new(),
        parsed_json: None,
        trace_id: None,
        span_id: None,
        deployment_environment: None,
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return parsed;
    };
    let Some(object) = value.as_object() else {
        return parsed;
    };

    parsed.timestamp = ["timestamp", "time", "@timestamp", "ts"]
        .iter()
        .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
        .map(str::to_string);
    parsed.service_id = ["service", "service_id", "service.name", "logger", "target"]
        .iter()
        .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    parsed.message = ["message", "msg", "body", "event"]
        .iter()
        .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
        .unwrap_or(trimmed)
        .to_string();
    parsed.severity_text = ["level", "severity", "severity_text"]
        .iter()
        .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
        .map(str::to_string);
    parsed.severity = parsed
        .severity_text
        .as_deref()
        .map(severity_from_level)
        .unwrap_or_else(|| severity_from_text(&parsed.message));
    parsed.trace_id = object
        .get("trace_id")
        .and_then(normalize_hex_trace_id_from_value);
    parsed.span_id = object
        .get("span_id")
        .and_then(normalize_hex_span_id_from_value);
    if let Some((trace_id, span_id)) = object
        .get("traceparent")
        .and_then(serde_json::Value::as_str)
        .and_then(parse_w3c_traceparent)
    {
        parsed.trace_id.get_or_insert(trace_id);
        parsed.span_id.get_or_insert(span_id);
    }
    parsed.deployment_environment = object
        .get("deployment_environment")
        .or_else(|| object.get("environment"))
        .or_else(|| object.get("deployment.environment"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);

    for key in [
        "http.status_code",
        "http.route",
        "exception.type",
        "deployment.environment",
        "service.name",
    ] {
        insert_scalar_json_attr(&mut parsed.attributes, key, object.get(key));
    }
    if let Some(attrs) = object
        .get("attributes")
        .and_then(serde_json::Value::as_object)
    {
        for (key, value) in attrs {
            insert_scalar_json_attr(&mut parsed.attributes, key, Some(value));
        }
    }
    parsed.parsed_json = Some(value);
    parsed
}

impl CollectorRuntime {
    pub async fn start(
        &self,
        config: &TomlValue,
        events_db: PathBuf,
        incidents_db: PathBuf,
    ) -> Result<()> {
        self.stop().await?;
        let (stop_tx, stop_rx) = watch::channel(false);
        let specs = collector_specs(config);
        {
            let mut guard = self
                .inner
                .stop_sender
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *guard = Some(stop_tx);
        }

        let mut handles = Vec::new();
        for spec in specs {
            match spec {
                CollectorSpec::HostMetrics {
                    poll_interval,
                    warn_cpu_percent,
                    warn_memory_percent,
                    warn_disk_percent,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "host_metrics".into(),
                        status: "running".into(),
                        source_type: "host".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_host_metrics(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            poll_interval,
                            warn_cpu_percent,
                            warn_memory_percent,
                            warn_disk_percent,
                        )
                        .await;
                    }));
                }
                CollectorSpec::Process {
                    poll_interval,
                    top_n,
                    min_cpu_percent,
                    min_memory_mb,
                    watch_processes,
                    watch_pids,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "process".into(),
                        status: "running".into(),
                        source_type: "process_snapshot".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_process_snapshot(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            poll_interval,
                            top_n,
                            min_cpu_percent,
                            min_memory_mb,
                            watch_processes,
                            watch_pids,
                        )
                        .await;
                    }));
                }
                CollectorSpec::LinuxSyslog {
                    poll_interval,
                    paths,
                    start_at_end,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "linux_syslog".into(),
                        status: "running".into(),
                        source_type: "syslog".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_linux_syslog(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            poll_interval,
                            paths,
                            start_at_end,
                        )
                        .await;
                    }));
                }
                CollectorSpec::File {
                    poll_interval,
                    start_at_end,
                    targets,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "file".into(),
                        status: "running".into(),
                        source_type: "file".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_file_tail(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            poll_interval,
                            start_at_end,
                            targets,
                        )
                        .await;
                    }));
                }
                CollectorSpec::Journald {
                    poll_interval,
                    units,
                    exclude_units,
                    min_priority,
                    since,
                    limit,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "journald".into(),
                        status: "running".into(),
                        source_type: "journald".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_journald(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            poll_interval,
                            units,
                            exclude_units,
                            min_priority,
                            since,
                            limit,
                        )
                        .await;
                    }));
                }
                CollectorSpec::WindowsEventLog {
                    poll_interval,
                    channels,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "windows_eventlog".into(),
                        status: "running".into(),
                        source_type: "eventlog".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_windows_eventlog(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            poll_interval,
                            channels,
                        )
                        .await;
                    }));
                }
                CollectorSpec::WindowsService {
                    poll_interval,
                    include_stopped,
                    include_automatic_stopped,
                    names,
                    exclude_names,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "windows_service".into(),
                        status: "running".into(),
                        source_type: "service".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_windows_service(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            poll_interval,
                            include_stopped,
                            include_automatic_stopped,
                            names,
                            exclude_names,
                        )
                        .await;
                    }));
                }
                CollectorSpec::Docker {
                    poll_interval,
                    socket,
                    include_names,
                    include_labels,
                    exclude_names,
                    include_all,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "docker".into(),
                        status: "running".into(),
                        source_type: "container".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_docker(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            poll_interval,
                            socket,
                            include_names,
                            include_labels,
                            exclude_names,
                            include_all,
                        )
                        .await;
                    }));
                }
                CollectorSpec::Kubernetes {
                    poll_interval,
                    namespaces,
                    all_namespaces,
                    label_selector,
                    limit,
                    include_pods,
                    include_events,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "kubernetes".into(),
                        status: "running".into(),
                        source_type: "kubernetes".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_kubernetes(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            poll_interval,
                            namespaces,
                            all_namespaces,
                            label_selector,
                            limit,
                            include_pods,
                            include_events,
                        )
                        .await;
                    }));
                }
                CollectorSpec::AppIngest => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "app".into(),
                        status: "running".into(),
                        source_type: "app_http".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                }
                CollectorSpec::AppStandalone {
                    listen,
                    mount_path,
                    shared_token,
                    max_payload_bytes,
                } => {
                    self.upsert_status(CollectorRuntimeRow {
                        collector_id: "app".into(),
                        status: "running".into(),
                        source_type: "app_http".into(),
                        is_running: true,
                        ..Default::default()
                    })
                    .await;
                    let runtime = self.clone();
                    let rx = stop_rx.clone();
                    let events_db = events_db.clone();
                    let incidents_db = incidents_db.clone();
                    let config = config.clone();
                    handles.push(tokio::spawn(async move {
                        run_app_standalone(
                            runtime,
                            rx,
                            events_db.clone(),
                            incidents_db.clone(),
                            config,
                            listen,
                            mount_path,
                            shared_token,
                            max_payload_bytes,
                        )
                        .await;
                    }));
                }
            }
        }
        let mut guard = self
            .inner
            .handles
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = handles;
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        if let Some(sender) = self
            .inner
            .stop_sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = sender.send(true);
        }
        let handles = {
            let mut guard = self
                .inner
                .handles
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *guard)
        };
        for handle in handles {
            let _ = handle.await;
        }
        let mut statuses = self.inner.statuses.write().await;
        for row in statuses.values_mut() {
            row.is_running = false;
            if row.status == "running" {
                row.status = "stopped".into();
            }
        }
        GLOBAL_QUEUE_DEPTH.store(0, Ordering::Relaxed);
        Ok(())
    }

    pub async fn collector_rows(&self, config: &TomlValue) -> Vec<CollectorRuntimeRow> {
        let configured = configured_collectors(config);
        let statuses = self.inner.statuses.read().await.clone();
        let mut rows = Vec::new();
        for item in configured {
            if let Some(runtime) = statuses.get(&item.collector_id) {
                rows.push(runtime.clone());
            } else {
                rows.push(CollectorRuntimeRow {
                    collector_id: item.collector_id,
                    status: item.status,
                    source_type: item.source_type,
                    is_running: false,
                    ..Default::default()
                });
            }
        }
        rows.sort_by(|a, b| a.collector_id.cmp(&b.collector_id));
        rows
    }

    pub fn queue_depth(&self) -> i64 {
        GLOBAL_QUEUE_DEPTH.load(Ordering::Relaxed)
    }

    pub async fn total_errors(&self) -> i64 {
        self.inner
            .statuses
            .read()
            .await
            .values()
            .map(|row| row.error_count as i64)
            .sum()
    }

    pub async fn active_error_count(&self) -> i64 {
        self.inner
            .statuses
            .read()
            .await
            .values()
            .filter(|row| row.status == "error")
            .map(|row| row.error_count as i64)
            .sum()
    }

    pub async fn ingest_app_event(
        &self,
        events_db: &Path,
        incidents_db: &Path,
        config: &TomlValue,
        payload: &serde_json::Value,
    ) -> Result<AppIngestResult> {
        let timestamp = payload
            .get("timestamp")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(now_iso);
        let service_id = payload
            .get("service")
            .or_else(|| payload.get("service_id"))
            .and_then(|value| value.as_str())
            .unwrap_or("app")
            .to_string();
        let message = payload
            .get("message")
            .or_else(|| payload.get("body"))
            .and_then(|value| value.as_str())
            .unwrap_or("ingested application event")
            .to_string();
        let level = payload
            .get("level")
            .or_else(|| payload.get("severity_text"))
            .and_then(|value| value.as_str())
            .unwrap_or("info");
        let source_id = payload
            .get("source_id")
            .and_then(|value| value.as_str())
            .unwrap_or("app-ingest")
            .to_string();
        let tags = payload
            .get("tags")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let explicit_trace_id = payload
            .get("trace_id")
            .and_then(normalize_hex_trace_id_from_value);
        let explicit_span_id = payload
            .get("span_id")
            .and_then(normalize_hex_span_id_from_value);
        let payload_trace_id = payload_attribute_value(payload, "inferra.trace_id")
            .and_then(normalize_hex_trace_id_from_value);
        let payload_span_id = payload_attribute_value(payload, "inferra.span_id")
            .and_then(normalize_hex_span_id_from_value);
        let traceparent = payload_traceparent(payload).and_then(parse_w3c_traceparent);
        let trace_id = explicit_trace_id
            .clone()
            .or(payload_trace_id.clone())
            .or_else(|| traceparent.as_ref().map(|(trace_id, _)| trace_id.clone()));
        let span_id = explicit_span_id
            .clone()
            .or(payload_span_id.clone())
            .or_else(|| match (&trace_id, traceparent.as_ref()) {
                (Some(trace_id), Some((tp_trace_id, tp_span_id))) if trace_id == tp_trace_id => {
                    Some(tp_span_id.clone())
                }
                (None, Some((_, tp_span_id))) => Some(tp_span_id.clone()),
                _ => None,
            });
        let signal_kind = payload
            .get("signal_kind")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("log")
            .to_string();
        let deployment_environment = payload
            .get("deployment_environment")
            .or_else(|| payload.get("environment"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| {
                payload
                    .get("attributes")
                    .and_then(|attrs| attrs.get("deployment.environment"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            });
        let severity_text = payload
            .get("severity_text")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or_else(|| {
                payload
                    .get("level")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            });
        let event_id = next_event_id("app");
        let fingerprint = semantic_fingerprint(
            "app",
            &service_id,
            "app_http",
            &message,
            trace_id.as_deref(),
        );
        adjust_queue_depth(1);
        let result = (|| {
            let mut store =
                EventsStore::open(events_db)?.context("event store not found for app ingest")?;
            store.insert_batch_governed(
                &[NewEventRecord {
                    event_id: event_id.clone(),
                    timestamp: timestamp.clone(),
                    service_id,
                    severity: severity_from_level(level),
                    message,
                    source_type: "app_http".into(),
                    source_id,
                    tags,
                    fingerprint,
                    host_id: "local".into(),
                    event_type: 0,
                    timestamp_source: "collector".into(),
                    collected_at: timestamp.clone(),
                    quality: Some("normalized".into()),
                    structured_data: Some(payload.clone()),
                    raw_offset: None,
                    trace_id,
                    span_id,
                    signal_kind,
                    deployment_environment,
                    severity_text,
                }],
                &ingest_governance(config),
            )
        })();
        adjust_queue_depth(-1);
        let result = result?;
        let accepted = result.inserted > 0;
        if result.inserted > 0 {
            reconcile_new_events(events_db, incidents_db, config, &result.inserted_event_ids)?;
            self.bump_success(
                "app",
                "app_http",
                result.inserted as u64,
                (result.suppressed_duplicates + result.suppressed_noise) as u64,
                Some(timestamp),
            )
            .await;
        }
        Ok(AppIngestResult {
            event_id,
            accepted,
            suppressed_duplicates: result.suppressed_duplicates as u64,
            suppressed_noise: result.suppressed_noise as u64,
        })
    }

    /// Ingest normalized OpenTelemetry `ExportLogsServiceRequest` JSON (OTLP/HTTP JSON or decoded protobuf).
    pub async fn ingest_otlp_logs_json(
        &self,
        events_db: &Path,
        incidents_db: &Path,
        config: &TomlValue,
        payload: &serde_json::Value,
        max_records: usize,
    ) -> Result<OtlpLogsIngestResult> {
        let built = otlp_logs::build_new_event_records_from_otlp_logs_json(payload, max_records)
            .context("parse OTLP logs JSON")?;
        let mut rejected = built.rejected_log_records;
        if built.records.is_empty() {
            return Ok(OtlpLogsIngestResult {
                inserted: 0,
                rejected_log_records: rejected,
                suppressed_duplicates: 0,
                suppressed_noise: 0,
            });
        }
        adjust_queue_depth(1);
        let result = persist_events(events_db, config, &built.records);
        adjust_queue_depth(-1);
        let result = result?;
        rejected += result.suppressed_duplicates as u64 + result.suppressed_noise as u64;
        if result.inserted > 0 {
            reconcile_new_events(events_db, incidents_db, config, &result.inserted_event_ids)?;
            let last_ts = built.records.last().map(|r| r.timestamp.clone());
            self.bump_success(
                "otlp",
                "otlp_json",
                result.inserted as u64,
                (result.suppressed_duplicates + result.suppressed_noise) as u64,
                last_ts,
            )
            .await;
        }
        Ok(OtlpLogsIngestResult {
            inserted: result.inserted as u64,
            rejected_log_records: rejected,
            suppressed_duplicates: result.suppressed_duplicates as u64,
            suppressed_noise: result.suppressed_noise as u64,
        })
    }

    async fn upsert_status(&self, row: CollectorRuntimeRow) {
        self.inner
            .statuses
            .write()
            .await
            .insert(row.collector_id.clone(), row);
    }

    async fn bump_success(
        &self,
        collector_id: &str,
        source_type: &str,
        emitted: u64,
        dropped: u64,
        last_event_at: Option<String>,
    ) {
        let mut statuses = self.inner.statuses.write().await;
        let row = statuses
            .entry(collector_id.to_string())
            .or_insert_with(|| CollectorRuntimeRow {
                collector_id: collector_id.to_string(),
                source_type: source_type.to_string(),
                status: "running".into(),
                is_running: true,
                ..Default::default()
            });
        row.is_running = true;
        row.status = "running".into();
        row.events_emitted += emitted;
        row.dropped_events += dropped;
        let now_ms = epoch_millis();
        let mut rate_state = self
            .inner
            .rate_state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = rate_state
            .entry(collector_id.to_string())
            .or_insert(CollectorRateState {
                last_events_emitted: row.events_emitted.saturating_sub(emitted),
                last_observed_at_ms: now_ms,
            });
        let emitted_delta = row.events_emitted.saturating_sub(state.last_events_emitted);
        let elapsed_ms = (now_ms - state.last_observed_at_ms).max(1) as f64;
        row.events_per_second = (emitted_delta as f64 / elapsed_ms) * 1000.0;
        state.last_events_emitted = row.events_emitted;
        state.last_observed_at_ms = now_ms;
        if let Some(last_event_at) = last_event_at {
            row.lag_seconds = lag_seconds_for(&last_event_at);
            row.last_event_at = Some(last_event_at);
        }
    }

    async fn record_error(&self, collector_id: &str, source_type: &str, error: &str) {
        let mut statuses = self.inner.statuses.write().await;
        let row = statuses
            .entry(collector_id.to_string())
            .or_insert_with(|| CollectorRuntimeRow {
                collector_id: collector_id.to_string(),
                source_type: source_type.to_string(),
                ..Default::default()
            });
        row.is_running = true;
        row.status = "error".into();
        row.error_count += 1;
        row.last_error = Some(error.to_string());
        row.last_error_at = Some(now_iso());
    }

    async fn record_unavailable(&self, collector_id: &str, source_type: &str, reason: &str) {
        let mut statuses = self.inner.statuses.write().await;
        let row = statuses
            .entry(collector_id.to_string())
            .or_insert_with(|| CollectorRuntimeRow {
                collector_id: collector_id.to_string(),
                source_type: source_type.to_string(),
                ..Default::default()
            });
        row.is_running = false;
        row.status = "unavailable".into();
        row.last_error = Some(reason.to_string());
        row.last_error_at = Some(now_iso());
    }
}

fn planned_collectors() -> Vec<CollectorHealth> {
    vec![
        CollectorHealth {
            collector_id: "host_metrics".into(),
            status: "planned".into(),
            source_type: "host".into(),
        },
        CollectorHealth {
            collector_id: "process".into(),
            status: "planned".into(),
            source_type: "process_snapshot".into(),
        },
    ]
}

fn source_type_for(collector_id: &str) -> &'static str {
    match collector_id {
        "docker" => "container",
        "journald" => "journald",
        "file" => "file",
        "process" => "process_snapshot",
        "app" => "app_http",
        "windows_eventlog" => "eventlog",
        "host_metrics" => "host",
        "windows_service" => "service",
        "linux_syslog" => "syslog",
        "kubernetes" => "kubernetes",
        _ => "runtime",
    }
}

pub fn configured_collectors(config: &TomlValue) -> Vec<CollectorHealth> {
    let Some(collectors) = config.get("collectors").and_then(TomlValue::as_table) else {
        return planned_collectors();
    };

    let mut rows = Vec::new();
    for (collector_id, section) in collectors {
        let Some(table) = section.as_table() else {
            continue;
        };
        let enabled = table
            .get("enabled")
            .and_then(TomlValue::as_bool)
            .unwrap_or(true);
        rows.push(CollectorHealth {
            collector_id: collector_id.clone(),
            status: if enabled {
                "configured".into()
            } else {
                "disabled".into()
            },
            source_type: source_type_for(collector_id).into(),
        });
    }

    if rows.is_empty() {
        planned_collectors()
    } else {
        rows.sort_by(|a, b| a.collector_id.cmp(&b.collector_id));
        rows
    }
}

fn collector_specs(config: &TomlValue) -> Vec<CollectorSpec> {
    let Some(collectors) = config.get("collectors").and_then(TomlValue::as_table) else {
        return vec![];
    };
    let mut specs = Vec::new();

    if let Some(table) = collectors.get("host_metrics").and_then(TomlValue::as_table) {
        if table
            .get("enabled")
            .and_then(TomlValue::as_bool)
            .unwrap_or(true)
        {
            specs.push(CollectorSpec::HostMetrics {
                poll_interval: poll_interval(table, 10.0),
                warn_cpu_percent: table
                    .get("warn_cpu_percent")
                    .and_then(TomlValue::as_float)
                    .unwrap_or(85.0) as f32,
                warn_memory_percent: table
                    .get("warn_memory_percent")
                    .and_then(TomlValue::as_float)
                    .unwrap_or(85.0) as f32,
                warn_disk_percent: table
                    .get("warn_disk_percent")
                    .and_then(TomlValue::as_float)
                    .unwrap_or(90.0) as f32,
            });
        }
    }
    if let Some(table) = collectors.get("process").and_then(TomlValue::as_table) {
        if table
            .get("enabled")
            .and_then(TomlValue::as_bool)
            .unwrap_or(true)
        {
            specs.push(CollectorSpec::Process {
                poll_interval: poll_interval(table, 10.0),
                top_n: table
                    .get("top_n")
                    .and_then(TomlValue::as_integer)
                    .unwrap_or(20) as usize,
                min_cpu_percent: table
                    .get("min_cpu_percent")
                    .and_then(TomlValue::as_float)
                    .unwrap_or(75.0) as f32,
                min_memory_mb: table
                    .get("min_memory_mb")
                    .and_then(TomlValue::as_float)
                    .unwrap_or(512.0),
                watch_processes: table
                    .get("watch_processes")
                    .and_then(TomlValue::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().map(|s| s.to_ascii_lowercase()))
                    .collect(),
                watch_pids: table
                    .get("watch_pids")
                    .and_then(TomlValue::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_integer().map(|n| n as u32))
                    .collect(),
            });
        }
    }
    if cfg!(target_os = "linux") {
        if let Some(table) = collectors.get("linux_syslog").and_then(TomlValue::as_table) {
            if table
                .get("enabled")
                .and_then(TomlValue::as_bool)
                .unwrap_or(true)
            {
                specs.push(CollectorSpec::LinuxSyslog {
                    poll_interval: poll_interval(table, 2.0),
                    paths: table
                        .get("paths")
                        .and_then(TomlValue::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|value| value.as_str().map(PathBuf::from))
                        .collect(),
                    start_at_end: table
                        .get("start_at_end")
                        .and_then(TomlValue::as_bool)
                        .unwrap_or(true),
                });
            }
        }
    }
    if let Some(table) = collectors.get("file").and_then(TomlValue::as_table) {
        if table
            .get("enabled")
            .and_then(TomlValue::as_bool)
            .unwrap_or(true)
        {
            let mut targets = table
                .get("paths")
                .and_then(TomlValue::as_array)
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str())
                .map(|path| FileTailTarget {
                    path: PathBuf::from(path),
                    service_id: None,
                    service_id_from_filename: false,
                    multiline_pattern: None,
                })
                .collect::<Vec<_>>();
            if let Some(entries) = table.get("entries").and_then(TomlValue::as_array) {
                for entry in entries.iter().filter_map(TomlValue::as_table) {
                    let path = entry
                        .get("path")
                        .and_then(TomlValue::as_str)
                        .unwrap_or_default();
                    let glob = entry
                        .get("glob")
                        .and_then(TomlValue::as_str)
                        .unwrap_or_default();
                    if path.is_empty() && glob.is_empty() {
                        continue;
                    }
                    targets.push(FileTailTarget {
                        path: if !path.is_empty() {
                            PathBuf::from(path)
                        } else {
                            PathBuf::from(glob)
                        },
                        service_id: entry
                            .get("service_id")
                            .and_then(TomlValue::as_str)
                            .map(str::to_string)
                            .filter(|value| !value.is_empty()),
                        service_id_from_filename: entry
                            .get("service_id_from_filename")
                            .and_then(TomlValue::as_bool)
                            .unwrap_or(false),
                        multiline_pattern: entry
                            .get("multiline_pattern")
                            .and_then(TomlValue::as_str)
                            .map(str::to_string)
                            .filter(|value| !value.is_empty()),
                    });
                }
            }
            if !targets.is_empty() {
                specs.push(CollectorSpec::File {
                    poll_interval: poll_interval(table, 1.0),
                    start_at_end: table
                        .get("start_at_end")
                        .and_then(TomlValue::as_bool)
                        .unwrap_or(false),
                    targets,
                });
            }
        }
    }
    if cfg!(target_os = "linux") {
        if let Some(table) = collectors.get("journald").and_then(TomlValue::as_table) {
            if table
                .get("enabled")
                .and_then(TomlValue::as_bool)
                .unwrap_or(true)
            {
                specs.push(CollectorSpec::Journald {
                    poll_interval: poll_interval(table, 5.0),
                    units: string_array(table.get("units")),
                    exclude_units: string_array(table.get("exclude_units")),
                    min_priority: table
                        .get("min_priority")
                        .and_then(TomlValue::as_integer)
                        .unwrap_or(6),
                    since: table
                        .get("since")
                        .and_then(TomlValue::as_str)
                        .unwrap_or("-1 hour")
                        .to_string(),
                    limit: table
                        .get("limit")
                        .and_then(TomlValue::as_integer)
                        .unwrap_or(200) as usize,
                });
            }
        }
    }
    if cfg!(target_os = "windows") {
        if let Some(table) = collectors
            .get("windows_eventlog")
            .and_then(TomlValue::as_table)
        {
            if table
                .get("enabled")
                .and_then(TomlValue::as_bool)
                .unwrap_or(true)
            {
                specs.push(CollectorSpec::WindowsEventLog {
                    poll_interval: poll_interval(table, 5.0),
                    channels: string_array(table.get("channels")),
                });
            }
        }
        if let Some(table) = collectors
            .get("windows_service")
            .and_then(TomlValue::as_table)
        {
            if table
                .get("enabled")
                .and_then(TomlValue::as_bool)
                .unwrap_or(true)
            {
                specs.push(CollectorSpec::WindowsService {
                    poll_interval: poll_interval(table, 30.0),
                    include_stopped: table
                        .get("include_stopped")
                        .and_then(TomlValue::as_bool)
                        .unwrap_or(false),
                    include_automatic_stopped: table
                        .get("include_automatic_stopped")
                        .and_then(TomlValue::as_bool)
                        .unwrap_or(true),
                    names: string_array(table.get("names"))
                        .into_iter()
                        .map(|value| value.to_ascii_lowercase())
                        .collect(),
                    exclude_names: string_array(table.get("exclude_names"))
                        .into_iter()
                        .map(|value| value.to_ascii_lowercase())
                        .collect(),
                });
            }
        }
    }
    if let Some(table) = collectors.get("docker").and_then(TomlValue::as_table) {
        if table
            .get("enabled")
            .and_then(TomlValue::as_bool)
            .unwrap_or(true)
        {
            specs.push(CollectorSpec::Docker {
                poll_interval: poll_interval(table, 10.0),
                socket: table
                    .get("socket")
                    .and_then(TomlValue::as_str)
                    .unwrap_or("/var/run/docker.sock")
                    .to_string(),
                include_names: string_array(table.get("include_names"))
                    .into_iter()
                    .map(|value| value.to_ascii_lowercase())
                    .collect(),
                include_labels: string_array(table.get("include_labels"))
                    .into_iter()
                    .map(|value| value.to_ascii_lowercase())
                    .collect(),
                exclude_names: string_array(table.get("exclude_names"))
                    .into_iter()
                    .map(|value| value.to_ascii_lowercase())
                    .collect(),
                include_all: table
                    .get("include_all")
                    .and_then(TomlValue::as_bool)
                    .unwrap_or(true),
            });
        }
    }
    if let Some(table) = collectors.get("kubernetes").and_then(TomlValue::as_table) {
        if table
            .get("enabled")
            .and_then(TomlValue::as_bool)
            .unwrap_or(false)
        {
            specs.push(CollectorSpec::Kubernetes {
                poll_interval: poll_interval(table, 15.0),
                namespaces: string_array(table.get("namespaces")),
                all_namespaces: table
                    .get("all_namespaces")
                    .and_then(TomlValue::as_bool)
                    .unwrap_or(true),
                label_selector: table
                    .get("label_selector")
                    .and_then(TomlValue::as_str)
                    .unwrap_or_default()
                    .to_string(),
                limit: table
                    .get("limit")
                    .and_then(TomlValue::as_integer)
                    .unwrap_or(200) as usize,
                include_pods: table
                    .get("include_pods")
                    .and_then(TomlValue::as_bool)
                    .unwrap_or(true),
                include_events: table
                    .get("include_events")
                    .and_then(TomlValue::as_bool)
                    .unwrap_or(true),
            });
        }
    }
    if let Some(table) = collectors.get("app").and_then(TomlValue::as_table) {
        if table
            .get("enabled")
            .and_then(TomlValue::as_bool)
            .unwrap_or(true)
        {
            if table
                .get("enable_main_api")
                .and_then(TomlValue::as_bool)
                .unwrap_or(true)
            {
                specs.push(CollectorSpec::AppIngest);
            }
            if table
                .get("enable_standalone")
                .and_then(TomlValue::as_bool)
                .unwrap_or(false)
            {
                specs.push(CollectorSpec::AppStandalone {
                    listen: table
                        .get("listen")
                        .and_then(TomlValue::as_str)
                        .unwrap_or("127.0.0.1:9876")
                        .to_string(),
                    mount_path: table
                        .get("mount_path")
                        .and_then(TomlValue::as_str)
                        .unwrap_or("/api/ingest")
                        .to_string(),
                    shared_token: table
                        .get("shared_token")
                        .and_then(TomlValue::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    max_payload_bytes: table
                        .get("max_payload_bytes")
                        .and_then(TomlValue::as_integer)
                        .unwrap_or(65536) as usize,
                });
            }
        }
    }

    specs
}

fn string_array(value: Option<&TomlValue>) -> Vec<String> {
    value
        .and_then(TomlValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(TomlValue::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn poll_interval(table: &toml::map::Map<String, TomlValue>, default_seconds: f64) -> Duration {
    Duration::from_secs_f64(
        table
            .get("poll_interval_seconds")
            .and_then(TomlValue::as_float)
            .unwrap_or(default_seconds)
            .max(0.5),
    )
}

#[allow(clippy::too_many_arguments)]
async fn run_host_metrics(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    poll_interval: Duration,
    warn_cpu_percent: f32,
    warn_memory_percent: f32,
    warn_disk_percent: f32,
) {
    let collector_id = "host_metrics";
    let source_type = "host";
    let collector = CollectorTaskContext {
        runtime: &runtime,
        events_db: &events_db,
        incidents_db: &incidents_db,
        config: &config,
        collector_id,
        source_type,
    };
    runtime
        .upsert_status(CollectorRuntimeRow {
            collector_id: collector_id.into(),
            status: "running".into(),
            source_type: source_type.into(),
            is_running: true,
            ..Default::default()
        })
        .await;
    let mut state = ThresholdState::default();
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        let sample = match tokio::task::spawn_blocking(collect_host_sample).await {
            Ok(sample) => sample,
            Err(error) => {
                record_collector_error(&collector, &format!("host sample task failed: {error}"))
                    .await;
                tokio::select! {
                    _ = stop_rx.changed() => {},
                    _ = tokio::time::sleep(poll_interval) => {},
                }
                continue;
            }
        };
        match sample {
            Ok(sample) => {
                let events = state
                    .update_and_build_event(
                        &sample,
                        warn_cpu_percent,
                        warn_memory_percent,
                        warn_disk_percent,
                    )
                    .into_iter()
                    .collect::<Vec<_>>();
                handle_collected_events(&collector, observed_at, events).await;
            }
            Err(error) => record_collector_error(&collector, &format!("{error:#}")).await,
        }
        tokio::select! {
            _ = stop_rx.changed() => {},
            _ = tokio::time::sleep(poll_interval) => {},
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_process_snapshot(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    poll_interval: Duration,
    top_n: usize,
    min_cpu_percent: f32,
    min_memory_mb: f64,
    watch_processes: HashSet<String>,
    watch_pids: HashSet<u32>,
) {
    let collector_id = "process";
    let source_type = "process_snapshot";
    let collector = CollectorTaskContext {
        runtime: &runtime,
        events_db: &events_db,
        incidents_db: &incidents_db,
        config: &config,
        collector_id,
        source_type,
    };
    runtime
        .upsert_status(CollectorRuntimeRow {
            collector_id: collector_id.into(),
            status: "running".into(),
            source_type: source_type.into(),
            is_running: true,
            ..Default::default()
        })
        .await;
    let mut seen_hot: HashSet<String> = HashSet::new();
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        let watch_processes = watch_processes.clone();
        let watch_pids = watch_pids.clone();
        let mut seen_hot_local = seen_hot.clone();
        let collect_result = tokio::task::spawn_blocking(move || {
            collect_process_events(
                top_n,
                min_cpu_percent,
                min_memory_mb,
                &watch_processes,
                &watch_pids,
                &mut seen_hot_local,
            )
            .map(|events| (events, seen_hot_local))
        })
        .await;
        match collect_result {
            Ok(Ok((events, updated_seen))) => {
                seen_hot = updated_seen;
                handle_collected_events(&collector, observed_at, events).await;
            }
            Ok(Err(error)) => record_collector_error(&collector, &format!("{error:#}")).await,
            Err(error) => {
                record_collector_error(
                    &collector,
                    &format!("process snapshot task failed: {error}"),
                )
                .await
            }
        }
        tokio::select! {
            _ = stop_rx.changed() => {},
            _ = tokio::time::sleep(poll_interval) => {},
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_linux_syslog(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    poll_interval: Duration,
    paths: Vec<PathBuf>,
    start_at_end: bool,
) {
    let collector_id = "linux_syslog";
    let source_type = "syslog";
    let collector = CollectorTaskContext {
        runtime: &runtime,
        events_db: &events_db,
        incidents_db: &incidents_db,
        config: &config,
        collector_id,
        source_type,
    };
    runtime
        .upsert_status(CollectorRuntimeRow {
            collector_id: collector_id.into(),
            status: "running".into(),
            source_type: source_type.into(),
            is_running: true,
            ..Default::default()
        })
        .await;
    let mut started = false;
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        match collect_syslog_events(&events_db, &paths, start_at_end && !started) {
            Ok(events) => {
                started = true;
                handle_collected_events(&collector, observed_at, events).await;
            }
            Err(error) => record_collector_error(&collector, &format!("{error:#}")).await,
        }
        tokio::select! {
            _ = stop_rx.changed() => {},
            _ = tokio::time::sleep(poll_interval) => {},
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_file_tail(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    poll_interval: Duration,
    start_at_end: bool,
    targets: Vec<FileTailTarget>,
) {
    let collector_id = "file";
    let source_type = "file";
    let collector = CollectorTaskContext {
        runtime: &runtime,
        events_db: &events_db,
        incidents_db: &incidents_db,
        config: &config,
        collector_id,
        source_type,
    };
    let mut started = false;
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        match collect_file_events(&events_db, &targets, start_at_end && !started) {
            Ok(events) => {
                started = true;
                handle_collected_events(&collector, observed_at, events).await;
            }
            Err(error) => record_collector_error(&collector, &format!("{error:#}")).await,
        }
        tokio::select! {
            _ = stop_rx.changed() => {},
            _ = tokio::time::sleep(poll_interval) => {},
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_journald(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    poll_interval: Duration,
    units: Vec<String>,
    exclude_units: Vec<String>,
    min_priority: i64,
    since: String,
    limit: usize,
) {
    let collector_id = "journald";
    let source_type = "journald";
    let collector = CollectorTaskContext {
        runtime: &runtime,
        events_db: &events_db,
        incidents_db: &incidents_db,
        config: &config,
        collector_id,
        source_type,
    };
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        match collect_journald_events(
            &events_db,
            &units,
            &exclude_units,
            min_priority,
            &since,
            limit,
        ) {
            Ok(events) => {
                handle_collected_events(&collector, observed_at, events).await;
            }
            Err(error) => {
                let message = format!("{error:#}");
                if let Some(reason) = collector_unavailable_reason(collector_id, &message) {
                    record_collector_unavailable(&collector, &reason).await;
                } else {
                    record_collector_error(&collector, &message).await;
                }
            }
        }
        tokio::select! {
            _ = stop_rx.changed() => {},
            _ = tokio::time::sleep(poll_interval) => {},
        }
    }
}

async fn run_windows_eventlog(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    poll_interval: Duration,
    channels: Vec<String>,
) {
    let collector_id = "windows_eventlog";
    let source_type = "eventlog";
    let collector = CollectorTaskContext {
        runtime: &runtime,
        events_db: &events_db,
        incidents_db: &incidents_db,
        config: &config,
        collector_id,
        source_type,
    };
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        let events_db_for_collect = events_db.clone();
        let channels_for_collect = channels.clone();
        let collect_result = tokio::task::spawn_blocking(move || {
            collect_windows_eventlog_events(&events_db_for_collect, &channels_for_collect)
        })
        .await;
        match collect_result {
            Ok(Ok(events)) => {
                handle_collected_events(&collector, observed_at, events).await;
            }
            Ok(Err(error)) => record_collector_error(&collector, &format!("{error:#}")).await,
            Err(error) => {
                record_collector_error(
                    &collector,
                    &format!("windows eventlog task failed: {error}"),
                )
                .await
            }
        }
        tokio::select! {
            _ = stop_rx.changed() => {},
            _ = tokio::time::sleep(poll_interval) => {},
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_windows_service(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    poll_interval: Duration,
    include_stopped: bool,
    include_automatic_stopped: bool,
    names: HashSet<String>,
    exclude_names: HashSet<String>,
) {
    let collector_id = "windows_service";
    let source_type = "service";
    let collector = CollectorTaskContext {
        runtime: &runtime,
        events_db: &events_db,
        incidents_db: &incidents_db,
        config: &config,
        collector_id,
        source_type,
    };
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        let events_db_for_collect = events_db.clone();
        let names = names.clone();
        let exclude_names = exclude_names.clone();
        let collect_result = tokio::task::spawn_blocking(move || {
            collect_windows_service_events(
                &events_db_for_collect,
                include_stopped,
                include_automatic_stopped,
                &names,
                &exclude_names,
            )
        })
        .await;
        match collect_result {
            Ok(Ok(events)) => {
                handle_collected_events(&collector, observed_at, events).await;
            }
            Ok(Err(error)) => record_collector_error(&collector, &format!("{error:#}")).await,
            Err(error) => {
                record_collector_error(&collector, &format!("windows service task failed: {error}"))
                    .await
            }
        }
        tokio::select! {
            _ = stop_rx.changed() => {},
            _ = tokio::time::sleep(poll_interval) => {},
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_docker(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    poll_interval: Duration,
    socket: String,
    include_names: HashSet<String>,
    include_labels: HashSet<String>,
    exclude_names: HashSet<String>,
    include_all: bool,
) {
    let collector_id = "docker";
    let source_type = "container";
    let collector = CollectorTaskContext {
        runtime: &runtime,
        events_db: &events_db,
        incidents_db: &incidents_db,
        config: &config,
        collector_id,
        source_type,
    };
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        match collect_docker_events(
            &events_db,
            &socket,
            &include_names,
            &include_labels,
            &exclude_names,
            include_all,
        ) {
            Ok(events) => {
                handle_collected_events(&collector, observed_at, events).await;
            }
            Err(error) => {
                let message = format!("{error:#}");
                if let Some(reason) = collector_unavailable_reason(collector_id, &message) {
                    record_collector_unavailable(&collector, &reason).await;
                } else {
                    record_collector_error(&collector, &message).await;
                }
            }
        }
        tokio::select! {
            _ = stop_rx.changed() => {},
            _ = tokio::time::sleep(poll_interval) => {},
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_kubernetes(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    poll_interval: Duration,
    namespaces: Vec<String>,
    all_namespaces: bool,
    label_selector: String,
    limit: usize,
    include_pods: bool,
    include_events: bool,
) {
    let collector_id = "kubernetes";
    let source_type = "kubernetes";
    let collector = CollectorTaskContext {
        runtime: &runtime,
        events_db: &events_db,
        incidents_db: &incidents_db,
        config: &config,
        collector_id,
        source_type,
    };
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        match collect_kubernetes_events(
            &events_db,
            &namespaces,
            all_namespaces,
            &label_selector,
            limit,
            include_pods,
            include_events,
        ) {
            Ok(events) => {
                handle_collected_events(&collector, observed_at, events).await;
            }
            Err(error) => {
                let message = format!("{error:#}");
                if let Some(reason) = collector_unavailable_reason(collector_id, &message) {
                    record_collector_unavailable(&collector, &reason).await;
                } else {
                    record_collector_error(&collector, &message).await;
                }
            }
        }
        tokio::select! {
            _ = stop_rx.changed() => {},
            _ = tokio::time::sleep(poll_interval) => {},
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_app_standalone(
    runtime: CollectorRuntime,
    mut stop_rx: watch::Receiver<bool>,
    events_db: PathBuf,
    incidents_db: PathBuf,
    config: TomlValue,
    listen: String,
    mount_path: String,
    shared_token: String,
    max_payload_bytes: usize,
) {
    let state = AppStandaloneState {
        runtime,
        events_db,
        incidents_db,
        config,
        shared_token,
    };
    let router = Router::new()
        .route(
            &normalized_mount_path(&mount_path),
            post(handle_app_standalone_ingest),
        )
        .with_state(state.clone())
        .layer(DefaultBodyLimit::max(max_payload_bytes));
    let listener = match tokio::net::TcpListener::bind(&listen).await {
        Ok(listener) => listener,
        Err(error) => {
            state
                .runtime
                .record_error(
                    "app",
                    "app_http",
                    &format!("bind standalone app ingest {listen}: {error}"),
                )
                .await;
            return;
        }
    };
    let shutdown = async move {
        let _ = stop_rx.changed().await;
    };
    if let Err(error) = axum::serve(listener, router)
        .with_graceful_shutdown(shutdown)
        .await
    {
        state
            .runtime
            .record_error(
                "app",
                "app_http",
                &format!("serve standalone app ingest: {error}"),
            )
            .await;
    }
}

async fn handle_app_standalone_ingest(
    State(state): State<AppStandaloneState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !state.shared_token.is_empty() {
        let auth = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let expected = format!("Bearer {}", state.shared_token);
        if auth != expected {
            return Err((
                StatusCode::UNAUTHORIZED,
                "missing or invalid bearer token".to_string(),
            ));
        }
    }
    let result = state
        .runtime
        .ingest_app_event(
            &state.events_db,
            &state.incidents_db,
            &state.config,
            &payload,
        )
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(Json(serde_json::json!({
        "event_id": result.event_id,
        "accepted": result.accepted,
        "suppressed_duplicates": result.suppressed_duplicates,
        "suppressed_noise": result.suppressed_noise,
    })))
}

async fn handle_collected_events(
    context: &CollectorTaskContext<'_>,
    observed_at: String,
    events: Vec<NewEventRecord>,
) {
    if events.is_empty() {
        context
            .runtime
            .bump_success(
                context.collector_id,
                context.source_type,
                0,
                0,
                Some(observed_at),
            )
            .await;
        return;
    }
    let events_db_owned = context.events_db.to_path_buf();
    let incidents_db_owned = context.incidents_db.to_path_buf();
    let config_owned = context.config.clone();
    let events_owned = events;
    let persist_result = tokio::task::spawn_blocking(move || {
        persist_events_and_reconcile(
            &events_db_owned,
            &incidents_db_owned,
            &config_owned,
            &events_owned,
        )
    })
    .await;
    match persist_result {
        Ok(Ok(result)) => {
            context
                .runtime
                .bump_success(
                    context.collector_id,
                    context.source_type,
                    result.inserted as u64,
                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                    Some(observed_at),
                )
                .await;
        }
        Ok(Err(error)) => {
            record_collector_error(context, &format!("{error:#}")).await;
        }
        Err(error) => {
            record_collector_error(context, &format!("persist task failed: {error}")).await;
        }
    }
}

async fn record_collector_error(context: &CollectorTaskContext<'_>, error: &str) {
    context
        .runtime
        .record_error(context.collector_id, context.source_type, error)
        .await;
    let _ = persist_collector_diagnostic_event(context, "collector_error", 3, error);
}

async fn record_collector_unavailable(context: &CollectorTaskContext<'_>, reason: &str) {
    context
        .runtime
        .record_unavailable(context.collector_id, context.source_type, reason)
        .await;
    let _ = persist_collector_diagnostic_event(context, "collector_unavailable", 1, reason);
}

fn persist_collector_diagnostic_event(
    context: &CollectorTaskContext<'_>,
    kind: &str,
    severity: i64,
    reason: &str,
) -> Result<()> {
    let collector_id = context.collector_id;
    let source_type = context.source_type;
    let message = format!("collector {collector_id} {kind}: {reason}");
    let event = NewEventRecord {
        event_id: next_event_id("collector"),
        timestamp: now_iso(),
        service_id: collector_id.to_string(),
        severity,
        message: message.clone(),
        source_type: "collector_runtime".into(),
        source_id: format!("collector://{collector_id}"),
        tags: vec![
            "collector".into(),
            collector_id.to_string(),
            kind.to_string(),
        ],
        fingerprint: semantic_fingerprint("collector", collector_id, source_type, &message, None),
        host_id: System::host_name().unwrap_or_else(|| "local".into()),
        event_type: 1,
        timestamp_source: "collector".into(),
        collected_at: now_iso(),
        quality: Some("diagnostic".into()),
        structured_data: Some(serde_json::json!({
            "collector_id": collector_id,
            "collector_source_type": source_type,
            "kind": kind,
            "reason": reason,
        })),
        raw_offset: None,
        trace_id: None,
        span_id: None,
        signal_kind: "log".into(),
        deployment_environment: None,
        severity_text: None,
    };
    persist_events_and_reconcile(
        context.events_db,
        context.incidents_db,
        context.config,
        &[event],
    )
    .map(|_| ())
}

fn collector_unavailable_reason(collector_id: &str, message: &str) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    match collector_id {
        "docker"
            if lower.contains("program not found")
                || lower.contains("os error 2")
                || lower.contains("not recognized")
                || lower.contains("cannot connect to the docker daemon")
                || lower.contains("docker daemon") =>
        {
            Some(
                "Docker is enabled in Inferra, but Docker is not installed or the Docker daemon is not reachable on this host. Marking the collector unavailable instead of degrading system health."
                    .into(),
            )
        }
        "kubernetes"
            if lower.contains("program not found")
                || lower.contains("os error 2")
                || lower.contains("not recognized")
                || lower.contains("connection refused")
                || lower.contains("unable to connect")
                || lower.contains("forbidden")
                || lower.contains("unauthorized") =>
        {
            Some(
                "Kubernetes is enabled in Inferra, but kubectl, kubeconfig, cluster connectivity, or RBAC is unavailable on this host."
                    .into(),
            )
        }
        "journald"
            if lower.contains("program not found")
                || lower.contains("os error 2")
                || lower.contains("not recognized")
                || lower.contains("no journal files") =>
        {
            Some(
                "journald is enabled in Inferra, but journalctl or readable systemd journal data is unavailable on this host."
                    .into(),
            )
        }
        _ => None,
    }
}

#[derive(Default)]
struct ThresholdState {
    cpu_high: bool,
    memory_high: bool,
    disk_high: bool,
}

impl ThresholdState {
    fn update_and_build_event(
        &mut self,
        sample: &HostSample,
        warn_cpu_percent: f32,
        warn_memory_percent: f32,
        warn_disk_percent: f32,
    ) -> Option<NewEventRecord> {
        let cpu_high = sample.cpu_percent >= warn_cpu_percent;
        let memory_high = sample.memory_percent >= warn_memory_percent;
        let disk_high = sample.disk_percent >= warn_disk_percent;
        let mut entered = Vec::new();
        let mut recovered = Vec::new();
        for (name, next, previous) in [
            ("cpu", cpu_high, self.cpu_high),
            ("memory", memory_high, self.memory_high),
            ("disk", disk_high, self.disk_high),
        ] {
            if next && !previous {
                entered.push(name);
            } else if !next && previous {
                recovered.push(name);
            }
        }
        self.cpu_high = cpu_high;
        self.memory_high = memory_high;
        self.disk_high = disk_high;
        if entered.is_empty() && recovered.is_empty() {
            return None;
        }
        let severity = if !entered.is_empty() { 2 } else { 1 };
        let message = if !entered.is_empty() {
            format!("host resource pressure detected: {}", entered.join(", "))
        } else {
            format!("host resource pressure recovered: {}", recovered.join(", "))
        };
        let mut attrs = serde_json::Map::new();
        insert_attr(
            &mut attrs,
            "host.cpu_percent",
            serde_json::json!(sample.cpu_percent),
        );
        insert_attr(
            &mut attrs,
            "host.memory_percent",
            serde_json::json!(sample.memory_percent),
        );
        insert_attr(
            &mut attrs,
            "host.disk_percent",
            serde_json::json!(sample.disk_percent),
        );
        insert_attr(
            &mut attrs,
            "host.disk_free_bytes",
            serde_json::json!(sample.disk_free_bytes),
        );
        if let Some(worst_disk) = sample.disks.iter().max_by(|left, right| {
            left.percent
                .partial_cmp(&right.percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            insert_attr(
                &mut attrs,
                "host.disk_path",
                serde_json::json!(worst_disk.path),
            );
        }
        let mut tags = vec!["host".into()];
        tags.push(if !entered.is_empty() {
            "resource_pressure".into()
        } else {
            "recovered".into()
        });
        tags.extend(
            entered
                .iter()
                .chain(recovered.iter())
                .map(|value| value.to_string()),
        );
        Some(collector_event(CollectorEventArgs {
            id_prefix: "host",
            timestamp: None,
            service_id: "host".into(),
            severity,
            message,
            source_type: "host_metrics".into(),
            source_id: "host_metrics://local".into(),
            tags,
            fingerprint: Some(format!(
                "host-metrics:{}",
                entered
                    .iter()
                    .chain(recovered.iter())
                    .map(|value| value.to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join("|")
            )),
            host_id: Some(sample.hostname.clone()),
            kind: CollectorEventKind::Metric,
            quality: "normalized",
            structured_data: Some(structured_with_attributes(
                attrs,
                "metrics",
                serde_json::json!({
                    "cpu_percent": sample.cpu_percent,
                    "memory_percent": sample.memory_percent,
                    "disk_percent": sample.disk_percent,
                    "disk_free_bytes": sample.disk_free_bytes,
                    "disks": sample.disks,
                }),
            )),
            raw_offset: None,
            trace_id: None,
            span_id: None,
            signal_kind: "metric",
            deployment_environment: None,
            severity_text: Some(if severity >= 2 {
                "warn".into()
            } else {
                "info".into()
            }),
        }))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DiskSample {
    path: String,
    percent: f32,
    free_bytes: u64,
    total_bytes: u64,
}

struct HostSample {
    hostname: String,
    cpu_percent: f32,
    memory_percent: f32,
    disk_percent: f32,
    disk_free_bytes: u64,
    disks: Vec<DiskSample>,
}

fn collect_host_sample() -> Result<HostSample> {
    let mut system = System::new_all();
    system.refresh_cpu_usage();
    system.refresh_memory();
    let disks = Disks::new_with_refreshed_list();
    let mut disk_samples = Vec::new();
    for disk in disks.list() {
        let total = disk.total_space().max(1);
        let free = disk.available_space();
        let percent = (((total - free) as f64 / total as f64) * 100.0) as f32;
        disk_samples.push(DiskSample {
            path: disk.mount_point().display().to_string(),
            percent,
            free_bytes: free,
            total_bytes: total,
        });
    }
    let worst_disk = disk_samples.iter().max_by(|left, right| {
        left.percent
            .partial_cmp(&right.percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(HostSample {
        hostname: host_name(),
        cpu_percent: system.global_cpu_usage(),
        memory_percent: if system.total_memory() == 0 {
            0.0
        } else {
            ((system.used_memory() as f64 / system.total_memory() as f64) * 100.0) as f32
        },
        disk_percent: worst_disk.map(|disk| disk.percent).unwrap_or(0.0),
        disk_free_bytes: worst_disk.map(|disk| disk.free_bytes).unwrap_or(0),
        disks: disk_samples,
    })
}

fn collect_process_events(
    top_n: usize,
    min_cpu_percent: f32,
    min_memory_mb: f64,
    watch_processes: &HashSet<String>,
    watch_pids: &HashSet<u32>,
    seen_hot: &mut HashSet<String>,
) -> Result<Vec<NewEventRecord>> {
    let mut system = System::new_all();
    system.refresh_cpu_usage();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let logical_processors = system.cpus().len().max(host_logical_processors()).max(1);
    let mut entries = system
        .processes()
        .values()
        .filter(|process| {
            let pid = process.pid().as_u32();
            let name = process.name().to_string_lossy().to_ascii_lowercase();
            (watch_pids.is_empty() || watch_pids.contains(&pid))
                && (watch_processes.is_empty() || watch_processes.contains(&name))
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.cpu_usage()
            .partial_cmp(&a.cpu_usage())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut active_hot = HashSet::new();
    let mut events = Vec::new();
    for process in entries.into_iter().take(top_n) {
        let pid = process.pid().as_u32();
        let name = process.name().to_string_lossy().to_string();
        let memory_mb = process.memory() as f64 / (1024.0 * 1024.0);
        let cpu = process.cpu_usage();
        let host_cpu = normalize_process_cpu_to_host_percent(cpu, logical_processors);
        let command = process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        let create_time = process.start_time();
        let key = format!("{name}:{pid}:{create_time}");
        let cpu_hot = host_cpu >= min_cpu_percent;
        let memory_hot = memory_mb >= min_memory_mb;
        let hot = cpu_hot || memory_hot;
        if hot {
            active_hot.insert(key.clone());
            if !seen_hot.contains(&key) {
                let entered = [("cpu", cpu_hot), ("memory", memory_hot)]
                    .into_iter()
                    .filter_map(|(name, active)| active.then_some(name))
                    .collect::<Vec<_>>();
                let mut attrs = serde_json::Map::new();
                insert_attr(&mut attrs, "process.pid", serde_json::json!(pid));
                insert_attr(&mut attrs, "process.name", serde_json::json!(name));
                insert_attr(
                    &mut attrs,
                    "process.cpu_percent",
                    serde_json::json!(host_cpu),
                );
                insert_attr(
                    &mut attrs,
                    "process.memory_mb",
                    serde_json::json!(memory_mb),
                );
                insert_attr(
                    &mut attrs,
                    "process.status",
                    serde_json::json!(format!("{:?}", process.status())),
                );
                insert_attr(
                    &mut attrs,
                    "process.create_time",
                    serde_json::json!(create_time),
                );
                let message = format!(
                    "process {name} pid={pid} high {} host_cpu={host_cpu:.1}% raw_process_cpu={cpu:.1}% memory={memory_mb:.1}MB",
                    entered.join(" and ")
                );
                events.push(collector_event(CollectorEventArgs {
                    id_prefix: "process",
                    timestamp: None,
                    service_id: name.clone(),
                    severity: 2,
                    message,
                    source_type: "process_snapshot".into(),
                    source_id: format!("process://{pid}"),
                    tags: vec![
                        "process".into(),
                        "threshold".into(),
                        "resource_pressure".into(),
                    ],
                    fingerprint: Some(format!("process-hot-{key}")),
                    host_id: None,
                    kind: CollectorEventKind::Metric,
                    quality: "normalized",
                    structured_data: Some(structured_with_attributes(
                        attrs,
                        "process",
                        serde_json::json!({
                            "pid": pid,
                            "name": name,
                            "command": command,
                            "create_time": create_time,
                            "threshold_crossings": entered.iter().map(|metric| serde_json::json!({
                                "metric": metric,
                                "state": "entered"
                            })).collect::<Vec<_>>(),
                            "cpu_percent": host_cpu,
                            "cpu_raw_percent": cpu,
                            "cpu_percent_scope": "host_total",
                            "cpu_raw_percent_scope": "single_core_equivalent",
                            "cpu_logical_processors": logical_processors,
                            "memory_mb": memory_mb,
                            "status": format!("{:?}", process.status()),
                        }),
                    )),
                    raw_offset: None,
                    trace_id: None,
                    span_id: None,
                    signal_kind: "metric",
                    deployment_environment: None,
                    severity_text: Some("warn".into()),
                }));
            }
        } else if seen_hot.contains(&key) {
            let mut attrs = serde_json::Map::new();
            insert_attr(&mut attrs, "process.pid", serde_json::json!(pid));
            insert_attr(&mut attrs, "process.name", serde_json::json!(name));
            insert_attr(
                &mut attrs,
                "process.cpu_percent",
                serde_json::json!(host_cpu),
            );
            insert_attr(
                &mut attrs,
                "process.memory_mb",
                serde_json::json!(memory_mb),
            );
            insert_attr(
                &mut attrs,
                "process.status",
                serde_json::json!(format!("{:?}", process.status())),
            );
            insert_attr(
                &mut attrs,
                "process.create_time",
                serde_json::json!(create_time),
            );
            events.push(collector_event(CollectorEventArgs {
                id_prefix: "process",
                timestamp: None,
                service_id: name.clone(),
                severity: 1,
                message: format!(
                    "process {name} pid={pid} recovered host_cpu={host_cpu:.1}% raw_process_cpu={cpu:.1}% memory={memory_mb:.1}MB"
                ),
                source_type: "process_snapshot".into(),
                source_id: format!("process://{pid}"),
                tags: vec!["process".into(), "recovered".into()],
                fingerprint: Some(format!("process-recovered-{key}")),
                host_id: None,
                kind: CollectorEventKind::Metric,
                quality: "normalized",
                structured_data: Some(structured_with_attributes(
                    attrs,
                    "process",
                    serde_json::json!({
                        "pid": pid,
                        "name": name,
                        "command": command,
                        "create_time": create_time,
                        "threshold_crossings": [
                            {"metric": "resource", "state": "recovered"}
                        ],
                        "cpu_percent": host_cpu,
                        "cpu_raw_percent": cpu,
                        "cpu_percent_scope": "host_total",
                        "cpu_raw_percent_scope": "single_core_equivalent",
                        "cpu_logical_processors": logical_processors,
                        "memory_mb": memory_mb,
                        "status": format!("{:?}", process.status()),
                    }),
                )),
                raw_offset: None,
                trace_id: None,
                span_id: None,
                signal_kind: "metric",
                deployment_environment: None,
                severity_text: Some("info".into()),
            }));
        }
    }
    *seen_hot = active_hot;
    Ok(events)
}

fn host_logical_processors() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .max(1)
}

fn normalize_process_cpu_to_host_percent(raw_percent: f32, logical_processors: usize) -> f32 {
    (raw_percent / logical_processors.max(1) as f32).clamp(0.0, 100.0)
}

fn collect_syslog_events(
    events_db: &Path,
    paths: &[PathBuf],
    start_at_end: bool,
) -> Result<Vec<NewEventRecord>> {
    let store =
        EventsStore::open(events_db)?.context("event store not found for syslog collector")?;
    let mut events = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let key = format!("offset:{}", path.display());
        let mut offset = store
            .get_collector_state("linux_syslog", &key)?
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("read metadata for {}", path.display()))?;
        if start_at_end && offset == 0 {
            offset = metadata.len();
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .open(path)
            .with_context(|| format!("open syslog file {}", path.display()))?;
        use std::io::{BufRead, BufReader, Seek, SeekFrom};
        file.seek(SeekFrom::Start(offset))
            .with_context(|| format!("seek syslog file {}", path.display()))?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut current_offset = offset;
        loop {
            line.clear();
            let read = reader
                .read_line(&mut line)
                .with_context(|| format!("read syslog file {}", path.display()))?;
            if read == 0 {
                break;
            }
            current_offset += read as u64;
            let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
            if trimmed.is_empty() {
                continue;
            }
            let parsed = parse_collector_log_line(trimmed);
            let (fallback_service_id, fallback_severity, fallback_message) =
                parse_syslog_line(trimmed);
            let service_id = parsed.service_id.unwrap_or(fallback_service_id);
            let severity = if parsed.parsed_json.is_some() {
                parsed.severity
            } else {
                fallback_severity
            };
            let message = if parsed.parsed_json.is_some() {
                parsed.message
            } else {
                fallback_message
            };
            let mut attrs = parsed.attributes;
            insert_attr(
                &mut attrs,
                "log.file.path",
                serde_json::json!(path.display().to_string()),
            );
            insert_attr(
                &mut attrs,
                "log.raw_offset",
                serde_json::json!(current_offset),
            );
            insert_attr(&mut attrs, "service.name", serde_json::json!(service_id));
            events.push(collector_event(CollectorEventArgs {
                id_prefix: "syslog",
                timestamp: parsed.timestamp,
                service_id,
                severity,
                message,
                source_type: "linux_syslog".into(),
                source_id: path.display().to_string(),
                tags: vec!["syslog".into()],
                fingerprint: Some(semantic_fingerprint(
                    "syslog",
                    &path.display().to_string(),
                    "linux_syslog",
                    trimmed,
                    None,
                )),
                host_id: None,
                kind: CollectorEventKind::Log,
                quality: if parsed.parsed_json.is_some() {
                    "normalized"
                } else {
                    "raw"
                },
                structured_data: Some(structured_with_attributes(
                    attrs,
                    "syslog",
                    serde_json::json!({
                        "path": path.display().to_string(),
                        "raw_offset": current_offset,
                        "raw": trimmed,
                        "parsed": parsed.parsed_json,
                    }),
                )),
                raw_offset: Some(current_offset as i64),
                trace_id: parsed.trace_id,
                span_id: parsed.span_id,
                signal_kind: "log",
                deployment_environment: parsed.deployment_environment,
                severity_text: parsed.severity_text,
            }));
        }
        store.set_collector_state(
            "linux_syslog",
            &key,
            &current_offset.to_string(),
            &now_iso(),
        )?;
    }
    Ok(events)
}

fn parse_syslog_line(line: &str) -> (String, i64, String) {
    let lowered = line.to_ascii_lowercase();
    let severity = if lowered.contains("critical")
        || lowered.contains("fatal")
        || lowered.contains("panic")
    {
        4
    } else if lowered.contains("error") || lowered.contains("failed") || lowered.contains("failure")
    {
        3
    } else if lowered.contains("warn") || lowered.contains("degraded") {
        2
    } else {
        1
    };
    let mut service_id = "syslog".to_string();
    if let Some(colon) = line.find(':') {
        let head = &line[..colon];
        if let Some(last_space) = head.split_whitespace().last() {
            service_id = last_space
                .trim_matches(|ch: char| ch == '[' || ch == ']')
                .to_string();
        }
    }
    (service_id, severity, line.to_string())
}

fn collect_file_events(
    events_db: &Path,
    targets: &[FileTailTarget],
    start_at_end: bool,
) -> Result<Vec<NewEventRecord>> {
    let store =
        EventsStore::open(events_db)?.context("event store not found for file collector")?;
    let mut events = Vec::new();
    for target in resolve_file_targets(targets)? {
        if !target.path.exists() || !target.path.is_file() {
            continue;
        }
        let key = format!("offset:file:{}", target.path.display());
        let mut offset = store
            .get_collector_state("file", &key)?
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let metadata = std::fs::metadata(&target.path)
            .with_context(|| format!("read metadata for {}", target.path.display()))?;
        if start_at_end && offset == 0 {
            offset = metadata.len();
            store.set_collector_state("file", &key, &offset.to_string(), &now_iso())?;
            continue;
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .open(&target.path)
            .with_context(|| format!("open tailed file {}", target.path.display()))?;
        use std::io::{BufRead, BufReader, Seek, SeekFrom};
        file.seek(SeekFrom::Start(offset))
            .with_context(|| format!("seek tailed file {}", target.path.display()))?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut current_offset = offset;
        let mut current_message = String::new();
        let mut message_end_offset = offset;
        while reader
            .read_line(&mut line)
            .with_context(|| format!("read tailed file {}", target.path.display()))?
            > 0
        {
            current_offset += line.len() as u64;
            let trimmed = line.trim_end_matches(&['\r', '\n'][..]).to_string();
            line.clear();
            if trimmed.is_empty() {
                continue;
            }
            let starts_new = target
                .multiline_pattern
                .as_ref()
                .map(|pattern| trimmed.contains(pattern))
                .unwrap_or(true);
            if starts_new && !current_message.is_empty() {
                events.push(file_event_record(
                    &target,
                    &current_message,
                    message_end_offset,
                ));
                current_message.clear();
            }
            if !current_message.is_empty() {
                current_message.push('\n');
            }
            current_message.push_str(&trimmed);
            message_end_offset = current_offset;
        }
        if !current_message.is_empty() {
            events.push(file_event_record(
                &target,
                &current_message,
                message_end_offset,
            ));
        }
        store.set_collector_state("file", &key, &current_offset.to_string(), &now_iso())?;
    }
    Ok(events)
}

fn collect_journald_events(
    events_db: &Path,
    units: &[String],
    exclude_units: &[String],
    min_priority: i64,
    since: &str,
    limit: usize,
) -> Result<Vec<NewEventRecord>> {
    if !cfg!(target_os = "linux") {
        return Ok(vec![]);
    }
    let store =
        EventsStore::open(events_db)?.context("event store not found for journald collector")?;
    let cursor = store.get_collector_state("journald", "cursor")?;
    let mut command = Command::new("journalctl");
    command
        .arg("--no-pager")
        .arg("-o")
        .arg("json")
        .arg("-n")
        .arg(limit.to_string());
    command.arg("-p").arg(min_priority.to_string());
    if let Some(cursor) = cursor.as_deref().filter(|value| !value.is_empty()) {
        command.arg("--after-cursor").arg(cursor);
    } else {
        command.arg("--since").arg(since);
    }
    for unit in units {
        command.arg("-u").arg(unit);
    }
    let output = command
        .output()
        .context("run journalctl for journald collector")?;
    if !output.status.success() {
        anyhow::bail!("journalctl failed: {}", sc_output_text_like(&output));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let exclude: HashSet<String> = exclude_units
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let allow: HashSet<String> = units
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let mut events = Vec::new();
    let mut last_cursor = None::<String>;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let cursor = value
            .get("__CURSOR")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        if cursor.is_some() {
            last_cursor = cursor.clone();
        }
        let unit = value
            .get("_SYSTEMD_UNIT")
            .or_else(|| value.get("UNIT"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !allow.is_empty() && !unit.is_empty() && !allow.contains(&unit) {
            continue;
        }
        if !unit.is_empty() && exclude.contains(&unit) {
            continue;
        }
        let message = value
            .get("MESSAGE")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("journald event")
            .to_string();
        let service_id = value
            .get("SYSLOG_IDENTIFIER")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| (!unit.is_empty()).then_some(unit.clone()))
            .unwrap_or_else(|| "journald".to_string());
        let timestamp = value
            .get("__REALTIME_TIMESTAMP")
            .and_then(serde_json::Value::as_str)
            .and_then(epoch_micros_to_iso)
            .unwrap_or_else(now_iso);
        let priority = value
            .get("PRIORITY")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(6);
        let fingerprint = semantic_fingerprint("journald", &service_id, "journald", &message, None);
        let mut attrs = serde_json::Map::new();
        insert_attr(&mut attrs, "service.name", serde_json::json!(service_id));
        insert_attr(&mut attrs, "journald.priority", serde_json::json!(priority));
        if !unit.is_empty() {
            insert_attr(&mut attrs, "systemd.unit", serde_json::json!(unit));
        }
        insert_scalar_json_attr(&mut attrs, "process.pid", value.get("_PID"));
        insert_scalar_json_attr(&mut attrs, "host.name", value.get("_HOSTNAME"));
        events.push(collector_event(CollectorEventArgs {
            id_prefix: "journald",
            timestamp: Some(timestamp.clone()),
            service_id,
            severity: severity_from_priority(priority),
            message,
            source_type: "journald".into(),
            source_id: cursor.clone().unwrap_or_else(|| "journalctl".into()),
            tags: vec!["journald".into(), "systemd".into()],
            fingerprint: Some(fingerprint),
            host_id: value
                .get("_HOSTNAME")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            kind: CollectorEventKind::Log,
            quality: "raw",
            structured_data: Some(structured_with_attributes(attrs, "journald", value)),
            raw_offset: None,
            trace_id: None,
            span_id: None,
            signal_kind: "log",
            deployment_environment: None,
            severity_text: Some(
                match severity_from_priority(priority) {
                    4 => "critical",
                    3 => "error",
                    2 => "warn",
                    _ => "info",
                }
                .into(),
            ),
        }));
    }
    if let Some(cursor) = last_cursor {
        store.set_collector_state("journald", "cursor", &cursor, &now_iso())?;
    }
    Ok(events)
}

fn collect_windows_eventlog_events(
    events_db: &Path,
    channels: &[String],
) -> Result<Vec<NewEventRecord>> {
    if !cfg!(target_os = "windows") {
        return Ok(vec![]);
    }
    let store = EventsStore::open(events_db)?
        .context("event store not found for windows eventlog collector")?;
    let mut events = Vec::new();
    for channel in channels {
        let state_key = format!("last_record:{channel}");
        let last_record = store
            .get_collector_state("windows_eventlog", &state_key)?
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        if last_record == 0 {
            let output = Command::new("wevtutil")
                .args(["qe", channel, "/rd:true", "/c:1", "/f:xml"])
                .output()
                .with_context(|| format!("query latest windows eventlog record for {channel}"))?;
            if !output.status.success() {
                anyhow::bail!(
                    "wevtutil failed for {channel}: {}",
                    sc_output_text_like(&output)
                );
            }
            let latest =
                parse_wevtutil_xml_events(&String::from_utf8_lossy(&output.stdout), channel)
                    .into_iter()
                    .map(|item| item.record_id)
                    .max()
                    .unwrap_or(0);
            if latest > 0 {
                store.set_collector_state(
                    "windows_eventlog",
                    &state_key,
                    &latest.to_string(),
                    &now_iso(),
                )?;
            }
            continue;
        }
        let query = format!("*[System[(EventRecordID>{last_record})]]");
        let output = Command::new("wevtutil")
            .args(["qe", channel, "/rd:false", "/f:xml", "/c:64"])
            .arg(format!("/q:{query}"))
            .output()
            .with_context(|| format!("query windows eventlog channel {channel}"))?;
        if !output.status.success() {
            anyhow::bail!(
                "wevtutil failed for {channel}: {}",
                sc_output_text_like(&output)
            );
        }
        let parsed = parse_wevtutil_xml_events(&String::from_utf8_lossy(&output.stdout), channel);
        let mut newest = last_record;
        for item in parsed {
            newest = newest.max(item.record_id);
            let mut attrs = serde_json::Map::new();
            insert_attr(
                &mut attrs,
                "windows.eventlog.channel",
                serde_json::json!(channel),
            );
            insert_attr(
                &mut attrs,
                "windows.eventlog.record_id",
                serde_json::json!(item.record_id),
            );
            insert_attr(
                &mut attrs,
                "windows.eventlog.event_id",
                serde_json::json!(item.event_id),
            );
            insert_attr(
                &mut attrs,
                "windows.eventlog.provider",
                serde_json::json!(item.provider),
            );
            insert_attr(
                &mut attrs,
                "windows.eventlog.level",
                serde_json::json!(item.level),
            );
            if !item.computer_name.is_empty() {
                insert_attr(
                    &mut attrs,
                    "host.name",
                    serde_json::json!(item.computer_name),
                );
            }
            events.push(collector_event(CollectorEventArgs {
                id_prefix: "eventlog",
                timestamp: Some(item.timestamp),
                service_id: item.provider.clone(),
                severity: severity_from_level(&item.level),
                message: item.message,
                source_type: "windows_eventlog".into(),
                source_id: channel.to_string(),
                tags: vec![
                    "eventlog".into(),
                    "windows_eventlog".into(),
                    channel.to_ascii_lowercase(),
                    item.level.clone(),
                ],
                fingerprint: Some(format!("{channel}:{}", item.record_id)),
                host_id: (!item.computer_name.is_empty()).then_some(item.computer_name.clone()),
                kind: CollectorEventKind::Log,
                quality: "raw",
                structured_data: Some(structured_with_attributes(
                    attrs,
                    "windows_eventlog",
                    serde_json::json!({
                        "channel": channel,
                        "record_id": item.record_id,
                        "event_id": item.event_id,
                        "provider": item.provider,
                        "level": item.level,
                        "computer_name": item.computer_name,
                        "event_data": item.event_data,
                    }),
                )),
                raw_offset: Some(item.record_id as i64),
                trace_id: None,
                span_id: None,
                signal_kind: "log",
                deployment_environment: None,
                severity_text: Some(item.level),
            }));
        }
        if newest > last_record {
            store.set_collector_state(
                "windows_eventlog",
                &state_key,
                &newest.to_string(),
                &now_iso(),
            )?;
        }
    }
    Ok(events)
}

fn collect_windows_service_events(
    events_db: &Path,
    include_stopped: bool,
    include_automatic_stopped: bool,
    names: &HashSet<String>,
    exclude_names: &HashSet<String>,
) -> Result<Vec<NewEventRecord>> {
    if !cfg!(target_os = "windows") {
        return Ok(vec![]);
    }
    let store = EventsStore::open(events_db)?
        .context("event store not found for windows service collector")?;
    let output = Command::new("sc.exe")
        .args(["queryex", "state=", "all"])
        .output()
        .context("query windows services with sc.exe queryex")?;
    if !output.status.success() {
        anyhow::bail!(
            "sc.exe queryex state= all failed: {}",
            sc_output_text_like(&output)
        );
    }
    let mut snapshot = parse_windows_service_snapshot(&String::from_utf8_lossy(&output.stdout));
    enrich_windows_service_snapshots(&mut snapshot, exclude_names);
    let current_json =
        serde_json::to_string(&snapshot).context("serialize windows service snapshot")?;
    let previous = store
        .get_collector_state("windows_service", "snapshot")?
        .and_then(|value| {
            serde_json::from_str::<HashMap<String, WindowsServiceSnapshot>>(&value).ok()
        });
    store.set_collector_state("windows_service", "snapshot", &current_json, &now_iso())?;
    let mut events = Vec::new();
    for (service, current) in snapshot {
        let service_key = service.to_ascii_lowercase();
        if windows_service_excluded(&service_key, names, exclude_names) {
            continue;
        }
        let previous_snapshot = previous.as_ref().and_then(|items| items.get(&service));
        let previous_state = previous_snapshot
            .map(|item| item.state.clone())
            .unwrap_or_else(|| "unknown".into());
        let state = current.state.clone();
        if previous_state == state {
            continue;
        }
        let automatic_stopped = current.is_automatic() && !current.is_healthy_running();
        if previous_snapshot.is_some_and(|previous| previous == &current) && !automatic_stopped {
            continue;
        }
        if previous_snapshot.is_none() && !automatic_stopped {
            continue;
        }
        if automatic_stopped
            && previous_snapshot
                .is_some_and(|previous| previous.is_automatic() && !previous.is_healthy_running())
        {
            continue;
        }
        if !include_stopped
            && state == "stopped"
            && !(include_automatic_stopped && automatic_stopped)
        {
            continue;
        }
        let severity = if automatic_stopped {
            if is_benign_windows_updater(&service_key) {
                1
            } else {
                2
            }
        } else if matches!(
            state.as_str(),
            "stopped" | "stop_pending" | "paused" | "pause_pending"
        ) {
            2
        } else {
            1
        };
        let mut attrs = serde_json::Map::new();
        insert_attr(
            &mut attrs,
            "windows.service.name",
            serde_json::json!(service_key),
        );
        insert_attr(
            &mut attrs,
            "windows.service.state",
            serde_json::json!(state),
        );
        insert_attr(
            &mut attrs,
            "windows.service.previous_state",
            serde_json::json!(previous_state),
        );
        if let Some(start_type) = current.start_type.as_ref() {
            insert_attr(
                &mut attrs,
                "windows.service.start_type",
                serde_json::json!(start_type),
            );
        }
        if let Some(pid) = current.pid {
            insert_attr(&mut attrs, "process.pid", serde_json::json!(pid));
        }
        let message = format!(
            "windows service {service_key} transitioned {previous_state} -> {state} start_type={}",
            current.start_type.as_deref().unwrap_or("unknown")
        );
        events.push(collector_event(CollectorEventArgs {
            id_prefix: "service",
            timestamp: None,
            service_id: service_key.clone(),
            severity,
            message,
            source_type: "windows_service".into(),
            source_id: format!("service://{service_key}"),
            tags: vec![
                "service".into(),
                "windows_service".into(),
                state.clone(),
                if automatic_stopped {
                    "automatic_stopped".into()
                } else {
                    "state_change".into()
                },
            ],
            fingerprint: Some(format!("service:{service_key}:{state}")),
            host_id: None,
            kind: CollectorEventKind::StateChange,
            quality: "normalized",
            structured_data: Some(structured_with_attributes(
                attrs,
                "windows_service",
                serde_json::json!({
                    "service": current,
                    "previous_state": previous_state,
                    "automatic_stopped": automatic_stopped,
                }),
            )),
            raw_offset: None,
            trace_id: None,
            span_id: None,
            signal_kind: "log",
            deployment_environment: None,
            severity_text: Some(if severity >= 3 {
                "error".into()
            } else if severity == 2 {
                "warn".into()
            } else {
                "info".into()
            }),
        }));
    }
    Ok(events)
}

fn collect_docker_events(
    events_db: &Path,
    socket: &str,
    include_names: &HashSet<String>,
    include_labels: &HashSet<String>,
    exclude_names: &HashSet<String>,
    include_all: bool,
) -> Result<Vec<NewEventRecord>> {
    let store =
        EventsStore::open(events_db)?.context("event store not found for docker collector")?;
    let since = store
        .get_collector_state("docker", "since")?
        .unwrap_or_else(now_iso);
    let until = now_iso();
    let mut command = Command::new("docker");
    command
        .arg("events")
        .arg("--since")
        .arg(&since)
        .arg("--until")
        .arg(&until)
        .arg("--format")
        .arg("{{json .}}");
    if socket.starts_with("tcp://")
        || socket.starts_with("http://")
        || socket.starts_with("https://")
    {
        command.env("DOCKER_HOST", socket);
    }
    let output = command.output().context("run docker events")?;
    if !output.status.success() {
        anyhow::bail!("docker events failed: {}", sc_output_text_like(&output));
    }
    let mut events = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        let actor_attrs = value
            .get("Actor")
            .and_then(|actor| actor.get("Attributes"))
            .and_then(serde_json::Value::as_object)
            .cloned()
            .unwrap_or_default();
        let name = actor_attrs
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("docker")
            .to_string();
        let name_lower = name.to_ascii_lowercase();
        if exclude_names.contains(&name_lower) {
            continue;
        }
        let labels = actor_attrs
            .keys()
            .filter(|key| key.starts_with("label:"))
            .map(|key| key.trim_start_matches("label:").to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let include_name = include_names.is_empty() || include_names.contains(&name_lower);
        let include_label =
            include_labels.is_empty() || include_labels.iter().any(|label| labels.contains(label));
        if !include_all && !include_name && !include_label {
            continue;
        }
        let action = value
            .get("Action")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("event")
            .to_string();
        let timestamp = value
            .get("TimeNano")
            .and_then(serde_json::Value::as_i64)
            .and_then(epoch_nanos_to_iso)
            .unwrap_or_else(now_iso);
        let service_id = docker_service_id(&name, &actor_attrs);
        let severity = docker_action_severity(&action);
        let container_id = value
            .get("Actor")
            .and_then(|actor| actor.get("ID"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| value.get("id").and_then(serde_json::Value::as_str))
            .unwrap_or("docker")
            .to_string();
        let mut attrs = serde_json::Map::new();
        insert_attr(&mut attrs, "container.name", serde_json::json!(name));
        insert_attr(&mut attrs, "container.id", serde_json::json!(container_id));
        insert_attr(&mut attrs, "container.action", serde_json::json!(action));
        insert_attr(&mut attrs, "service.name", serde_json::json!(service_id));
        insert_scalar_json_attr(&mut attrs, "container.image.name", actor_attrs.get("image"));
        let mut tags = vec!["docker".into(), "container".into(), action.clone()];
        if matches!(action.as_str(), "oom" | "die" | "kill") {
            tags.push("service_instability".into());
        }
        if action.contains("health_status") {
            tags.push("health_check".into());
        }
        events.push(collector_event(CollectorEventArgs {
            id_prefix: "docker",
            timestamp: Some(timestamp.clone()),
            service_id,
            severity,
            message: format!("docker container {name} action={action}"),
            source_type: "docker".into(),
            source_id: container_id.clone(),
            tags,
            fingerprint: Some(format!(
                "docker:{}:{}",
                value
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&container_id),
                action
            )),
            host_id: None,
            kind: CollectorEventKind::StateChange,
            quality: "raw",
            structured_data: Some(structured_with_attributes(attrs, "docker", value)),
            raw_offset: None,
            trace_id: None,
            span_id: None,
            signal_kind: "log",
            deployment_environment: None,
            severity_text: Some(if severity >= 3 {
                "error".into()
            } else if severity == 2 {
                "warn".into()
            } else {
                "info".into()
            }),
        }));
    }
    store.set_collector_state("docker", "since", &until, &now_iso())?;
    Ok(events)
}

fn docker_service_id(name: &str, attrs: &serde_json::Map<String, serde_json::Value>) -> String {
    for key in [
        "label:com.docker.compose.service",
        "label:app.kubernetes.io/name",
        "label:app",
        "com.docker.compose.service",
        "app.kubernetes.io/name",
        "app",
    ] {
        if let Some(value) = attrs
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return value.to_string();
        }
    }
    name.trim_start_matches('/').to_string()
}

fn docker_action_severity(action: &str) -> i64 {
    let lower = action.to_ascii_lowercase();
    if matches!(lower.as_str(), "oom" | "die" | "kill")
        || lower.contains("health_status: unhealthy")
    {
        3
    } else if matches!(lower.as_str(), "restart" | "stop" | "pause") || lower.contains("unhealthy")
    {
        2
    } else {
        1
    }
}

fn collect_kubernetes_events(
    events_db: &Path,
    namespaces: &[String],
    all_namespaces: bool,
    label_selector: &str,
    limit: usize,
    include_pods: bool,
    include_events: bool,
) -> Result<Vec<NewEventRecord>> {
    let store =
        EventsStore::open(events_db)?.context("event store not found for kubernetes collector")?;
    let namespace_filter: HashSet<String> = namespaces
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect();
    let mut emitted = Vec::new();

    if include_events {
        let previous = store
            .get_collector_state("kubernetes", "events_last_ts")?
            .unwrap_or_default();
        let payload = kubectl_json(["get", "events"], all_namespaces, label_selector)?;
        let items = payload
            .get("items")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut newest = previous.clone();
        for item in items.into_iter().rev().take(limit) {
            let namespace = item
                .get("metadata")
                .and_then(|meta| meta.get("namespace"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !all_namespaces
                && !namespace_filter.is_empty()
                && !namespace_filter.contains(&namespace)
            {
                continue;
            }
            let ts = item
                .get("eventTime")
                .or_else(|| item.get("lastTimestamp"))
                .or_else(|| {
                    item.get("metadata")
                        .and_then(|meta| meta.get("creationTimestamp"))
                })
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            if previous.is_empty() {
                newest = newest.max(ts.clone());
                continue;
            }
            if ts <= previous {
                continue;
            }
            newest = newest.max(ts.clone());
            let involved = item
                .get("involvedObject")
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default();
            let object_name = involved
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("kubernetes");
            let service_id = kubernetes_workload_name(object_name);
            let reason = item
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("event");
            let event_type = item
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Normal");
            let note = item
                .get("note")
                .or_else(|| item.get("message"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("kubernetes event");
            let severity = kubernetes_event_severity(event_type, reason, note);
            let mut attrs = serde_json::Map::new();
            insert_attr(&mut attrs, "k8s.namespace", serde_json::json!(namespace));
            insert_attr(&mut attrs, "k8s.event.reason", serde_json::json!(reason));
            insert_attr(&mut attrs, "k8s.event.type", serde_json::json!(event_type));
            insert_attr(
                &mut attrs,
                "k8s.object.name",
                serde_json::json!(object_name),
            );
            insert_attr(&mut attrs, "service.name", serde_json::json!(service_id));
            emitted.push(collector_event(CollectorEventArgs {
                id_prefix: "k8s",
                timestamp: Some(if ts.is_empty() { now_iso() } else { ts.clone() }),
                service_id,
                severity,
                message: format!("kubernetes {reason}: {note}"),
                source_type: "kubernetes".into(),
                source_id: item
                    .get("metadata")
                    .and_then(|meta| meta.get("uid"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("kubernetes")
                    .to_string(),
                tags: vec![
                    "kubernetes".into(),
                    "event".into(),
                    reason.to_ascii_lowercase(),
                ],
                fingerprint: Some(format!(
                    "k8s-event:{}",
                    item.get("metadata")
                        .and_then(|meta| meta.get("uid"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                )),
                host_id: item
                    .get("source")
                    .and_then(|source| source.get("host"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .or_else(|| Some("cluster".into())),
                kind: CollectorEventKind::StateChange,
                quality: "raw",
                structured_data: Some(structured_with_attributes(attrs, "kubernetes", item)),
                raw_offset: None,
                trace_id: None,
                span_id: None,
                signal_kind: "log",
                deployment_environment: None,
                severity_text: Some(if severity >= 3 {
                    "error".into()
                } else if severity == 2 {
                    "warn".into()
                } else {
                    "info".into()
                }),
            }));
        }
        if !newest.is_empty() {
            store.set_collector_state("kubernetes", "events_last_ts", &newest, &now_iso())?;
        }
    }

    if include_pods {
        let payload = kubectl_json(["get", "pods"], all_namespaces, label_selector)?;
        let items = payload
            .get("items")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let previous = store
            .get_collector_state("kubernetes", "pod_snapshot")?
            .and_then(|value| serde_json::from_str::<HashMap<String, String>>(&value).ok())
            .unwrap_or_default();
        let mut current = HashMap::new();
        for item in items {
            let namespace = item
                .get("metadata")
                .and_then(|meta| meta.get("namespace"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if !all_namespaces
                && !namespace_filter.is_empty()
                && !namespace_filter.contains(&namespace.to_ascii_lowercase())
            {
                continue;
            }
            let name = item
                .get("metadata")
                .and_then(|meta| meta.get("name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("pod");
            let phase = item
                .get("status")
                .and_then(|status| status.get("phase"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Unknown")
                .to_string();
            let labels = item
                .get("metadata")
                .and_then(|meta| meta.get("labels"))
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default();
            let service_id = kubernetes_service_id_from_labels(&labels)
                .unwrap_or_else(|| kubernetes_workload_name(name));
            let ready = kubernetes_pod_ready(&item);
            let restart_count = kubernetes_restart_count(&item);
            let oom_killed = kubernetes_oom_killed(&item);
            let key = format!("{namespace}/{name}");
            let state_signature =
                format!("{phase}|ready={ready}|restarts={restart_count}|oom={oom_killed}");
            let noteworthy =
                !phase.eq_ignore_ascii_case("running") || !ready || restart_count > 0 || oom_killed;
            if let Some(previous_state) = previous.get(&key) {
                if previous_state != &state_signature && noteworthy {
                    let severity = if phase.eq_ignore_ascii_case("failed") || oom_killed {
                        3
                    } else if !ready || restart_count > 0 {
                        2
                    } else {
                        1
                    };
                    let mut attrs = serde_json::Map::new();
                    insert_attr(&mut attrs, "k8s.namespace", serde_json::json!(namespace));
                    insert_attr(&mut attrs, "k8s.pod.name", serde_json::json!(name));
                    insert_attr(&mut attrs, "k8s.pod.phase", serde_json::json!(phase));
                    insert_attr(&mut attrs, "k8s.pod.ready", serde_json::json!(ready));
                    insert_attr(
                        &mut attrs,
                        "k8s.pod.restart_count",
                        serde_json::json!(restart_count),
                    );
                    insert_attr(
                        &mut attrs,
                        "k8s.pod.oom_killed",
                        serde_json::json!(oom_killed),
                    );
                    insert_attr(&mut attrs, "service.name", serde_json::json!(service_id));
                    let node = item
                        .get("spec")
                        .and_then(|spec| spec.get("nodeName"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(namespace);
                    emitted.push(collector_event(CollectorEventArgs {
                        id_prefix: "k8s-pod",
                        timestamp: None,
                        service_id: service_id.clone(),
                        severity,
                        message: format!(
                            "kubernetes pod {key} state changed previous={previous_state} current={state_signature}"
                        ),
                        source_type: "kubernetes".into(),
                        source_id: key.clone(),
                        tags: vec![
                            "kubernetes".into(),
                            "pod".into(),
                            phase.clone(),
                            if oom_killed {
                                "oom".into()
                            } else if restart_count > 0 {
                                "restart".into()
                            } else if !ready {
                                "not_ready".into()
                            } else {
                                "state_change".into()
                            },
                        ],
                        fingerprint: Some(format!("k8s-pod:{key}:{state_signature}")),
                        host_id: Some(node.to_string()),
                        kind: CollectorEventKind::StateChange,
                        quality: "normalized",
                        structured_data: Some(structured_with_attributes(
                            attrs,
                            "kubernetes",
                            serde_json::json!({
                                "kind": "Pod",
                                "namespace": namespace,
                                "name": name,
                                "service": service_id,
                                "phase": phase,
                                "ready": ready,
                                "restart_count": restart_count,
                                "oom_killed": oom_killed,
                                "previous_state": previous_state,
                                "state": state_signature,
                                "pod": item,
                            }),
                        )),
                        raw_offset: None,
                        trace_id: None,
                        span_id: None,
                        signal_kind: "log",
                        deployment_environment: None,
                        severity_text: Some(if severity >= 3 {
                            "error".into()
                        } else if severity == 2 {
                            "warn".into()
                        } else {
                            "info".into()
                        }),
                    }));
                }
            }
            current.insert(key, state_signature);
        }
        store.set_collector_state(
            "kubernetes",
            "pod_snapshot",
            &serde_json::to_string(&current).context("serialize kubernetes pod snapshot")?,
            &now_iso(),
        )?;
    }

    Ok(emitted)
}

fn kubernetes_workload_name(name: &str) -> String {
    let parts = name.split('-').collect::<Vec<_>>();
    if parts.len() >= 3
        && parts.last().is_some_and(|tail| {
            tail.len() <= 6 && tail.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
    {
        return parts[..parts.len() - 2].join("-");
    }
    if parts.len() >= 2
        && parts.last().is_some_and(|tail| {
            tail.len() <= 10 && tail.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
    {
        return parts[..parts.len() - 1].join("-");
    }
    name.to_string()
}

fn kubernetes_service_id_from_labels(
    labels: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    for key in ["app.kubernetes.io/name", "app", "k8s-app"] {
        if let Some(value) = labels
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }
    None
}

fn kubernetes_event_severity(event_type: &str, reason: &str, note: &str) -> i64 {
    let text = format!("{event_type} {reason} {note}").to_ascii_lowercase();
    if text.contains("oom")
        || text.contains("failed")
        || text.contains("backoff")
        || text.contains("error")
        || text.contains("unhealthy")
    {
        3
    } else if event_type.eq_ignore_ascii_case("warning")
        || text.contains("warn")
        || text.contains("notready")
    {
        2
    } else {
        1
    }
}

fn kubernetes_restart_count(pod: &serde_json::Value) -> i64 {
    pod.get("status")
        .and_then(|status| status.get("containerStatuses"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|status| {
            status
                .get("restartCount")
                .and_then(serde_json::Value::as_i64)
        })
        .sum()
}

fn kubernetes_pod_ready(pod: &serde_json::Value) -> bool {
    pod.get("status")
        .and_then(|status| status.get("conditions"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|condition| {
            condition
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value == "Ready")
        })
        .and_then(|condition| condition.get("status").and_then(serde_json::Value::as_str))
        .is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn kubernetes_oom_killed(pod: &serde_json::Value) -> bool {
    pod.get("status")
        .and_then(|status| status.get("containerStatuses"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .any(|status| {
            ["state", "lastState"].iter().any(|key| {
                status
                    .get(*key)
                    .and_then(|state| state.get("terminated"))
                    .and_then(|terminated| terminated.get("reason"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|reason| reason == "OOMKilled")
            })
        })
}

fn resolve_file_targets(targets: &[FileTailTarget]) -> Result<Vec<FileTailTarget>> {
    let mut resolved = Vec::new();
    for target in targets {
        let path_text = target.path.to_string_lossy();
        if path_text.contains('*') || path_text.contains('?') {
            let parent = target
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            let pattern = target
                .path
                .file_name()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "*".to_string());
            if !parent.exists() {
                continue;
            }
            for entry in std::fs::read_dir(&parent)
                .with_context(|| format!("read glob parent {}", parent.display()))?
            {
                let entry = entry?;
                let file_name = entry.file_name().to_string_lossy().to_string();
                if wildcard_match(&pattern, &file_name) {
                    let mut cloned = target.clone();
                    cloned.path = entry.path();
                    resolved.push(cloned);
                }
            }
        } else {
            resolved.push(target.clone());
        }
    }
    Ok(resolved)
}

fn file_event_record(target: &FileTailTarget, message: &str, raw_offset: u64) -> NewEventRecord {
    let parsed = parse_collector_log_line(message);
    let service_id = target
        .service_id
        .clone()
        .or(parsed.service_id.clone())
        .or_else(|| {
            target.service_id_from_filename.then(|| {
                target
                    .path
                    .file_stem()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| "file".to_string())
            })
        })
        .unwrap_or_else(|| {
            target
                .path
                .file_stem()
                .map(|value| value.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string())
        });
    let mut attrs = parsed.attributes;
    insert_attr(
        &mut attrs,
        "log.file.path",
        serde_json::json!(target.path.display().to_string()),
    );
    insert_attr(&mut attrs, "log.raw_offset", serde_json::json!(raw_offset));
    insert_attr(&mut attrs, "service.name", serde_json::json!(service_id));
    collector_event(CollectorEventArgs {
        id_prefix: "file",
        timestamp: parsed.timestamp,
        service_id,
        severity: parsed.severity,
        message: parsed.message,
        source_type: "file".into(),
        source_id: target.path.display().to_string(),
        tags: vec!["file".into()],
        fingerprint: Some(semantic_fingerprint(
            "file",
            &target.path.display().to_string(),
            "file",
            message,
            parsed.trace_id.as_deref(),
        )),
        host_id: None,
        kind: CollectorEventKind::Log,
        quality: if parsed.parsed_json.is_some() {
            "normalized"
        } else {
            "raw"
        },
        structured_data: Some(structured_with_attributes(
            attrs,
            "file",
            serde_json::json!({
                "path": target.path.display().to_string(),
                "raw_offset": raw_offset,
                "multiline_pattern": target.multiline_pattern,
                "raw": message,
                "parsed": parsed.parsed_json,
            }),
        )),
        raw_offset: Some(raw_offset as i64),
        trace_id: parsed.trace_id,
        span_id: parsed.span_id,
        signal_kind: "log",
        deployment_environment: parsed.deployment_environment,
        severity_text: parsed.severity_text,
    })
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    wildcard_match_bytes(pattern.as_bytes(), text.as_bytes())
}

fn windows_service_excluded(
    service_key: &str,
    names: &HashSet<String>,
    exclude_names: &HashSet<String>,
) -> bool {
    if !names.is_empty() && !names.contains(service_key) {
        return true;
    }
    exclude_names
        .iter()
        .any(|pattern| wildcard_match(pattern, service_key))
}

fn is_benign_windows_updater(service_key: &str) -> bool {
    service_key.contains("updater")
        || service_key.starts_with("edgeupdate")
        || service_key == "sppsvc"
        || service_key.starts_with("gupdate")
}

pub fn active_collector_ids(config: &TomlValue) -> HashSet<String> {
    collector_specs(config)
        .into_iter()
        .map(|spec| match spec {
            CollectorSpec::HostMetrics { .. } => "host_metrics",
            CollectorSpec::Process { .. } => "process",
            CollectorSpec::File { .. } => "file",
            CollectorSpec::LinuxSyslog { .. } => "linux_syslog",
            CollectorSpec::Journald { .. } => "journald",
            CollectorSpec::WindowsEventLog { .. } => "windows_eventlog",
            CollectorSpec::WindowsService { .. } => "windows_service",
            CollectorSpec::Docker { .. } => "docker",
            CollectorSpec::Kubernetes { .. } => "kubernetes",
            CollectorSpec::AppIngest | CollectorSpec::AppStandalone { .. } => "app",
        })
        .map(str::to_string)
        .collect()
}

fn wildcard_match_bytes(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' => {
            wildcard_match_bytes(&pattern[1..], text)
                || (!text.is_empty() && wildcard_match_bytes(pattern, &text[1..]))
        }
        b'?' => !text.is_empty() && wildcard_match_bytes(&pattern[1..], &text[1..]),
        ch => !text.is_empty() && ch == text[0] && wildcard_match_bytes(&pattern[1..], &text[1..]),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WindowsServiceSnapshot {
    name: String,
    display_name: String,
    state: String,
    start_type: Option<String>,
    pid: Option<u32>,
    service_start_name: Option<String>,
    binary_path: Option<String>,
}

impl WindowsServiceSnapshot {
    fn new(name: String) -> Self {
        Self {
            display_name: name.clone(),
            name,
            state: "unknown".into(),
            start_type: None,
            pid: None,
            service_start_name: None,
            binary_path: None,
        }
    }

    fn is_automatic(&self) -> bool {
        self.start_type
            .as_deref()
            .is_some_and(|value| value.contains("auto") || value == "automatic")
    }

    fn is_healthy_running(&self) -> bool {
        matches!(self.state.as_str(), "running" | "paused")
    }
}

fn parse_windows_service_snapshot(text: &str) -> HashMap<String, WindowsServiceSnapshot> {
    let mut current = None::<WindowsServiceSnapshot>;
    let mut out = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some((_, tail)) = line.split_once("SERVICE_NAME:") {
            if let Some(service) = current.take() {
                out.insert(service.name.clone(), service);
            }
            current = Some(WindowsServiceSnapshot::new(tail.trim().to_string()));
        } else if let Some((_, tail)) = line.split_once("DISPLAY_NAME:") {
            if let Some(service) = current.as_mut() {
                let value = tail.trim();
                if !value.is_empty() {
                    service.display_name = value.to_string();
                }
            }
        } else if let Some((_, tail)) = line.split_once("STATE") {
            if let Some((_, value)) = tail.split_once(':') {
                if let (Some(service), Some(state)) =
                    (current.as_mut(), parse_state_value(value.trim()))
                {
                    service.state = state;
                }
            }
        } else if let Some((_, tail)) = line.split_once("PID") {
            if let Some((_, value)) = tail.split_once(':') {
                if let Some(service) = current.as_mut() {
                    service.pid = value.trim().parse::<u32>().ok().filter(|pid| *pid > 0);
                }
            }
        }
    }
    if let Some(service) = current {
        out.insert(service.name.clone(), service);
    }
    out
}

fn enrich_windows_service_snapshots(
    snapshot: &mut HashMap<String, WindowsServiceSnapshot>,
    exclude_names: &HashSet<String>,
) {
    for service in snapshot.values_mut() {
        let service_key = service.name.to_ascii_lowercase();
        if windows_service_excluded(&service_key, &HashSet::new(), exclude_names) {
            continue;
        }
        let output = Command::new("sc.exe").args(["qc", &service.name]).output();
        let Ok(output) = output else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        apply_windows_service_qc(service, &String::from_utf8_lossy(&output.stdout));
    }
}

fn apply_windows_service_qc(service: &mut WindowsServiceSnapshot, text: &str) {
    for raw in text.lines() {
        let line = raw.trim();
        if let Some((_, value)) = line.split_once("DISPLAY_NAME") {
            if let Some((_, display_name)) = value.split_once(':') {
                let display_name = display_name.trim();
                if !display_name.is_empty() {
                    service.display_name = display_name.to_string();
                }
            }
        } else if let Some((_, value)) = line.split_once("START_TYPE") {
            if let Some((_, start_type)) = value.split_once(':') {
                service.start_type = parse_windows_start_type(start_type.trim());
            }
        } else if let Some((_, value)) = line.split_once("BINARY_PATH_NAME") {
            if let Some((_, binary_path)) = value.split_once(':') {
                let binary_path = binary_path.trim();
                if !binary_path.is_empty() {
                    service.binary_path = Some(binary_path.to_string());
                }
            }
        } else if let Some((_, value)) = line.split_once("SERVICE_START_NAME") {
            if let Some((_, start_name)) = value.split_once(':') {
                let start_name = start_name.trim();
                if !start_name.is_empty() {
                    service.service_start_name = Some(start_name.to_string());
                }
            }
        }
    }
}

fn parse_windows_start_type(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("auto_start") || lower.contains("automatic") {
        Some("automatic".into())
    } else if lower.contains("demand_start") || lower.contains("manual") {
        Some("manual".into())
    } else if lower.contains("disabled") {
        Some("disabled".into())
    } else if lower.trim().is_empty() {
        None
    } else {
        Some(
            lower
                .split_whitespace()
                .last()
                .unwrap_or(lower.trim())
                .to_string(),
        )
    }
}

fn parse_state_value(raw: &str) -> Option<String> {
    let mut parts = raw.split_whitespace();
    let first = parts.next()?;
    let second = parts.next();
    let value = if first.chars().all(|ch| ch.is_ascii_digit()) {
        second.unwrap_or(first)
    } else {
        first
    };
    Some(value.to_ascii_lowercase())
}

#[derive(Debug)]
struct WindowsEventRecord {
    record_id: u64,
    timestamp: String,
    provider: String,
    event_id: String,
    level: String,
    message: String,
    computer_name: String,
    event_data: Vec<String>,
}

fn parse_wevtutil_xml_events(text: &str, channel: &str) -> Vec<WindowsEventRecord> {
    text.split("<Event ")
        .filter(|chunk| chunk.contains("<System>") || chunk.contains("<System "))
        .filter_map(|chunk| parse_wevtutil_xml_event(chunk, channel))
        .collect()
}

fn parse_wevtutil_xml_event(chunk: &str, channel: &str) -> Option<WindowsEventRecord> {
    let record_id = xml_tag_text(chunk, "EventRecordID")?.parse::<u64>().ok()?;
    let provider = xml_attr_value(chunk, "Provider", "Name").unwrap_or_else(|| channel.to_string());
    let timestamp = xml_attr_value(chunk, "TimeCreated", "SystemTime").unwrap_or_else(now_iso);
    let level = xml_tag_text(chunk, "Level").unwrap_or_else(|| "4".to_string());
    let event_id = xml_tag_text(chunk, "EventID").unwrap_or_default();
    let computer_name = xml_tag_text(chunk, "Computer").unwrap_or_default();
    let data = xml_all_tag_text(chunk, "Data");
    let message = if data.is_empty() {
        format!("{provider} event {event_id} in {channel}")
    } else {
        format!(
            "{provider} event {event_id}: {}",
            data.iter()
                .filter(|item| !item.trim().is_empty())
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        )
    };
    Some(WindowsEventRecord {
        record_id,
        timestamp,
        provider,
        event_id,
        level: windows_event_level_name(&level).to_string(),
        message,
        computer_name,
        event_data: data,
    })
}

fn xml_tag_text(text: &str, tag: &str) -> Option<String> {
    let start = text.find(&format!("<{tag}"))?;
    let rest = &text[start..];
    let value_start = rest.find('>')? + 1;
    let after_start = &rest[value_start..];
    let value_end = after_start.find(&format!("</{tag}>"))?;
    Some(xml_unescape(after_start[..value_end].trim()))
}

fn xml_all_tag_text(text: &str, tag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(&format!("<{tag}")) {
        rest = &rest[start..];
        let Some(value_start) = rest.find('>').map(|idx| idx + 1) else {
            break;
        };
        let after_start = &rest[value_start..];
        let Some(value_end) = after_start.find(&format!("</{tag}>")) else {
            break;
        };
        out.push(xml_unescape(after_start[..value_end].trim()));
        rest = &after_start[value_end + tag.len() + 3..];
    }
    out
}

fn xml_attr_value(text: &str, tag: &str, attr: &str) -> Option<String> {
    let start = text.find(&format!("<{tag}"))?;
    let rest = &text[start..];
    let tag_end = rest.find('>')?;
    let tag_text = &rest[..tag_end];
    for quote in ['\'', '"'] {
        let needle = format!("{attr}={quote}");
        if let Some(attr_start) = tag_text.find(&needle) {
            let value = &tag_text[attr_start + needle.len()..];
            let attr_end = value.find(quote)?;
            return Some(xml_unescape(&value[..attr_end]));
        }
    }
    None
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .trim_matches(char::from(0))
        .trim()
        .to_string()
}

fn windows_event_level_name(value: &str) -> &'static str {
    match value.trim() {
        "1" => "critical",
        "2" => "error",
        "3" => "warn",
        "4" | "0" => "info",
        "5" => "debug",
        _ => "info",
    }
}

fn kubectl_json(
    prefix_args: [&str; 2],
    all_namespaces: bool,
    label_selector: &str,
) -> Result<serde_json::Value> {
    let mut command = Command::new("kubectl");
    command.args(prefix_args);
    if all_namespaces {
        command.arg("-A");
    }
    if !label_selector.trim().is_empty() {
        command.arg("-l").arg(label_selector);
    }
    command.arg("-o").arg("json");
    let output = command.output().context("run kubectl collector command")?;
    if !output.status.success() {
        anyhow::bail!("kubectl failed: {}", sc_output_text_like(&output));
    }
    serde_json::from_slice(&output.stdout).context("parse kubectl json")
}

fn normalized_mount_path(mount_path: &str) -> String {
    let trimmed = mount_path.trim();
    if trimmed.is_empty() {
        "/api/ingest".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn epoch_micros_to_iso(value: &str) -> Option<String> {
    let micros = value.parse::<i128>().ok()?;
    let seconds = micros / 1_000_000;
    let remainder = micros % 1_000_000;
    let datetime = OffsetDateTime::from_unix_timestamp(seconds as i64).ok()?
        + time::Duration::microseconds(remainder as i64);
    datetime
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

fn epoch_nanos_to_iso(value: i64) -> Option<String> {
    let seconds = value / 1_000_000_000;
    let nanos = value % 1_000_000_000;
    let datetime =
        OffsetDateTime::from_unix_timestamp(seconds).ok()? + time::Duration::nanoseconds(nanos);
    datetime
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

fn severity_from_priority(priority: i64) -> i64 {
    match priority {
        0..=2 => 4,
        3 => 3,
        4 => 2,
        _ => 1,
    }
}

fn severity_from_text(message: &str) -> i64 {
    severity_from_level(
        if message.to_ascii_lowercase().contains("critical")
            || message.to_ascii_lowercase().contains("fatal")
            || message.to_ascii_lowercase().contains("panic")
        {
            "critical"
        } else if message.to_ascii_lowercase().contains("error")
            || message.to_ascii_lowercase().contains("fail")
        {
            "error"
        } else if message.to_ascii_lowercase().contains("warn")
            || message.to_ascii_lowercase().contains("degraded")
        {
            "warn"
        } else {
            "info"
        },
    )
}

fn sc_output_text_like(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => "(no output)".to_string(),
    }
}

fn persist_events(
    events_db: &Path,
    config: &TomlValue,
    events: &[NewEventRecord],
) -> Result<IngestBatchResult> {
    let mut store = EventsStore::open(events_db)?.context("event store not found")?;
    store.insert_batch_governed(events, &ingest_governance(config))
}

fn persist_events_and_reconcile(
    events_db: &Path,
    incidents_db: &Path,
    config: &TomlValue,
    events: &[NewEventRecord],
) -> Result<IngestBatchResult> {
    adjust_queue_depth(1);
    let result = persist_events(events_db, config, events);
    adjust_queue_depth(-1);
    let result = result?;
    if result.inserted > 0 {
        reconcile_new_events(events_db, incidents_db, config, &result.inserted_event_ids)?;
    }
    Ok(result)
}

fn next_event_id(prefix: &str) -> String {
    let seq = NEXT_EVENT_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{nanos}-{seq}")
}

fn severity_from_level(level: &str) -> i64 {
    match level.trim().to_ascii_lowercase().as_str() {
        "critical" | "fatal" | "panic" => 4,
        "error" => 3,
        "warn" | "warning" => 2,
        "debug" | "trace" => 0,
        _ => 1,
    }
}

fn severity_from_config(value: Option<&TomlValue>, default_level: &str) -> i64 {
    value
        .and_then(|item| {
            item.as_integer()
                .or_else(|| item.as_str().map(severity_from_level))
        })
        .unwrap_or_else(|| severity_from_level(default_level))
}

fn governance_rules(config: &TomlValue, key: &str) -> Vec<GovernanceRule> {
    config
        .get("noise_filter")
        .and_then(|noise| noise.get(key))
        .and_then(TomlValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(TomlValue::as_table)
        .map(|rule| GovernanceRule {
            pattern: rule
                .get("pattern")
                .and_then(TomlValue::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase(),
            service_id: rule
                .get("service_id")
                .and_then(TomlValue::as_str)
                .map(str::to_string),
            severity_min: Some(severity_from_config(rule.get("severity_min"), "info"))
                .filter(|_| rule.contains_key("severity_min")),
            severity_max: Some(severity_from_config(rule.get("severity_max"), "critical"))
                .filter(|_| rule.contains_key("severity_max")),
            tags: rule
                .get("tags")
                .and_then(TomlValue::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(TomlValue::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            reason: rule
                .get("reason")
                .and_then(TomlValue::as_str)
                .map(str::to_string),
        })
        .collect()
}

fn ingest_governance(config: &TomlValue) -> IngestGovernance {
    let dedup = config.get("deduplication").and_then(TomlValue::as_table);
    let noise = config.get("noise_filter").and_then(TomlValue::as_table);
    IngestGovernance {
        dedup_enabled: dedup
            .and_then(|table| table.get("enabled"))
            .and_then(TomlValue::as_bool)
            .unwrap_or(true),
        dedup_window_seconds: dedup
            .and_then(|table| table.get("window_seconds"))
            .and_then(TomlValue::as_integer)
            .unwrap_or(60),
        max_tracked_fingerprints: dedup
            .and_then(|table| table.get("max_tracked_fingerprints"))
            .and_then(TomlValue::as_integer)
            .unwrap_or(10_000)
            .max(1) as usize,
        severity_escalation_splits: dedup
            .and_then(|table| table.get("severity_escalation_splits"))
            .and_then(TomlValue::as_bool)
            .unwrap_or(true),
        noise_enabled: noise
            .and_then(|table| table.get("enabled"))
            .and_then(TomlValue::as_bool)
            .unwrap_or(true),
        blocklist_enabled: noise
            .and_then(|table| table.get("blocklist_enabled"))
            .and_then(TomlValue::as_bool)
            .unwrap_or(true),
        allowlist_enabled: noise
            .and_then(|table| table.get("allowlist_enabled"))
            .and_then(TomlValue::as_bool)
            .unwrap_or(true),
        registry_enabled: noise
            .and_then(|table| table.get("registry_enabled"))
            .and_then(TomlValue::as_bool)
            .unwrap_or(true),
        high_rate_threshold_per_minute: noise
            .and_then(|table| table.get("high_rate_threshold_per_minute"))
            .and_then(TomlValue::as_integer)
            .unwrap_or(100),
        always_keep_severity: noise
            .and_then(|table| table.get("always_keep_severity"))
            .map(|value| severity_from_config(Some(value), "error"))
            .unwrap_or_else(|| severity_from_level("error")),
        blocklist: governance_rules(config, "blocklist"),
        allowlist: governance_rules(config, "allowlist"),
        indexed_attribute_keys: inferra_config::observability_indexed_attribute_keys(config),
    }
}

fn normalize_hex_trace_id_from_value(value: &serde_json::Value) -> Option<String> {
    let s = value.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    let hex: String = s
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    (hex.len() == 32).then_some(hex)
}

fn normalize_hex_span_id_from_value(value: &serde_json::Value) -> Option<String> {
    let s = value.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    let hex: String = s
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    (hex.len() == 16).then_some(hex)
}

fn payload_traceparent(payload: &serde_json::Value) -> Option<&str> {
    let object = payload.as_object()?;
    find_case_insensitive_json_value(object, "traceparent")
        .and_then(|value| value.as_str())
        .or_else(|| {
            find_case_insensitive_json_value(object, "headers")
                .and_then(|value| value.as_object())
                .and_then(|headers| find_case_insensitive_json_value(headers, "traceparent"))
                .and_then(|value| value.as_str())
        })
}

fn payload_attribute_value<'a>(
    payload: &'a serde_json::Value,
    key: &str,
) -> Option<&'a serde_json::Value> {
    payload.as_object().and_then(|object| {
        find_case_insensitive_json_value(object, key).or_else(|| {
            find_case_insensitive_json_value(object, "attributes")
                .and_then(|value| value.as_object())
                .and_then(|attrs| find_case_insensitive_json_value(attrs, key))
        })
    })
}

fn find_case_insensitive_json_value<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<&'a serde_json::Value> {
    map.iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| value)
}

fn parse_w3c_traceparent(raw: &str) -> Option<(String, String)> {
    let parts = raw.trim().split('-').collect::<Vec<_>>();
    if parts.len() != 4 {
        return None;
    }
    let [version, trace_id, span_id, flags] = <[&str; 4]>::try_from(parts).ok()?;
    if version.len() != 2
        || trace_id.len() != 32
        || span_id.len() != 16
        || flags.len() != 2
        || !version.chars().all(|ch| ch.is_ascii_hexdigit())
        || !trace_id.chars().all(|ch| ch.is_ascii_hexdigit())
        || !span_id.chars().all(|ch| ch.is_ascii_hexdigit())
        || !flags.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return None;
    }
    let trace_id = trace_id.to_ascii_lowercase();
    let span_id = span_id.to_ascii_lowercase();
    if trace_id.chars().all(|ch| ch == '0') || span_id.chars().all(|ch| ch == '0') {
        return None;
    }
    Some((trace_id, span_id))
}

fn semantic_fingerprint(
    prefix: &str,
    service: &str,
    source_type: &str,
    message: &str,
    trace_id: Option<&str>,
) -> String {
    let mut out = format!(
        "{prefix}:{}:{}:{}",
        service.trim().to_ascii_lowercase(),
        source_type.trim().to_ascii_lowercase(),
        message
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == ' ' {
                    ch.to_ascii_lowercase()
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .take(18)
            .collect::<Vec<_>>()
            .join(" ")
    );
    if let Some(t) = trace_id.map(str::trim).filter(|t| !t.is_empty()) {
        out.push_str("::tr:");
        out.push_str(t);
    }
    out
}

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

fn adjust_queue_depth(delta: i64) {
    if delta >= 0 {
        GLOBAL_QUEUE_DEPTH.fetch_add(delta, Ordering::Relaxed);
        return;
    }
    let previous = GLOBAL_QUEUE_DEPTH.fetch_sub(-delta, Ordering::Relaxed);
    if previous <= -delta {
        GLOBAL_QUEUE_DEPTH.store(0, Ordering::Relaxed);
    }
}

fn epoch_millis() -> i128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i128
}

fn lag_seconds_for(timestamp: &str) -> Option<f64> {
    let observed =
        OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339).ok()?;
    let now = OffsetDateTime::now_utc();
    let lag = (now - observed).whole_milliseconds() as f64 / 1000.0;
    Some(lag.max(0.0))
}

#[cfg(test)]
mod collector_unit_tests;
