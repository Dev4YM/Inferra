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

static NEXT_EVENT_ID: AtomicU64 = AtomicU64::new(1);

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
    pub lag_seconds: Option<f64>,
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
    queue_depth: AtomicI64,
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
        names: HashSet<String>,
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
            let mut guard = self.inner.stop_sender.lock().expect("stop sender lock");
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
                    names,
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
                            names,
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
        let mut guard = self.inner.handles.lock().expect("handles lock");
        *guard = handles;
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        if let Some(sender) = self
            .inner
            .stop_sender
            .lock()
            .expect("stop sender lock")
            .take()
        {
            let _ = sender.send(true);
        }
        let handles = {
            let mut guard = self.inner.handles.lock().expect("handles lock");
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
        self.inner.queue_depth.store(0, Ordering::Relaxed);
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
        self.inner.queue_depth.load(Ordering::Relaxed)
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

    pub async fn ingest_app_event(
        &self,
        events_db: &Path,
        incidents_db: &Path,
        config: &TomlValue,
        payload: &serde_json::Value,
    ) -> Result<String> {
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
            .and_then(|value| value.as_str())
            .unwrap_or("ingested application event")
            .to_string();
        let level = payload
            .get("level")
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
        let event_id = next_event_id("app");
        let fingerprint = semantic_fingerprint("app", &service_id, "app_http", &message);
        let mut store =
            EventsStore::open(events_db)?.context("event store not found for app ingest")?;
        let result = store.insert_batch_governed(
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
            }],
            &ingest_governance(config),
        )?;
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
        Ok(event_id)
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
        row.events_per_second = 0.0;
        if last_event_at.is_some() {
            row.last_event_at = last_event_at;
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
                    names: string_array(table.get("names"))
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
        let sample = collect_host_sample();
        match sample {
            Ok(sample) => {
                if let Some(event) = state.update_and_build_event(
                    &sample,
                    warn_cpu_percent,
                    warn_memory_percent,
                    warn_disk_percent,
                ) {
                    match persist_events_and_reconcile(&events_db, &incidents_db, &config, &[event])
                    {
                        Ok(result) => {
                            runtime
                                .bump_success(
                                    collector_id,
                                    source_type,
                                    result.inserted as u64,
                                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                                    Some(observed_at),
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .record_error(collector_id, source_type, &error.to_string())
                                .await;
                        }
                    }
                } else {
                    runtime
                        .upsert_status(CollectorRuntimeRow {
                            collector_id: collector_id.into(),
                            status: "running".into(),
                            source_type: source_type.into(),
                            is_running: true,
                            ..Default::default()
                        })
                        .await;
                }
            }
            Err(error) => {
                runtime
                    .record_error(collector_id, source_type, &error.to_string())
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
        match collect_process_events(
            top_n,
            min_cpu_percent,
            min_memory_mb,
            &watch_processes,
            &watch_pids,
            &mut seen_hot,
        ) {
            Ok(events) => {
                if !events.is_empty() {
                    match persist_events_and_reconcile(&events_db, &incidents_db, &config, &events)
                    {
                        Ok(result) => {
                            runtime
                                .bump_success(
                                    collector_id,
                                    source_type,
                                    result.inserted as u64,
                                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                                    Some(observed_at),
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .record_error(collector_id, source_type, &error.to_string())
                                .await;
                        }
                    }
                }
            }
            Err(error) => {
                runtime
                    .record_error(collector_id, source_type, &error.to_string())
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
                if !events.is_empty() {
                    match persist_events_and_reconcile(&events_db, &incidents_db, &config, &events)
                    {
                        Ok(result) => {
                            runtime
                                .bump_success(
                                    collector_id,
                                    source_type,
                                    result.inserted as u64,
                                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                                    Some(observed_at),
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .record_error(collector_id, source_type, &error.to_string())
                                .await;
                        }
                    }
                }
            }
            Err(error) => {
                runtime
                    .record_error(collector_id, source_type, &error.to_string())
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
    let mut started = false;
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        match collect_file_events(&events_db, &targets, start_at_end && !started) {
            Ok(events) => {
                started = true;
                if !events.is_empty() {
                    match persist_events_and_reconcile(&events_db, &incidents_db, &config, &events)
                    {
                        Ok(result) => {
                            runtime
                                .bump_success(
                                    collector_id,
                                    source_type,
                                    result.inserted as u64,
                                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                                    Some(observed_at),
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .record_error(collector_id, source_type, &error.to_string())
                                .await;
                        }
                    }
                }
            }
            Err(error) => {
                runtime
                    .record_error(collector_id, source_type, &error.to_string())
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
                if !events.is_empty() {
                    match persist_events_and_reconcile(&events_db, &incidents_db, &config, &events)
                    {
                        Ok(result) => {
                            runtime
                                .bump_success(
                                    collector_id,
                                    source_type,
                                    result.inserted as u64,
                                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                                    Some(observed_at),
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .record_error(collector_id, source_type, &error.to_string())
                                .await;
                        }
                    }
                }
            }
            Err(error) => {
                runtime
                    .record_error(collector_id, source_type, &error.to_string())
                    .await
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
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        match collect_windows_eventlog_events(&events_db, &channels) {
            Ok(events) => {
                if !events.is_empty() {
                    match persist_events_and_reconcile(&events_db, &incidents_db, &config, &events)
                    {
                        Ok(result) => {
                            runtime
                                .bump_success(
                                    collector_id,
                                    source_type,
                                    result.inserted as u64,
                                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                                    Some(observed_at),
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .record_error(collector_id, source_type, &error.to_string())
                                .await;
                        }
                    }
                }
            }
            Err(error) => {
                runtime
                    .record_error(collector_id, source_type, &error.to_string())
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
    names: HashSet<String>,
) {
    let collector_id = "windows_service";
    let source_type = "service";
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let observed_at = now_iso();
        match collect_windows_service_events(&events_db, include_stopped, &names) {
            Ok(events) => {
                if !events.is_empty() {
                    match persist_events_and_reconcile(&events_db, &incidents_db, &config, &events)
                    {
                        Ok(result) => {
                            runtime
                                .bump_success(
                                    collector_id,
                                    source_type,
                                    result.inserted as u64,
                                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                                    Some(observed_at),
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .record_error(collector_id, source_type, &error.to_string())
                                .await;
                        }
                    }
                }
            }
            Err(error) => {
                runtime
                    .record_error(collector_id, source_type, &error.to_string())
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
                if !events.is_empty() {
                    match persist_events_and_reconcile(&events_db, &incidents_db, &config, &events)
                    {
                        Ok(result) => {
                            runtime
                                .bump_success(
                                    collector_id,
                                    source_type,
                                    result.inserted as u64,
                                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                                    Some(observed_at),
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .record_error(collector_id, source_type, &error.to_string())
                                .await;
                        }
                    }
                }
            }
            Err(error) => {
                runtime
                    .record_error(collector_id, source_type, &error.to_string())
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
                if !events.is_empty() {
                    match persist_events_and_reconcile(&events_db, &incidents_db, &config, &events)
                    {
                        Ok(result) => {
                            runtime
                                .bump_success(
                                    collector_id,
                                    source_type,
                                    result.inserted as u64,
                                    (result.suppressed_duplicates + result.suppressed_noise) as u64,
                                    Some(observed_at),
                                )
                                .await;
                        }
                        Err(error) => {
                            runtime
                                .record_error(collector_id, source_type, &error.to_string())
                                .await;
                        }
                    }
                }
            }
            Err(error) => {
                runtime
                    .record_error(collector_id, source_type, &error.to_string())
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
    let event_id = state
        .runtime
        .ingest_app_event(
            &state.events_db,
            &state.incidents_db,
            &state.config,
            &payload,
        )
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(Json(
        serde_json::json!({ "event_id": event_id, "accepted": true }),
    ))
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
        Some(NewEventRecord {
            event_id: next_event_id("host"),
            timestamp: now_iso(),
            service_id: "host".into(),
            severity,
            message,
            source_type: "host_metrics".into(),
            source_id: "host_metrics://local".into(),
            tags: entered
                .iter()
                .chain(recovered.iter())
                .map(|value| value.to_string())
                .collect(),
            fingerprint: format!(
                "host-metrics:{}",
                entered
                    .iter()
                    .chain(recovered.iter())
                    .map(|value| value.to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join("|")
            ),
            host_id: sample.hostname.clone(),
            event_type: 1,
            timestamp_source: "collector".into(),
            collected_at: now_iso(),
            quality: Some("normalized".into()),
            structured_data: Some(serde_json::json!({
                "cpu_percent": sample.cpu_percent,
                "memory_percent": sample.memory_percent,
                "disk_percent": sample.disk_percent,
                "disk_free_bytes": sample.disk_free_bytes,
            })),
            raw_offset: None,
        })
    }
}

struct HostSample {
    hostname: String,
    cpu_percent: f32,
    memory_percent: f32,
    disk_percent: f32,
    disk_free_bytes: u64,
}

fn collect_host_sample() -> Result<HostSample> {
    let mut system = System::new_all();
    system.refresh_cpu_usage();
    system.refresh_memory();
    let disks = Disks::new_with_refreshed_list();
    let mut disk_percent = 0.0f32;
    let mut disk_free_bytes = 0u64;
    if let Some(disk) = disks.list().first() {
        let total = disk.total_space().max(1);
        disk_free_bytes = disk.available_space();
        disk_percent = (((total - disk_free_bytes) as f64 / total as f64) * 100.0) as f32;
    }
    Ok(HostSample {
        hostname: System::host_name().unwrap_or_else(|| "local".into()),
        cpu_percent: system.global_cpu_usage(),
        memory_percent: if system.total_memory() == 0 {
            0.0
        } else {
            ((system.used_memory() as f64 / system.total_memory() as f64) * 100.0) as f32
        },
        disk_percent,
        disk_free_bytes,
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
    let mut entries = system.processes().values().collect::<Vec<_>>();
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
        if !watch_pids.is_empty() && !watch_pids.contains(&pid) {
            continue;
        }
        if !watch_processes.is_empty() && !watch_processes.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        let memory_mb = process.memory() as f64 / (1024.0 * 1024.0);
        let cpu = process.cpu_usage();
        let key = format!("{name}:{pid}");
        let hot = cpu >= min_cpu_percent || memory_mb >= min_memory_mb;
        if hot {
            active_hot.insert(key.clone());
            if !seen_hot.contains(&key) {
                events.push(NewEventRecord {
                    event_id: next_event_id("process"),
                    timestamp: now_iso(),
                    service_id: name.clone(),
                    severity: 2,
                    message: format!(
                        "process {name} pid={pid} high cpu={cpu:.1}% memory={memory_mb:.1}MB"
                    ),
                    source_type: "process_snapshot".into(),
                    source_id: format!("process://{pid}"),
                    tags: vec!["process".into(), "threshold".into()],
                    fingerprint: format!("process-hot-{pid}"),
                    host_id: System::host_name().unwrap_or_else(|| "local".into()),
                    event_type: 1,
                    timestamp_source: "collector".into(),
                    collected_at: now_iso(),
                    quality: Some("normalized".into()),
                    structured_data: Some(serde_json::json!({
                        "pid": pid,
                        "cpu_percent": cpu,
                        "memory_mb": memory_mb,
                        "status": format!("{:?}", process.status()),
                    })),
                    raw_offset: None,
                });
            }
        } else if seen_hot.contains(&key) {
            events.push(NewEventRecord {
                event_id: next_event_id("process"),
                timestamp: now_iso(),
                service_id: name.clone(),
                severity: 1,
                message: format!(
                    "process {name} pid={pid} recovered cpu={cpu:.1}% memory={memory_mb:.1}MB"
                ),
                source_type: "process_snapshot".into(),
                source_id: format!("process://{pid}"),
                tags: vec!["process".into(), "recovered".into()],
                fingerprint: format!("process-recovered-{pid}"),
                host_id: System::host_name().unwrap_or_else(|| "local".into()),
                event_type: 1,
                timestamp_source: "collector".into(),
                collected_at: now_iso(),
                quality: Some("normalized".into()),
                structured_data: Some(serde_json::json!({
                    "pid": pid,
                    "cpu_percent": cpu,
                    "memory_mb": memory_mb,
                    "status": format!("{:?}", process.status()),
                })),
                raw_offset: None,
            });
        }
    }
    *seen_hot = active_hot;
    Ok(events)
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
            let (service_id, severity, message) = parse_syslog_line(trimmed);
            events.push(NewEventRecord {
                event_id: next_event_id("syslog"),
                timestamp: now_iso(),
                service_id,
                severity,
                message,
                source_type: "linux_syslog".into(),
                source_id: path.display().to_string(),
                tags: vec!["syslog".into()],
                fingerprint: semantic_fingerprint(
                    "syslog",
                    &path.display().to_string(),
                    "linux_syslog",
                    trimmed,
                ),
                host_id: System::host_name().unwrap_or_else(|| "local".into()),
                event_type: 0,
                timestamp_source: "collector".into(),
                collected_at: now_iso(),
                quality: Some("raw".into()),
                structured_data: Some(serde_json::json!({
                    "path": path.display().to_string(),
                    "raw_offset": current_offset,
                    "raw": trimmed,
                })),
                raw_offset: Some(current_offset as i64),
            });
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
        let fingerprint = semantic_fingerprint("journald", &service_id, "journald", &message);
        events.push(NewEventRecord {
            event_id: next_event_id("journald"),
            timestamp: timestamp.clone(),
            service_id,
            severity: severity_from_priority(priority),
            message,
            source_type: "journald".into(),
            source_id: cursor.clone().unwrap_or_else(|| "journalctl".into()),
            tags: vec!["journald".into()],
            fingerprint,
            host_id: System::host_name().unwrap_or_else(|| "local".into()),
            event_type: 0,
            timestamp_source: "collector".into(),
            collected_at: now_iso(),
            quality: Some("raw".into()),
            structured_data: Some(value),
            raw_offset: None,
        });
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
                .args(["qe", channel, "/rd:true", "/c:1", "/f:text"])
                .output()
                .with_context(|| format!("query latest windows eventlog record for {channel}"))?;
            let latest = parse_wevtutil_events(&String::from_utf8_lossy(&output.stdout), channel)
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
            .args(["qe", channel, "/rd:false", "/f:text", "/c:64", "/q"])
            .arg(&query)
            .output()
            .with_context(|| format!("query windows eventlog channel {channel}"))?;
        if !output.status.success() {
            anyhow::bail!(
                "wevtutil failed for {channel}: {}",
                sc_output_text_like(&output)
            );
        }
        let parsed = parse_wevtutil_events(&String::from_utf8_lossy(&output.stdout), channel);
        let mut newest = last_record;
        for item in parsed {
            newest = newest.max(item.record_id);
            events.push(NewEventRecord {
                event_id: next_event_id("eventlog"),
                timestamp: item.timestamp,
                service_id: item.provider.clone(),
                severity: severity_from_level(&item.level),
                message: item.message,
                source_type: "windows_eventlog".into(),
                source_id: channel.to_string(),
                tags: vec!["eventlog".into(), channel.to_ascii_lowercase()],
                fingerprint: format!("{channel}:{}", item.record_id),
                host_id: System::host_name().unwrap_or_else(|| "local".into()),
                event_type: 0,
                timestamp_source: "collector".into(),
                collected_at: now_iso(),
                quality: Some("raw".into()),
                structured_data: Some(serde_json::json!({
                    "channel": channel,
                    "record_id": item.record_id,
                    "provider": item.provider,
                    "level": item.level,
                })),
                raw_offset: Some(item.record_id as i64),
            });
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
    names: &HashSet<String>,
) -> Result<Vec<NewEventRecord>> {
    if !cfg!(target_os = "windows") {
        return Ok(vec![]);
    }
    let store = EventsStore::open(events_db)?
        .context("event store not found for windows service collector")?;
    let output = Command::new("sc.exe")
        .args(["query", "state=", "all"])
        .output()
        .context("query windows services with sc.exe")?;
    if !output.status.success() {
        anyhow::bail!(
            "sc.exe query state= all failed: {}",
            sc_output_text_like(&output)
        );
    }
    let snapshot = parse_windows_service_snapshot(&String::from_utf8_lossy(&output.stdout));
    let current_json =
        serde_json::to_string(&snapshot).context("serialize windows service snapshot")?;
    let previous = store
        .get_collector_state("windows_service", "snapshot")?
        .and_then(|value| serde_json::from_str::<HashMap<String, String>>(&value).ok());
    store.set_collector_state("windows_service", "snapshot", &current_json, &now_iso())?;
    let Some(previous) = previous else {
        return Ok(vec![]);
    };
    let mut events = Vec::new();
    for (service, state) in snapshot {
        if !names.is_empty() && !names.contains(&service.to_ascii_lowercase()) {
            continue;
        }
        let previous_state = previous.get(&service).cloned().unwrap_or_default();
        if previous_state == state {
            continue;
        }
        if !include_stopped && state == "stopped" {
            continue;
        }
        let severity = if matches!(state.as_str(), "stopped" | "stop_pending") {
            2
        } else {
            1
        };
        events.push(NewEventRecord {
            event_id: next_event_id("service"),
            timestamp: now_iso(),
            service_id: service.clone(),
            severity,
            message: format!("windows service {service} transitioned {previous_state} -> {state}"),
            source_type: "windows_service".into(),
            source_id: format!("service://{service}"),
            tags: vec!["service".into(), state.clone()],
            fingerprint: format!("service:{service}:{state}"),
            host_id: System::host_name().unwrap_or_else(|| "local".into()),
            event_type: 1,
            timestamp_source: "collector".into(),
            collected_at: now_iso(),
            quality: Some("normalized".into()),
            structured_data: Some(serde_json::json!({
                "service": service,
                "previous_state": previous_state,
                "state": state,
            })),
            raw_offset: None,
        });
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
        let name = value
            .get("Actor")
            .and_then(|actor| actor.get("Attributes"))
            .and_then(|attrs| attrs.get("name"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("docker")
            .to_string();
        let name_lower = name.to_ascii_lowercase();
        if exclude_names.contains(&name_lower) {
            continue;
        }
        let labels = value
            .get("Actor")
            .and_then(|actor| actor.get("Attributes"))
            .and_then(serde_json::Value::as_object)
            .map(|attrs| {
                attrs
                    .keys()
                    .filter(|key| key.starts_with("label:"))
                    .map(|key| key.trim_start_matches("label:").to_ascii_lowercase())
                    .collect::<HashSet<_>>()
            })
            .unwrap_or_default();
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
        events.push(NewEventRecord {
            event_id: next_event_id("docker"),
            timestamp: timestamp.clone(),
            service_id: name.clone(),
            severity: if matches!(action.as_str(), "die" | "kill" | "oom") {
                3
            } else {
                1
            },
            message: format!("docker container {name} action={action}"),
            source_type: "docker".into(),
            source_id: value
                .get("Actor")
                .and_then(|actor| actor.get("ID"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("docker")
                .to_string(),
            tags: vec!["docker".into(), action.clone()],
            fingerprint: format!(
                "docker:{}:{}",
                value
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(&name),
                action
            ),
            host_id: System::host_name().unwrap_or_else(|| "local".into()),
            event_type: 0,
            timestamp_source: "collector".into(),
            collected_at: now_iso(),
            quality: Some("raw".into()),
            structured_data: Some(value),
            raw_offset: None,
        });
    }
    store.set_collector_state("docker", "since", &until, &now_iso())?;
    Ok(events)
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
            let service_id = involved
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("kubernetes")
                .to_string();
            let reason = item
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("event");
            let note = item
                .get("note")
                .or_else(|| item.get("message"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("kubernetes event");
            emitted.push(NewEventRecord {
                event_id: next_event_id("k8s"),
                timestamp: if ts.is_empty() { now_iso() } else { ts.clone() },
                service_id,
                severity: if reason.to_ascii_lowercase().contains("fail") {
                    3
                } else {
                    1
                },
                message: format!("kubernetes {reason}: {note}"),
                source_type: "kubernetes".into(),
                source_id: item
                    .get("metadata")
                    .and_then(|meta| meta.get("uid"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("kubernetes")
                    .to_string(),
                tags: vec!["kubernetes".into(), "event".into()],
                fingerprint: format!(
                    "k8s-event:{}",
                    item.get("metadata")
                        .and_then(|meta| meta.get("uid"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                ),
                host_id: "cluster".into(),
                event_type: 0,
                timestamp_source: "collector".into(),
                collected_at: now_iso(),
                quality: Some("raw".into()),
                structured_data: Some(item),
                raw_offset: None,
            });
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
            let key = format!("{namespace}/{name}");
            if let Some(previous_phase) = previous.get(&key) {
                if previous_phase != &phase {
                    emitted.push(NewEventRecord {
                        event_id: next_event_id("k8s-pod"),
                        timestamp: now_iso(),
                        service_id: name.to_string(),
                        severity: if phase.eq_ignore_ascii_case("failed") {
                            3
                        } else {
                            1
                        },
                        message: format!(
                            "kubernetes pod {key} transitioned {previous_phase} -> {phase}"
                        ),
                        source_type: "kubernetes".into(),
                        source_id: key.clone(),
                        tags: vec!["kubernetes".into(), "pod".into(), phase.clone()],
                        fingerprint: format!("k8s-pod:{key}:{phase}"),
                        host_id: namespace.to_string(),
                        event_type: 1,
                        timestamp_source: "collector".into(),
                        collected_at: now_iso(),
                        quality: Some("normalized".into()),
                        structured_data: Some(item.clone()),
                        raw_offset: None,
                    });
                }
            }
            current.insert(key, phase);
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
    let service_id = target
        .service_id
        .clone()
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
    NewEventRecord {
        event_id: next_event_id("file"),
        timestamp: now_iso(),
        service_id,
        severity: severity_from_text(message),
        message: message.to_string(),
        source_type: "file".into(),
        source_id: target.path.display().to_string(),
        tags: vec!["file".into()],
        fingerprint: semantic_fingerprint(
            "file",
            &target.path.display().to_string(),
            "file",
            message,
        ),
        host_id: System::host_name().unwrap_or_else(|| "local".into()),
        event_type: 0,
        timestamp_source: "collector".into(),
        collected_at: now_iso(),
        quality: Some("raw".into()),
        structured_data: Some(serde_json::json!({
            "path": target.path.display().to_string(),
            "raw_offset": raw_offset,
            "multiline_pattern": target.multiline_pattern,
        })),
        raw_offset: Some(raw_offset as i64),
    }
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    wildcard_match_bytes(pattern.as_bytes(), text.as_bytes())
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

fn parse_windows_service_snapshot(text: &str) -> HashMap<String, String> {
    let mut current_service = None::<String>;
    let mut current_state = None::<String>;
    let mut out = HashMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some((_, tail)) = line.split_once("SERVICE_NAME:") {
            if let (Some(service), Some(state)) = (current_service.take(), current_state.take()) {
                out.insert(service, state);
            }
            current_service = Some(tail.trim().to_string());
        } else if let Some((_, tail)) = line.split_once("STATE") {
            if let Some((_, value)) = tail.split_once(':') {
                current_state = parse_state_value(value.trim());
            }
        }
    }
    if let (Some(service), Some(state)) = (current_service, current_state) {
        out.insert(service, state);
    }
    out
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
    level: String,
    message: String,
}

fn parse_wevtutil_events(text: &str, channel: &str) -> Vec<WindowsEventRecord> {
    let blocks = text
        .split("\r\n\r\n")
        .flat_map(|chunk| chunk.split("\n\n"))
        .filter(|chunk| chunk.contains("Event ID:"));
    let mut out = Vec::new();
    for block in blocks {
        let mut record_id = None::<u64>;
        let mut provider = channel.to_string();
        let mut level = "info".to_string();
        let mut timestamp = now_iso();
        let mut message_lines = Vec::new();
        let mut in_description = false;
        for raw in block.lines() {
            let line = raw.trim();
            if let Some(value) = line.strip_prefix("Event ID:") {
                record_id = value.trim().parse::<u64>().ok();
            } else if let Some(value) = line.strip_prefix("Provider Name:") {
                provider = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("Source:") {
                provider = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("Level:") {
                level = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("Date:") {
                timestamp = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("Description:") {
                in_description = true;
                if !value.trim().is_empty() {
                    message_lines.push(value.trim().to_string());
                }
            } else if in_description && !line.is_empty() {
                message_lines.push(line.to_string());
            }
        }
        if let Some(record_id) = record_id {
            out.push(WindowsEventRecord {
                record_id,
                timestamp,
                provider: provider.clone(),
                level,
                message: if message_lines.is_empty() {
                    format!("{provider} event in {channel}")
                } else {
                    message_lines.join(" ")
                },
            });
        }
    }
    out
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
    let result = persist_events(events_db, config, events)?;
    if result.inserted > 0 {
        let _ = reconcile_new_events(events_db, incidents_db, config, &result.inserted_event_ids)?;
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
    }
}

fn semantic_fingerprint(prefix: &str, service: &str, source_type: &str, message: &str) -> String {
    format!(
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
    )
}

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_match_supports_star_and_question_mark() {
        assert!(wildcard_match("*.log", "app.log"));
        assert!(wildcard_match("app-?.txt", "app-1.txt"));
        assert!(!wildcard_match("app-?.txt", "app-10.txt"));
    }

    #[test]
    fn collector_specs_include_file_and_app_standalone_when_enabled() {
        let config: TomlValue = r#"
[collectors.file]
enabled = true
paths = ["./logs/app.log"]
poll_interval_seconds = 1.0
start_at_end = false

[collectors.app]
enabled = true
enable_main_api = true
enable_standalone = true
listen = "127.0.0.1:9999"
mount_path = "/custom-ingest"
shared_token = "token"
max_payload_bytes = 2048
"#
        .parse()
        .expect("parse config");

        let specs = collector_specs(&config);
        assert!(specs
            .iter()
            .any(|spec| matches!(spec, CollectorSpec::File { .. })));
        assert!(specs
            .iter()
            .any(|spec| matches!(spec, CollectorSpec::AppIngest)));
        assert!(specs.iter().any(|spec| matches!(
            spec,
            CollectorSpec::AppStandalone {
                listen,
                mount_path,
                max_payload_bytes,
                ..
            } if listen == "127.0.0.1:9999" && mount_path == "/custom-ingest" && *max_payload_bytes == 2048
        )));
    }

    #[test]
    fn normalized_mount_path_adds_leading_slash() {
        assert_eq!(normalized_mount_path("api/ingest"), "/api/ingest");
        assert_eq!(normalized_mount_path("/api/ingest"), "/api/ingest");
        assert_eq!(normalized_mount_path(""), "/api/ingest");
    }
}
