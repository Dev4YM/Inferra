//! Axum HTTP server: native `/api/*` plus static UI.

use anyhow::{bail, Context, Result};
use async_stream::stream;
use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{sse::Event, sse::KeepAlive, sse::Sse, IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use futures_util::Stream;
use inferra_collectors::{
    configured_collectors, normalize_otlp_logs_protobuf_request, CollectorRuntime,
};
use inferra_config::{
    apply_config_put, config_to_json, experience_from_config, load_merged_config, resolve_data_dir,
    observability_export_enabled, observability_export_url, observability_logs_fts_enabled,
    observability_otlp_logs_enabled, observability_otlp_max_logs_per_request,
    observability_otlp_max_payload_bytes, server_listen, storage_retention_hours, Paths,
};
use inferra_contracts::{
    AiDoctorResponse, AiStatusResponse, ApiVersionResponse, CollectorRow, CollectorsResponse,
    ConfigResponse, EventRow, IncidentDetailResponse, IncidentRow, OverviewResponse,
    ServiceDetailResponse, SeverityValue, TraceSummary, WorkspaceMapResponse, WorkspaceMapping,
    WorkspaceRuntimeApp,
};
use inferra_core::{
    adaptive_learning_audit_log, adaptive_learning_bulk_review_artifacts,
    adaptive_learning_bulk_set_artifact_state, adaptive_learning_delete_review_view,
    adaptive_learning_history, adaptive_learning_review_artifact, adaptive_learning_review_summary,
    adaptive_learning_save_review_view, adaptive_learning_set_artifact_state,
    adaptive_learning_summary, adaptive_learning_touch_review_view, ai_status_from_config,
    build_overview, build_overview_with_runtime_signals, build_workspace_map,
    collect_host_resources_snapshot, collect_runtime_monitor_window, refresh_incident_reasoning,
    try_collect_gpu_summary, workspace_app_live_resources, AdaptiveArtifactSelection,
    AdaptiveSavedReviewViewDraft, OverviewRuntimeSignals, enrich_incident_rows_with_latest_traces,
};
use inferra_storage::{
    initialize_databases, AdaptiveLearningAuditQuery, AdaptiveLearningHistoryQuery, EventsStore,
    IncidentsStore, LogsQuery, StoredAiGeneration, StoredAiTrace, StoredExplanation, StoredFeedback,
    StoredUiSnapshot,
};
use serde_json::{json, Value as JsonValue};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use tokio::sync::RwLock;
use toml::Value as TomlValue;

mod export_sink;
mod middleware;

const INVESTIGATION_MAX_AI_ATTEMPTS: usize = 3;
const SCANNER_WORKSPACE_MIN_INTERVAL_SECONDS: u64 = 15;
const SCANNER_WORKSPACE_DEFAULT_INTERVAL_SECONDS: u64 = 120;
const SCANNER_WORKSPACE_MAX_INTERVAL_SECONDS: u64 = 3600;
const UI_SNAPSHOT_SOURCE_CORE: &str = "inferra_core";
const SNAPSHOT_OVERVIEW: &str = "overview";
const SNAPSHOT_GRAPH: &str = "graph";
const SNAPSHOT_SYSTEMS: &str = "systems";
const SNAPSHOT_WORKSPACE: &str = "workspace";
const SNAPSHOT_AI_STATUS: &str = "ai.status";
const SNAPSHOT_AI_INVESTIGATION: &str = "ai.investigation";
const SNAPSHOT_CONTROL: &str = "control";
const SNAPSHOT_SETTINGS: &str = "settings";
const INVESTIGATION_SYSTEM_PROMPT: &str = "You are Inferra's read-only investigation assistant.\n\
You receive a redacted runtime evidence bundle. You must:\n\
- explain what is happening using only the supplied facts\n\
- prioritize the next inspection step the operator should take\n\
- cite supporting incident_id, service_id, or event_id values when possible\n\
- never claim you executed or modified anything\n\
- never propose remediation that would mutate the observed system\n\
- do not include CLI commands unless the bundle explicitly exposes an executable action surface for that exact command\n\
- prefer UI inspection steps such as reviewing the incident, evidence, service events, collector status, or workspace app logs\n\
- include explicit uncertainty when evidence is thin\n\
- make the answer specific to context_summary, focus, selected workspace app/service/incident, and the supplied evidence\n\
- respect host_resources and runtime_monitor samples as authoritative for machine state during this investigation window\n\
- interpret process/app resources carefully: resources.cpu_percent is an estimated share of total host CPU, resources.cpu_raw_percent is the raw single-core-equivalent process reading, and cpu_raw_percent must never be described as whole-machine CPU saturation\n\
- if hypotheses[] is non-empty, list likely_causes in the SAME best-first order as hypotheses (by rank ascending, then total_score descending). If you disagree, keep that order and explain the disagreement under uncertainty[]\n\
- never invent IDs: every citation and evidence[].id must exist in the bundle\n\n\
Return a single JSON object. No markdown fences. No prose outside JSON.\n\
Every next_steps entry must have safety=\"read_only\" and requires_user_action=true.";
const INVESTIGATION_USER_TEMPLATE: &str = "Mode: {mode}\n\
Bundle:\n\
{bundle_json}\n\
Schema:\n\
{{\n\
  \"headline\": \"string\",\n\
  \"risk_level\": \"low|medium|high|critical\",\n\
  \"confidence\": 0.0,\n\
  \"what_happened\": [\"string\"],\n\
  \"why_it_matters\": [\"string\"],\n\
  \"likely_causes\": [\"string\"],\n\
  \"evidence\": [{{\"type\": \"incident|service|event|workspace\", \"id\": \"string\", \"summary\": \"string\"}}],\n\
  \"missing_evidence\": [\"string\"],\n\
  \"next_steps\": [\n\
    {{\n\
      \"title\": \"string\",\n\
      \"reason\": \"string\",\n\
      \"safety\": \"read_only\",\n\
      \"command\": \"string, optional and usually empty unless an exact executable surface exists\",\n\
      \"requires_user_action\": true\n\
    }}\n\
  ],\n\
  \"uncertainty\": [\"string\"],\n\
  \"citations\": [\"string\"]\n\
}}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiProbePurpose {
    Status,
    Investigate,
}

fn resolve_status_probe_model(ai: Option<&toml::value::Table>) -> String {
    let s = ai_table_string(ai, "model_status", "");
    if s.trim().is_empty() {
        ai_table_string(ai, "model", "")
    } else {
        s
    }
}

fn resolve_investigate_probe_model(ai: Option<&toml::value::Table>) -> String {
    let s = ai_table_string(ai, "model_investigate", "");
    if s.trim().is_empty() {
        ai_table_string(ai, "model", "")
    } else {
        s
    }
}

fn probe_model_name(ai: Option<&toml::value::Table>, purpose: AiProbePurpose) -> String {
    match purpose {
        AiProbePurpose::Status => resolve_status_probe_model(ai),
        AiProbePurpose::Investigate => resolve_investigate_probe_model(ai),
    }
}

#[derive(Clone, Debug)]
struct AiProviderProbe {
    enabled: bool,
    provider: String,
    base_url: String,
    model: String,
    allow_remote: bool,
    available: bool,
    installed: bool,
    resolved_model: Option<String>,
    reason: Option<String>,
    error: Option<String>,
}

impl AiProviderProbe {
    fn disabled(config: &TomlValue, purpose: AiProbePurpose) -> Self {
        let ai = config.get("ai").and_then(|value| value.as_table());
        let model = probe_model_name(ai, purpose);
        Self {
            enabled: false,
            provider: ai_table_string(ai, "provider", "ollama"),
            base_url: ai_table_string(ai, "base_url", "http://127.0.0.1:11434"),
            model,
            allow_remote: ai_table_bool(ai, "allow_remote", false),
            available: false,
            installed: false,
            resolved_model: None,
            reason: Some("AI is disabled in config.".to_string()),
            error: None,
        }
    }

    fn unavailable(config: &TomlValue, purpose: AiProbePurpose, reason: impl Into<String>) -> Self {
        let ai = config.get("ai").and_then(|value| value.as_table());
        let model = probe_model_name(ai, purpose);
        let reason = reason.into();
        Self {
            enabled: true,
            provider: ai_table_string(ai, "provider", "ollama"),
            base_url: ai_table_string(ai, "base_url", "http://127.0.0.1:11434"),
            model,
            allow_remote: ai_table_bool(ai, "allow_remote", false),
            available: false,
            installed: false,
            resolved_model: None,
            reason: Some(reason.clone()),
            error: Some(reason),
        }
    }

    fn provider_payload(&self) -> JsonValue {
        json!({
            "enabled": self.enabled,
            "available": self.available,
            "model": self.resolved_model.clone().unwrap_or_else(|| self.model.clone()),
            "base_url": self.base_url,
            "allow_remote": self.allow_remote,
            "reason": self.reason,
        })
    }
}

#[derive(Clone)]
pub struct AppState {
    pub paths: Arc<Paths>,
    pub config: Arc<RwLock<TomlValue>>,
    pub collectors: CollectorRuntime,
    pub scanner_cache: Arc<RwLock<ScannerCache>>,
    pub ui_dist: PathBuf,
    pub rate_limits: Arc<middleware::RateLimitState>,
}

#[derive(Default)]
pub struct ScannerCache {
    workspace: Option<CachedWorkspaceMap>,
}

#[derive(Clone)]
struct CachedWorkspaceMap {
    value: WorkspaceMapResponse,
    scanned_at: OffsetDateTime,
    cached_at: Instant,
    interval_seconds: u64,
}

#[derive(serde::Deserialize)]
struct ScanQuery {
    force: Option<bool>,
}

pub fn app_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(api_healthz))
        .route("/readyz", get(api_readyz))
        .route("/api/version", get(api_version))
        .route("/api/health", get(api_health))
        .route("/api/config", get(api_get_config).put(api_put_config))
        .route("/api/overview", get(api_overview))
        .route("/api/metrics", get(api_metrics))
        .route("/api/events", get(api_events))
        .route("/api/events/{event_id}", get(api_event_detail))
        .route("/api/anomaly/{service_id}/status", get(api_anomaly_status))
        .route("/api/logs", get(api_logs))
        .route("/api/v2/logs", get(api_logs_v2))
        .route("/api/traces/{trace_id}", get(api_trace_timeline))
        .route("/api/incidents", get(api_incidents))
        .route("/api/incidents/{incident_id}", get(api_incident_detail))
        .route(
            "/api/incidents/{incident_id}/events",
            get(api_incident_events),
        )
        .route(
            "/api/incidents/{incident_id}/logs",
            get(api_incident_logs),
        )
        .route(
            "/api/incidents/{incident_id}/hypotheses",
            get(api_incident_hypotheses),
        )
        .route(
            "/api/incidents/{incident_id}/clusters",
            get(api_incident_clusters),
        )
        .route(
            "/api/incidents/{incident_id}/feedback",
            post(api_incident_feedback),
        )
        .route("/api/learning/adaptive", get(api_adaptive_learning))
        .route(
            "/api/learning/adaptive/audit",
            get(api_adaptive_learning_audit),
        )
        .route(
            "/api/learning/adaptive/history",
            get(api_adaptive_learning_history),
        )
        .route(
            "/api/learning/adaptive/review",
            get(api_adaptive_learning_review),
        )
        .route(
            "/api/learning/adaptive/views",
            post(api_adaptive_learning_save_view),
        )
        .route(
            "/api/learning/adaptive/views/{view_id}",
            delete(api_adaptive_learning_delete_view),
        )
        .route(
            "/api/learning/adaptive/views/{view_id}/use",
            post(api_adaptive_learning_use_view),
        )
        .route(
            "/api/learning/adaptive/bulk/review",
            post(api_adaptive_learning_bulk_review_action),
        )
        .route(
            "/api/learning/adaptive/bulk/state",
            post(api_adaptive_learning_bulk_state_action),
        )
        .route(
            "/api/learning/adaptive/{artifact_kind}/{artifact_id}/review",
            post(api_adaptive_learning_review_action),
        )
        .route(
            "/api/learning/adaptive/{artifact_kind}/{artifact_id}",
            post(api_adaptive_learning_action),
        )
        .route("/api/services", get(api_services))
        .route("/api/services/{service_id}", get(api_service_detail))
        .route("/api/services/{service_id}/events", get(api_service_events))
        .route("/api/ai/status", get(api_ai_status))
        .route("/api/ai/doctor", get(api_ai_doctor))
        .route("/api/ai/generations", get(api_ai_generations))
        .route("/api/ai/ask", post(api_ai_ask))
        .route(
            "/api/ai/investigate-stream",
            post(api_ai_investigate_stream),
        )
        .route(
            "/api/ai/context",
            get(api_ai_context_get).put(api_ai_context_put),
        )
        .route("/api/ai/report/{incident_id}", get(api_ai_report))
        .route("/api/investigate/now", get(api_investigate_now))
        .route(
            "/api/investigate/incident/{incident_id}",
            get(api_investigate_incident),
        )
        .route(
            "/api/investigate/service/{service_id}",
            get(api_investigate_service),
        )
        .route("/api/collectors", get(api_collectors))
        .route("/api/collectors/start", post(api_collectors_start))
        .route("/api/collectors/stop", post(api_collectors_stop))
        .route("/api/scanner/status", get(api_scanner_status))
        .route("/api/scanner/run", post(api_scanner_run))
        .route("/api/ingest", post(api_ingest))
        .route("/v1/logs", post(api_otlp_logs))
        .route("/api/workspace/projects", get(api_workspace_projects))
        .route("/api/workspace/map", get(api_workspace_map))
        .route("/api/workspace/services", get(api_workspace_services))
        .route(
            "/api/workspace/apps/{app_name}/logs",
            get(api_workspace_app_logs),
        )
        .route(
            "/api/workspace/apps/{app_name}/resources",
            get(api_workspace_app_resources),
        )
        .route("/api/workspace/inspect", get(api_workspace_inspect))
        .route("/api/workspace/mappings", post(api_workspace_add_mapping))
        .route("/api/topology", get(api_topology))
        .route("/api/topology/edges", post(api_topology_add_edge))
        .fallback(proxy_or_static_handler)
        .with_state(state)
}

pub async fn serve(paths: Paths, ui_dist: PathBuf) -> Result<()> {
    serve_with_shutdown(paths, ui_dist, std::future::pending::<()>()).await
}

pub async fn serve_with_shutdown<F>(paths: Paths, ui_dist: PathBuf, shutdown: F) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let merged = load_merged_config(&paths.config_path)?;
    let collectors_auto_start = merged
        .get("collectors")
        .and_then(|value| value.get("auto_start"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let (host, port) = server_listen(&merged)?;
    let chat_rate = merged
        .get("server")
        .and_then(|s| s.get("rate_limit_chat_tokens_per_minute"))
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .unwrap_or(30.0);
    let explain_rate = merged
        .get("server")
        .and_then(|s| s.get("rate_limit_explain_tokens_per_minute"))
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .unwrap_or(15.0);
    let state = AppState {
        paths: Arc::new(paths),
        config: Arc::new(RwLock::new(merged)),
        collectors: CollectorRuntime::default(),
        scanner_cache: Arc::new(RwLock::new(ScannerCache::default())),
        ui_dist,
        rate_limits: Arc::new(middleware::RateLimitState::new(chat_rate, explain_rate)),
    };
    if collectors_auto_start {
        state
            .collectors
            .start(
                &state.config.read().await.clone(),
                state.paths.events_db.clone(),
                state.paths.incidents_db.clone(),
            )
            .await?;
    }
    tokio::spawn(scanner_service_loop(state.clone()));
    let export_handle = {
        let cfg = state.config.read().await;
        if observability_export_enabled(&cfg) && observability_export_url(&cfg).is_some() {
            Some(tokio::spawn(export_sink::run(state.clone())))
        } else {
            None
        }
    };
    let app = middleware::apply_http_middleware(state.clone(), app_router(state.clone()));

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("inferra Rust runtime listening on http://{addr}");
    let serve_result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await;
    if let Some(handle) = export_handle {
        handle.abort();
    }
    serve_result?;
    Ok(())
}

fn workspace_scan_enabled(config: &TomlValue) -> bool {
    config
        .get("workspace")
        .and_then(|value| value.get("enabled"))
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

async fn scanner_service_loop(state: AppState) {
    loop {
        let cfg = state.config.read().await.clone();
        let interval_seconds = workspace_scan_interval_seconds(&cfg);
        if workspace_scan_enabled(&cfg) {
            match refresh_workspace_scan_cache(&state, &cfg).await {
                Ok(workspace) => tracing::debug!(
                    projects = workspace.projects.len(),
                    runtime_apps = workspace.runtime_apps.len(),
                    "workspace scanner snapshot refreshed"
                ),
                Err(error) => tracing::warn!(error = %error, "workspace scanner refresh failed"),
            }
        }
        tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
    }
}

async fn api_version() -> Json<ApiVersionResponse> {
    Json(ApiVersionResponse {
        name: "inferra".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api: "1".into(),
    })
}

async fn api_healthz() -> Json<JsonValue> {
    Json(json!({
        "status": "ok",
        "runtime": "rust",
    }))
}

fn storage_degraded_reasons(paths: &Paths) -> Vec<String> {
    let mut degraded_reasons: Vec<String> = Vec::new();
    if !paths.data_dir.exists() {
        degraded_reasons.push(format!("data_dir missing: {}", paths.data_dir.display()));
    }
    match inferra_storage::probe_database_writable(&paths.events_db) {
        Ok(()) => {}
        Err(reason) => degraded_reasons.push(format!("events.db: {reason}")),
    }
    match inferra_storage::probe_database_writable(&paths.incidents_db) {
        Ok(()) => {}
        Err(reason) => degraded_reasons.push(format!("incidents.db: {reason}")),
    }
    degraded_reasons
}

async fn api_readyz(State(state): State<AppState>) -> Response {
    let degraded_reasons = storage_degraded_reasons(state.paths.as_ref());
    let ready = degraded_reasons.is_empty();
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "status": if ready { "ready" } else { "not_ready" },
            "runtime": "rust",
            "storage_writes_ok": ready,
        })),
    )
        .into_response()
}

async fn api_health(State(state): State<AppState>) -> Json<JsonValue> {
    let cfg = state.config.read().await;
    let paths = state.paths.as_ref();
    let degraded_reasons = storage_degraded_reasons(paths);
    let storage_writes_ok = degraded_reasons.is_empty();
    let status = if storage_writes_ok { "ok" } else { "degraded" };
    Json(json!({
        "status": status,
        "runtime": "rust",
        "storage_writes_ok": storage_writes_ok,
        "degraded_reasons": degraded_reasons,
        "config_path": paths.config_path,
        "data_dir": paths.data_dir,
        "events_db": paths.events_db,
        "incidents_db": paths.incidents_db,
        "ai_enabled": cfg.get("ai").and_then(|a| a.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false),
    }))
}

async fn api_get_config(State(state): State<AppState>) -> Json<ConfigResponse> {
    let cfg = state.config.read().await;
    let config_json = config_to_json(&cfg);
    let _ = persist_ui_snapshot(
        state.paths.as_ref(),
        SNAPSHOT_SETTINGS,
        &config_json,
        UI_SNAPSHOT_SOURCE_CORE,
        None,
    );
    Json(ConfigResponse {
        config: config_json,
        applied: None,
    })
}

async fn api_put_config(
    State(state): State<AppState>,
    Json(body): Json<JsonValue>,
) -> Result<Json<ConfigResponse>, (StatusCode, String)> {
    let base = state.config.read().await.clone();
    let new_cfg = apply_config_put(base, &body)
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    let new_root = resolve_data_dir(&state.paths.config_path, &new_cfg)
        .map_err(|e| (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    if new_root != state.paths.data_dir {
        return Err((
            StatusCode::CONFLICT,
            "storage.data_dir cannot be changed at runtime".into(),
        ));
    }
    let config_path = state.paths.config_path.clone();
    let paths = state.paths.clone();
    let cfg_for_disk = new_cfg.clone();
    let config_json = config_to_json(&new_cfg);
    let snapshot_json = config_json.clone();
    tokio::task::spawn_blocking(move || {
        inferra_config::write_config(&config_path, &cfg_for_disk)
            .map_err(|e| e.to_string())?;
        persist_ui_snapshot(
            paths.as_ref(),
            SNAPSHOT_SETTINGS,
            &snapshot_json,
            UI_SNAPSHOT_SOURCE_CORE,
            None,
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    *state.config.write().await = new_cfg.clone();
    Ok(Json(ConfigResponse {
        config: config_json,
        applied: Some(true),
    }))
}

async fn api_overview(
    State(state): State<AppState>,
) -> Result<Json<OverviewResponse>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let probe = probe_ai_provider(&cfg, AiProbePurpose::Status).await;
    let signals = OverviewRuntimeSignals {
        ai_available: Some(probe.available),
        ai_reason: probe.reason.clone(),
        queue_depth: Some(state.collectors.queue_depth()),
        collector_errors: Some(state.collectors.active_error_count().await),
    };
    let paths = state.paths.clone();
    let cfg_for_overview = cfg.clone();
    let signals_for_overview = signals.clone();
    let overview = tokio::task::spawn_blocking(move || {
        build_overview_with_runtime_signals(&cfg_for_overview, paths.as_ref(), Some(&signals_for_overview))
    })
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let _ = persist_ui_snapshot(
        state.paths.as_ref(),
        SNAPSHOT_OVERVIEW,
        &serde_json::to_value(&overview).unwrap_or_else(|_| json!({})),
        UI_SNAPSHOT_SOURCE_CORE,
        None,
    );
    Ok(Json(overview))
}

async fn api_metrics(State(state): State<AppState>) -> Response {
    let expose_metrics = {
        let cfg = state.config.read().await;
        cfg.get("server")
            .and_then(|server| server.get("expose_prometheus_metrics"))
            .and_then(TomlValue::as_bool)
            .unwrap_or(false)
    };
    if !expose_metrics {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"detail": "metrics endpoint disabled"})),
        )
            .into_response();
    }
    let event_count = EventsStore::open(&state.paths.events_db)
        .ok()
        .flatten()
        .and_then(|store| store.count_events().ok())
        .unwrap_or(0);
    let active_incidents = IncidentsStore::open(&state.paths.incidents_db)
        .ok()
        .flatten()
        .and_then(|store| store.active_incident_count().ok())
        .unwrap_or(0);
    let queue_depth = state.collectors.queue_depth();
    let ex_ok = export_sink::EXPORT_BATCHES_SUCCESS.load(std::sync::atomic::Ordering::Relaxed);
    let ex_fail = export_sink::EXPORT_BATCHES_FAILED.load(std::sync::atomic::Ordering::Relaxed);
    let ex_events = export_sink::EXPORT_EVENTS_FORWARDED.load(std::sync::atomic::Ordering::Relaxed);
    let ex_dropped = export_sink::EXPORT_EVENTS_DROPPED.load(std::sync::atomic::Ordering::Relaxed);
    let ex_retries = export_sink::EXPORT_RETRIES_TOTAL.load(std::sync::atomic::Ordering::Relaxed);
    let ex_split = export_sink::EXPORT_BATCHES_SPLIT.load(std::sync::atomic::Ordering::Relaxed);
    let ex_partial = export_sink::EXPORT_PARTIAL_REJECTIONS.load(std::sync::atomic::Ordering::Relaxed);
    let ex_busy = export_sink::EXPORT_TICKS_SKIPPED_BUSY.load(std::sync::atomic::Ordering::Relaxed);
    let payload = format!(
        "# HELP inferra_events_total Approximate stored normalized events.\n# TYPE inferra_events_total counter\ninferra_events_total {event_count}\n# HELP inferra_active_incidents Active incidents (open, investigating, explained).\n# TYPE inferra_active_incidents gauge\ninferra_active_incidents {active_incidents}\n# HELP inferra_raw_queue_depth In-flight ingestion operations.\n# TYPE inferra_raw_queue_depth gauge\ninferra_raw_queue_depth {queue_depth}\n# HELP inferra_observability_export_batches_success_total OTLP export HTTP batches accepted by sink.\n# TYPE inferra_observability_export_batches_success_total counter\ninferra_observability_export_batches_success_total {ex_ok}\n# HELP inferra_observability_export_batches_failed_total OTLP export HTTP batches that failed or were rejected.\n# TYPE inferra_observability_export_batches_failed_total counter\ninferra_observability_export_batches_failed_total {ex_fail}\n# HELP inferra_observability_export_events_forwarded_total Event rows included in successful export batches.\n# TYPE inferra_observability_export_events_forwarded_total counter\ninferra_observability_export_events_forwarded_total {ex_events}\n# HELP inferra_observability_export_events_dropped_total Event rows skipped after sink rejection was isolated to a single poison record.\n# TYPE inferra_observability_export_events_dropped_total counter\ninferra_observability_export_events_dropped_total {ex_dropped}\n# HELP inferra_observability_export_retries_total Additional export attempts made after retryable sink failures.\n# TYPE inferra_observability_export_retries_total counter\ninferra_observability_export_retries_total {ex_retries}\n# HELP inferra_observability_export_batches_split_total Export batches recursively split after sink-side validation or partial rejection.\n# TYPE inferra_observability_export_batches_split_total counter\ninferra_observability_export_batches_split_total {ex_split}\n# HELP inferra_observability_export_partial_rejections_total Downstream OTLP partialSuccess rejected log record count observed by the exporter.\n# TYPE inferra_observability_export_partial_rejections_total counter\ninferra_observability_export_partial_rejections_total {ex_partial}\n# HELP inferra_observability_export_ticks_skipped_busy_total Export ticks skipped because a previous export was still running (backpressure).\n# TYPE inferra_observability_export_ticks_skipped_busy_total counter\ninferra_observability_export_ticks_skipped_busy_total {ex_busy}\n"
    );
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; version=0.0.4"),
        )],
        payload,
    )
        .into_response()
}

async fn api_events(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 500);
    let store = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let events = if let Some(store) = store {
        store
            .latest_events(limit)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        vec![]
    };
    Ok(Json(json!({ "events": events })))
}

async fn api_event_detail(
    State(state): State<AppState>,
    AxumPath(event_id): AxumPath<String>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let store = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Event store not found".to_string()))?;
    let event = store
        .get_event(&event_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Event not found".to_string()))?;
    Ok(Json(json!({ "event": event })))
}

async fn api_anomaly_status(
    State(state): State<AppState>,
    AxumPath(service_id): AxumPath<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let window_hours = params
        .get("window_hours")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(24)
        .clamp(1, 168);
    let store = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Event store not found".to_string()))?;
    let events = store
        .events_for_service(&service_id, 500)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let event_count = events.len();
    let mut severity_buckets = std::collections::BTreeMap::<String, i64>::new();
    let mut error_count = 0i64;
    let mut critical_count = 0i64;
    for event in &events {
        let severity = event
            .severity
            .as_ref()
            .and_then(severity_value_to_i64)
            .unwrap_or_default();
        *severity_buckets.entry(severity.to_string()).or_default() += 1;
        if severity >= 3 {
            error_count += 1;
        }
        if severity >= 4 {
            critical_count += 1;
        }
    }
    let status = if event_count == 0 {
        "unknown"
    } else if critical_count > 0 {
        "critical"
    } else if error_count > 0 || (error_count as f64 / event_count as f64) >= 0.25 {
        "degraded"
    } else {
        "healthy"
    };
    Ok(Json(json!({
        "enabled": true,
        "service_id": service_id,
        "status": status,
        "window_hours": window_hours,
        "event_count": event_count,
        "error_count": error_count,
        "last_event_at": events.last().and_then(|event| event.timestamp.clone()),
        "buckets": severity_buckets,
    })))
}

async fn api_logs(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 2000);
    let severity = params
        .get("severity")
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.clamp(0, 4));
    let cfg = state.config.read().await.clone();
    let retention = storage_retention_hours(&cfg);
    let store = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let logs = if let Some(store) = store {
        store
            .query_logs(&LogsQuery {
                limit,
                retention_hours: retention,
                service_id: params.get("service").cloned(),
                min_severity: severity,
                search: params
                    .get("search")
                    .or_else(|| params.get("q"))
                    .cloned(),
                source_type: params.get("source_type").cloned(),
                trace_id: params.get("trace_id").cloned(),
                attr_key: params.get("attr_key").cloned(),
                attr_value: params.get("attr_value").cloned(),
                ..Default::default()
            })
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        vec![]
    };
    Ok(Json(json!({ "logs": logs, "limit": limit })))
}

async fn api_logs_v2(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 2000);
    let severity = params
        .get("severity")
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.clamp(0, 4));
    let cfg = state.config.read().await.clone();
    let retention = storage_retention_hours(&cfg);
    let log_fts_enabled = observability_logs_fts_enabled(&cfg);
    let query = LogsQuery {
        limit,
        retention_hours: retention,
        service_id: params.get("service").cloned(),
        min_severity: severity,
        search: params.get("search").cloned(),
        fts_query: params.get("q").cloned(),
        log_fts_enabled,
        source_type: params.get("source_type").cloned(),
        trace_id: params.get("trace_id").cloned(),
        timestamp_after: params.get("start").cloned(),
        timestamp_before: params.get("end").cloned(),
        cursor_timestamp: params.get("cursor_timestamp").cloned(),
        cursor_event_id: params.get("cursor_event_id").cloned(),
        attr_key: params.get("attr_key").cloned(),
        attr_value: params.get("attr_value").cloned(),
        ..Default::default()
    };
    let store = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let items = if let Some(store) = store {
        store
            .query_logs(&query)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        vec![]
    };
    let next_cursor = if items.len() == limit {
        items.last().map(|row| {
            json!({
                "cursor_timestamp": row.timestamp,
                "cursor_event_id": row.event_id,
            })
        })
    } else {
        None
    };
    Ok(Json(json!({
        "items": items,
        "limit": limit,
        "retention_hours": retention,
        "log_fts_enabled": log_fts_enabled,
        "next_cursor": next_cursor,
    })))
}

fn normalize_w3c_trace_id_for_api(raw: &str) -> Result<String, (StatusCode, String)> {
    let hex: String = raw
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    if hex.len() == 32 {
        Ok(hex)
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "trace_id must be 32 hex characters (W3C trace id)".to_string(),
        ))
    }
}

async fn api_trace_timeline(
    State(state): State<AppState>,
    AxumPath(trace_id): AxumPath<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let trace_id = normalize_w3c_trace_id_for_api(&trace_id)?;
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let retention = storage_retention_hours(&cfg);
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(500)
        .clamp(1, 2000);
    let store = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let items = if let Some(store) = store {
        store
            .query_trace_timeline(
                &trace_id,
                limit,
                retention,
                params.get("start").cloned(),
                params.get("end").cloned(),
            )
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        vec![]
    };
    let count = items.len();
    Ok(Json(json!({
        "trace_id": trace_id,
        "items": items,
        "limit": limit,
        "retention_hours": retention,
        "count": count,
    })))
}

async fn api_incidents(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let incidents_store = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let events_store = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let incidents = if let Some(ref store) = incidents_store {
        let rows = store
            .active_incidents(100)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        enrich_incident_rows_with_latest_traces(
            rows,
            incidents_store.as_ref(),
            events_store.as_ref(),
        )
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        vec![]
    };
    Ok(Json(json!({ "incidents": incidents })))
}

async fn api_incident_detail(
    State(state): State<AppState>,
    AxumPath(incident_id): AxumPath<String>,
) -> Result<Json<IncidentDetailResponse>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let incidents = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Incident store not found".to_string(),
            )
        })?;
    let events = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Event store not found".to_string()))?;
    let incident = incidents
        .get_incident(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Incident not found".to_string()))?;
    let event_ids = incidents
        .incident_event_ids(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let event_rows = events
        .get_events(&event_ids)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let hypotheses = incidents
        .hypotheses(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let clusters = incidents
        .clusters(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let explanation = incidents
        .latest_explanation(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let latest_trace = incidents
        .latest_ai_trace(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let state_log = incidents
        .list_state_log(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let feedback = incidents
        .list_feedback(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let learning_provenance = aggregate_hypothesis_provenance(&hypotheses);
    Ok(Json(IncidentDetailResponse {
        incident,
        events: event_rows,
        hypotheses,
        clusters,
        explanation,
        latest_trace,
        state_log,
        feedback,
        learning_provenance,
    }))
}

async fn api_incident_events(
    State(state): State<AppState>,
    AxumPath(incident_id): AxumPath<String>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let incidents = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Incident store not found".to_string(),
            )
        })?;
    let events = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Event store not found".to_string()))?;
    incidents
        .get_incident(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Incident not found".to_string()))?;
    let event_ids = incidents
        .incident_event_ids(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let event_rows = events
        .get_events(&event_ids)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "events": event_rows })))
}

async fn api_incident_logs(
    State(state): State<AppState>,
    AxumPath(incident_id): AxumPath<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let retention = storage_retention_hours(&cfg);
    let log_fts_enabled = observability_logs_fts_enabled(&cfg);
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 2000);
    let min_severity = params
        .get("severity")
        .and_then(|value| value.parse::<i64>().ok())
        .map(|value| value.clamp(0, 4));
    let search = params.get("search").cloned();
    let fts_query = params.get("q").cloned();
    let trace_id = params.get("trace_id").cloned();
    let incidents = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Incident store not found".to_string(),
            )
        })?;
    let events = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Event store not found".to_string()))?;
    let incident = incidents
        .get_incident(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Incident not found".to_string()))?;
    let event_ids = incidents
        .incident_event_ids(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut base_query = LogsQuery {
        limit,
        retention_hours: retention,
        min_severity,
        search,
        fts_query,
        log_fts_enabled,
        trace_id,
        timestamp_after: params.get("start").cloned(),
        timestamp_before: params.get("end").cloned(),
        cursor_timestamp: params.get("cursor_timestamp").cloned(),
        cursor_event_id: params.get("cursor_event_id").cloned(),
        source_type: params.get("source_type").cloned(),
        attr_key: params.get("attr_key").cloned(),
        attr_value: params.get("attr_value").cloned(),
        ..Default::default()
    };
    let logs = if !event_ids.is_empty() {
        events
            .query_logs_for_event_ids(&event_ids, &base_query)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        if base_query.service_id.is_none() && !incident.primary_service.trim().is_empty() {
            base_query.service_id = Some(incident.primary_service.clone());
        }
        events
            .query_logs(&base_query)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };
    let next_cursor = if logs.len() == limit {
        logs.last().map(|row| {
            json!({
                "cursor_timestamp": row.timestamp,
                "cursor_event_id": row.event_id,
            })
        })
    } else {
        None
    };
    Ok(Json(json!({
        "incident_id": incident_id,
        "logs": logs,
        "limit": limit,
        "retention_hours": retention,
        "next_cursor": next_cursor,
    })))
}

async fn api_incident_hypotheses(
    State(state): State<AppState>,
    AxumPath(incident_id): AxumPath<String>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let incidents = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Incident store not found".to_string(),
            )
        })?;
    incidents
        .get_incident(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Incident not found".to_string()))?;
    let hypotheses = incidents
        .hypotheses(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "hypotheses": hypotheses })))
}

async fn api_incident_clusters(
    State(state): State<AppState>,
    AxumPath(incident_id): AxumPath<String>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let incidents = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Incident store not found".to_string(),
            )
        })?;
    incidents
        .get_incident(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Incident not found".to_string()))?;
    let clusters = incidents
        .clusters(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "clusters": clusters })))
}

async fn api_incident_feedback(
    State(state): State<AppState>,
    AxumPath(incident_id): AxumPath<String>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let store = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "Incident store not found".to_string(),
            )
        })?;
    let Some(_incident) = store
        .get_incident(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    else {
        return Err((StatusCode::NOT_FOUND, "Incident not found".to_string()));
    };
    let feedback_type = payload
        .get("feedback_type")
        .and_then(JsonValue::as_str)
        .unwrap_or("skipped")
        .trim()
        .to_string();
    if !matches!(
        feedback_type.as_str(),
        "confirmed" | "none_correct" | "skipped"
    ) {
        return Err((
            StatusCode::BAD_REQUEST,
            "feedback_type must be one of confirmed, none_correct, skipped".to_string(),
        ));
    }
    let correct_hypothesis_id = payload
        .get("correct_hypothesis_id")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let operator_notes = payload
        .get("operator_notes")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();
    let resolved_at = payload
        .get("resolved_at")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(now_iso);
    let feedback_id = format!(
        "fb-{}-{}",
        incident_id,
        OffsetDateTime::now_utc().unix_timestamp_nanos()
    );
    store
        .add_feedback(&StoredFeedback {
            feedback_id: feedback_id.clone(),
            incident_id: incident_id.clone(),
            correct_hypothesis_id,
            feedback_type: feedback_type.clone(),
            operator_notes,
            resolved_at: resolved_at.clone(),
            created_at: Some(now_iso()),
        })
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let refreshed = refresh_incident_reasoning(&cfg, state.paths.as_ref(), &incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let feedback = store
        .list_feedback(&incident_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "stored": true,
        "feedback_id": feedback_id,
        "incident_id": incident_id,
        "feedback_type": feedback_type,
        "resolved_at": resolved_at,
        "refreshed": refreshed,
        "feedback": feedback,
    })))
}

async fn api_adaptive_learning(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let payload = adaptive_learning_summary(&cfg, state.paths.as_ref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(payload))
}

async fn api_adaptive_learning_audit(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let offset = params
        .get("offset")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
        .min(10_000);
    let payload = adaptive_learning_audit_log(
        &cfg,
        state.paths.as_ref(),
        &AdaptiveLearningAuditQuery {
            artifact_kind: params
                .get("artifact_kind")
                .cloned()
                .filter(|value| !value.is_empty()),
            artifact_id: params
                .get("artifact_id")
                .cloned()
                .filter(|value| !value.is_empty()),
            action: params
                .get("action")
                .cloned()
                .filter(|value| !value.is_empty()),
            review_status_after: params
                .get("review_status")
                .cloned()
                .filter(|value| !value.is_empty()),
            limit,
            offset,
        },
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(payload))
}

async fn api_adaptive_learning_history(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 1000);
    let offset = params
        .get("offset")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0)
        .min(10_000);
    let payload = adaptive_learning_history(
        &cfg,
        state.paths.as_ref(),
        &AdaptiveLearningHistoryQuery {
            artifact_kind: params
                .get("artifact_kind")
                .cloned()
                .filter(|value| !value.is_empty()),
            artifact_id: params
                .get("artifact_id")
                .cloned()
                .filter(|value| !value.is_empty()),
            incident_id: params
                .get("incident_id")
                .cloned()
                .filter(|value| !value.is_empty()),
            cause_type: params
                .get("cause_type")
                .cloned()
                .filter(|value| !value.is_empty()),
            limit,
            offset,
        },
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(payload))
}

async fn api_adaptive_learning_review(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let payload = adaptive_learning_review_summary(&cfg, state.paths.as_ref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(payload))
}

async fn api_adaptive_learning_review_action(
    State(state): State<AppState>,
    AxumPath((artifact_kind, artifact_id)): AxumPath<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let decision = payload
        .get("decision")
        .or_else(|| payload.get("action"))
        .and_then(JsonValue::as_str)
        .unwrap_or("approve");
    let reason = payload.get("reason").and_then(JsonValue::as_str);
    let review = adaptive_learning_review_artifact(
        &cfg,
        state.paths.as_ref(),
        &artifact_kind,
        &artifact_id,
        decision,
        reason,
    )
    .map_err(classify_adaptive_learning_error)?;
    Ok(Json(json!({
        "updated": true,
        "artifact_kind": artifact_kind,
        "artifact_id": artifact_id,
        "decision": decision,
        "review": review,
    })))
}

async fn api_adaptive_learning_save_view(
    State(state): State<AppState>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let draft = parse_adaptive_review_view_draft(&payload)?;
    let response = adaptive_learning_save_review_view(&cfg, state.paths.as_ref(), &draft)
        .map_err(classify_adaptive_learning_error)?;
    Ok(Json(response))
}

async fn api_adaptive_learning_delete_view(
    State(state): State<AppState>,
    AxumPath(view_id): AxumPath<String>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let response = adaptive_learning_delete_review_view(&cfg, state.paths.as_ref(), &view_id)
        .map_err(classify_adaptive_learning_error)?;
    Ok(Json(response))
}

async fn api_adaptive_learning_use_view(
    State(state): State<AppState>,
    AxumPath(view_id): AxumPath<String>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let response = adaptive_learning_touch_review_view(&cfg, state.paths.as_ref(), &view_id)
        .map_err(classify_adaptive_learning_error)?;
    Ok(Json(response))
}

async fn api_adaptive_learning_bulk_review_action(
    State(state): State<AppState>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let decision = payload
        .get("decision")
        .or_else(|| payload.get("action"))
        .and_then(JsonValue::as_str)
        .unwrap_or("approve");
    let reason = payload.get("reason").and_then(JsonValue::as_str);
    let artifacts = parse_adaptive_bulk_artifacts(&payload)?;
    let review = adaptive_learning_bulk_review_artifacts(
        &cfg,
        state.paths.as_ref(),
        &artifacts,
        decision,
        reason,
    )
    .map_err(classify_adaptive_learning_error)?;
    Ok(Json(review))
}

async fn api_adaptive_learning_action(
    State(state): State<AppState>,
    AxumPath((artifact_kind, artifact_id)): AxumPath<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let action = payload
        .get("action")
        .and_then(JsonValue::as_str)
        .unwrap_or("disable");
    let reason = payload.get("reason").and_then(JsonValue::as_str);
    let learning = adaptive_learning_set_artifact_state(
        &cfg,
        state.paths.as_ref(),
        &artifact_kind,
        &artifact_id,
        action,
        reason,
    )
    .map_err(classify_adaptive_learning_error)?;
    Ok(Json(json!({
        "updated": true,
        "artifact_kind": artifact_kind,
        "artifact_id": artifact_id,
        "action": action,
        "learning": learning,
    })))
}

async fn api_adaptive_learning_bulk_state_action(
    State(state): State<AppState>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let cfg = state.config.read().await.clone();
    let action = payload
        .get("action")
        .and_then(JsonValue::as_str)
        .unwrap_or("disable");
    let reason = payload.get("reason").and_then(JsonValue::as_str);
    let artifacts = parse_adaptive_bulk_artifacts(&payload)?;
    let review = adaptive_learning_bulk_set_artifact_state(
        &cfg,
        state.paths.as_ref(),
        &artifacts,
        action,
        reason,
    )
    .map_err(classify_adaptive_learning_error)?;
    Ok(Json(review))
}

fn parse_adaptive_bulk_artifacts(
    payload: &JsonValue,
) -> Result<Vec<AdaptiveArtifactSelection>, (StatusCode, String)> {
    let items = payload
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "expected JSON body with an artifacts array".to_string(),
            )
        })?;
    let mut selections = Vec::new();
    for item in items {
        let artifact_kind = item
            .get("artifact_kind")
            .or_else(|| item.get("kind"))
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "each bulk artifact must include artifact_kind".to_string(),
                )
            })?;
        let artifact_id = item
            .get("artifact_id")
            .or_else(|| item.get("id"))
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "each bulk artifact must include artifact_id".to_string(),
                )
            })?;
        selections.push(AdaptiveArtifactSelection {
            artifact_kind: artifact_kind.to_string(),
            artifact_id: artifact_id.to_string(),
        });
    }
    if selections.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "expected at least one adaptive artifact selection".to_string(),
        ));
    }
    Ok(selections)
}

fn parse_adaptive_review_view_draft(
    payload: &JsonValue,
) -> Result<AdaptiveSavedReviewViewDraft, (StatusCode, String)> {
    let name = payload
        .get("name")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "saved review view name must not be empty".to_string(),
            )
        })?;
    let artifact_selections = payload
        .get("artifacts")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(AdaptiveArtifactSelection {
                        artifact_kind: item
                            .get("artifact_kind")
                            .or_else(|| item.get("kind"))?
                            .as_str()?
                            .to_string(),
                        artifact_id: item
                            .get("artifact_id")
                            .or_else(|| item.get("id"))?
                            .as_str()?
                            .to_string(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(AdaptiveSavedReviewViewDraft {
        view_id: payload
            .get("view_id")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        name: name.to_string(),
        description: payload
            .get("description")
            .and_then(JsonValue::as_str)
            .map(str::to_string),
        search_text: payload
            .get("search_text")
            .or_else(|| payload.get("search"))
            .and_then(JsonValue::as_str)
            .map(str::to_string),
        assigned_reviewer: payload
            .get("assigned_reviewer")
            .or_else(|| payload.get("reviewer"))
            .and_then(JsonValue::as_str)
            .map(str::to_string),
        artifact_selections,
    })
}

fn classify_adaptive_learning_error(error: anyhow::Error) -> (StatusCode, String) {
    let message = error.to_string();
    if message.contains("not found") {
        (StatusCode::NOT_FOUND, message)
    } else if message.contains("unsupported") {
        (StatusCode::BAD_REQUEST, message)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

fn aggregate_hypothesis_provenance(
    hypotheses: &[inferra_contracts::HypothesisRow],
) -> Option<JsonValue> {
    let mut artifacts = std::collections::BTreeMap::<String, JsonValue>::new();
    let mut influenced_hypotheses = 0usize;
    let mut estimated_total_impact = 0.0;
    for hypothesis in hypotheses {
        let Some(provenance) = hypothesis.provenance.as_ref() else {
            continue;
        };
        let has_learning = provenance
            .get("has_learned_influence")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        if has_learning {
            influenced_hypotheses += 1;
        }
        estimated_total_impact += provenance
            .get("estimated_total_impact")
            .and_then(JsonValue::as_f64)
            .unwrap_or_default();
        if let Some(items) = provenance.get("artifacts").and_then(JsonValue::as_array) {
            for item in items {
                let key = format!(
                    "{}:{}",
                    item.get("kind")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("unknown"),
                    item.get("artifact_id")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("unknown"),
                );
                let impact = item
                    .get("impact_value")
                    .and_then(JsonValue::as_f64)
                    .unwrap_or_default();
                let entry = artifacts.entry(key).or_insert_with(|| {
                    json!({
                        "kind": item.get("kind").cloned().unwrap_or(JsonValue::Null),
                        "artifact_id": item.get("artifact_id").cloned().unwrap_or(JsonValue::Null),
                        "label": item.get("label").cloned().unwrap_or(JsonValue::Null),
                        "impact_metric": item.get("impact_metric").cloned().unwrap_or(JsonValue::Null),
                        "cumulative_impact": 0.0,
                    })
                });
                let current = entry
                    .get("cumulative_impact")
                    .and_then(JsonValue::as_f64)
                    .unwrap_or_default();
                entry["cumulative_impact"] = json!(current + impact);
            }
        }
    }
    let mut artifacts = artifacts.into_values().collect::<Vec<_>>();
    artifacts.sort_by(|left, right| {
        right["cumulative_impact"]
            .as_f64()
            .unwrap_or_default()
            .partial_cmp(&left["cumulative_impact"].as_f64().unwrap_or_default())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if artifacts.is_empty() {
        None
    } else {
        Some(json!({
            "influenced_hypotheses": influenced_hypotheses,
            "estimated_total_impact": estimated_total_impact,
            "artifacts": artifacts,
        }))
    }
}

async fn api_services(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let overview = build_overview(&cfg, state.paths.as_ref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let payload = json!({
        "services": overview.dashboard.services.unwrap_or_default(),
    });
    let _ = persist_ui_snapshot(
        state.paths.as_ref(),
        SNAPSHOT_SYSTEMS,
        &payload,
        UI_SNAPSHOT_SOURCE_CORE,
        None,
    );
    Ok(Json(payload))
}

async fn api_service_detail(
    State(state): State<AppState>,
    AxumPath(service_id): AxumPath<String>,
) -> Result<Json<ServiceDetailResponse>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let overview = build_overview(&cfg, state.paths.as_ref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let services = overview.dashboard.services.unwrap_or_default();
    let service = services
        .into_iter()
        .find(|item| item.service_id == service_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Service not found".to_string()))?;
    let events = if let Some(store) = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        store
            .events_for_service(&service_id, 200)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        vec![]
    };
    let incidents: Vec<IncidentRow> = overview
        .dashboard
        .incidents
        .unwrap_or_default()
        .into_iter()
        .filter(|incident| {
            incident.primary_service == service_id
                || incident
                    .affected_services
                    .as_ref()
                    .map(|items| items.iter().any(|item| item == &service_id))
                    .unwrap_or(false)
        })
        .collect();
    Ok(Json(ServiceDetailResponse {
        service,
        events,
        incidents,
    }))
}

async fn api_service_events(
    State(state): State<AppState>,
    AxumPath(service_id): AxumPath<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 500);
    let store = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "Event store not found".to_string()))?;
    let events = store
        .events_for_service(&service_id, limit)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "events": events })))
}

async fn api_ai_status(State(state): State<AppState>) -> Json<AiStatusResponse> {
    let cfg = state.config.read().await.clone();
    let mut status = ai_status_from_config(&cfg);
    let ai = cfg.get("ai").and_then(|v| v.as_table());
    status.status_model = Some(resolve_status_probe_model(ai));
    status.investigate_model = Some(resolve_investigate_probe_model(ai));
    let probe = probe_ai_provider(&cfg, AiProbePurpose::Status).await;
    status.enabled = probe.enabled;
    status.provider = Some(probe.provider.clone());
    status.base_url = Some(probe.base_url.clone());
    status.model = Some(probe.model.clone());
    status.resolved_model = probe
        .resolved_model
        .clone()
        .or_else(|| (!probe.model.is_empty()).then_some(probe.model.clone()));
    status.available = Some(probe.available);
    status.installed = Some(probe.installed);
    status.reason = probe.reason.clone();
    status.error = probe.error.clone();
    status.allow_remote = Some(probe.allow_remote);
    let _ = persist_ui_snapshot(
        state.paths.as_ref(),
        SNAPSHOT_AI_STATUS,
        &serde_json::to_value(&status).unwrap_or_else(|_| json!({})),
        UI_SNAPSHOT_SOURCE_CORE,
        None,
    );
    Json(status)
}

async fn api_ai_doctor(
    State(state): State<AppState>,
) -> Result<Json<AiDoctorResponse>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let ai = cfg.get("ai").and_then(|v| v.as_table()).ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "ai config missing".to_string(),
        )
    })?;
    let enabled = ai.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    let allow_remote = ai
        .get("allow_remote")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let token_env = ai
        .get("token_env")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let redact_raw_logs = ai
        .get("redact_raw_logs")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut warnings = Vec::new();
    let probe = probe_ai_provider(&cfg, AiProbePurpose::Status).await;
    if enabled && !probe.available {
        warnings.push(
            probe
                .reason
                .clone()
                .unwrap_or_else(|| "AI provider is unavailable.".to_string()),
        );
    }
    if allow_remote && token_env.is_empty() {
        warnings.push("Remote provider allowed but no auth token env is configured.".to_string());
    }
    if !redact_raw_logs {
        warnings.push(
            "Raw log redaction is disabled; remote providers may receive sensitive content."
                .to_string(),
        );
    }
    if enabled && !probe.installed {
        warnings.push(format!(
            "Configured model {} is not installed at {}.",
            probe.model, probe.base_url
        ));
    }
    let inv = resolve_investigate_probe_model(cfg.get("ai").and_then(|v| v.as_table()));
    let inv_probe = probe_ai_provider(&cfg, AiProbePurpose::Investigate).await;
    if enabled && inv != probe.model && !inv_probe.installed {
        warnings.push(format!(
            "Investigation model {inv} is not installed at {} (status probe uses {}).",
            probe.base_url, probe.model
        ));
    }
    Ok(Json(AiDoctorResponse {
        ok: !enabled || probe.available,
        enabled,
        provider: probe.provider,
        base_url: probe.base_url,
        model: probe
            .resolved_model
            .clone()
            .unwrap_or_else(|| probe.model.clone()),
        investigate_model: Some(inv),
        allow_remote,
        token_env_set: !token_env.is_empty(),
        redact_raw_logs,
        available: probe.available,
        warnings,
        guidance: Some(vec![
            "AI is presentation-only; deterministic scores are never silently changed.".into(),
            "All AI suggestions are read-only; no command is executed automatically.".into(),
        ]),
    }))
}

async fn api_ai_generations(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let store = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "incidents database missing".into()))?;
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(50)
        .clamp(1, 200);
    let generations = store
        .list_ai_generations(params.get("scope").map(String::as_str), limit)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let count = generations.len();
    Ok(Json(json!({
        "generations": generations,
        "count": count,
    })))
}

async fn api_investigate_now(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let mode = current_mode(&cfg, params.get("mode").map(String::as_str));
    let focus = "overview".to_string();
    let monitor = resolve_monitor_seconds(&cfg, params.get("monitor_seconds"), None);
    let use_cache = !query_bool(&params, "force").unwrap_or(false);
    let bundle =
        investigation_bundle_enriched(state.paths.as_ref(), &cfg, &focus, "", &mode, monitor)
            .await
            .map_err(|e| (investigation_status(&e), e.to_string()))?;
    investigation_response_for_bundle(
        state.paths.as_ref(),
        &cfg,
        bundle,
        &focus,
        &mode,
        None,
        false,
        use_cache,
    )
    .await
}

async fn api_ai_ask(
    State(state): State<AppState>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let focus = resolve_focus_from_scope(
        state.paths.as_ref(),
        payload
            .get("scope")
            .and_then(|v| v.as_str())
            .unwrap_or("overview"),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let question = payload
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if question.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "'question' is required".into()));
    }
    let mode = current_mode(&cfg, payload.get("mode").and_then(|v| v.as_str()));
    let monitor = resolve_monitor_seconds(&cfg, None, Some(&payload));
    let use_cache = !payload
        .get("force")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let bundle = investigation_bundle_enriched(
        state.paths.as_ref(),
        &cfg,
        &focus,
        &question,
        &mode,
        monitor,
    )
    .await
    .map_err(|e| (investigation_status(&e), e.to_string()))?;
    investigation_response_for_bundle(
        state.paths.as_ref(),
        &cfg,
        bundle,
        &focus,
        &mode,
        Some(question),
        false,
        use_cache,
    )
    .await
}

async fn api_investigate_incident(
    State(state): State<AppState>,
    AxumPath(incident_id): AxumPath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let mode = current_mode(&cfg, params.get("mode").map(String::as_str));
    let focus = format!("incident:{incident_id}");
    let monitor = resolve_monitor_seconds(&cfg, params.get("monitor_seconds"), None);
    let use_cache = !query_bool(&params, "force").unwrap_or(false);
    let bundle =
        investigation_bundle_enriched(state.paths.as_ref(), &cfg, &focus, "", &mode, monitor)
            .await
            .map_err(|e| (investigation_status(&e), e.to_string()))?;
    investigation_response_for_bundle(
        state.paths.as_ref(),
        &cfg,
        bundle,
        &focus,
        &mode,
        None,
        false,
        use_cache,
    )
    .await
}

async fn api_investigate_service(
    State(state): State<AppState>,
    AxumPath(service_id): AxumPath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let mode = current_mode(&cfg, params.get("mode").map(String::as_str));
    let focus = format!("service:{service_id}");
    let monitor = resolve_monitor_seconds(&cfg, params.get("monitor_seconds"), None);
    let use_cache = !query_bool(&params, "force").unwrap_or(false);
    let bundle =
        investigation_bundle_enriched(state.paths.as_ref(), &cfg, &focus, "", &mode, monitor)
            .await
            .map_err(|e| (investigation_status(&e), e.to_string()))?;
    investigation_response_for_bundle(
        state.paths.as_ref(),
        &cfg,
        bundle,
        &focus,
        &mode,
        None,
        false,
        use_cache,
    )
    .await
}

async fn api_ai_report(
    State(state): State<AppState>,
    AxumPath(incident_id): AxumPath<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let mode = current_mode(&cfg, params.get("mode").map(String::as_str));
    let focus = format!("incident:{incident_id}");
    let monitor = resolve_monitor_seconds(&cfg, params.get("monitor_seconds"), None);
    let use_cache = !query_bool(&params, "force").unwrap_or(false);
    let bundle =
        investigation_bundle_enriched(state.paths.as_ref(), &cfg, &focus, "", &mode, monitor)
            .await
            .map_err(|e| (investigation_status(&e), e.to_string()))?;
    investigation_response_for_bundle(
        state.paths.as_ref(),
        &cfg,
        bundle,
        &focus,
        &mode,
        None,
        true,
        use_cache,
    )
    .await
}

async fn investigation_response_for_bundle(
    paths: &Paths,
    config: &TomlValue,
    bundle: JsonValue,
    focus: &str,
    mode: &str,
    question: Option<String>,
    report: bool,
    use_cache: bool,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let question_text = question.clone().unwrap_or_default();
    let scope_key = ai_generation_scope_key(focus, mode, &question_text, report);
    if use_cache {
        if let Some(mut saved) = load_saved_ai_generation(paths, &scope_key)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?
        {
            if let Some(mut response) = saved.get_mut("response").cloned() {
                if let Some(object) = saved.as_object_mut() {
                    object.remove("response");
                }
                response["cached"] = JsonValue::Bool(true);
                response["ai_generation"] = saved;
                return Ok(Json(response));
            }
        }
    }
    let mut response = run_investigation_response(paths, config, &bundle)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    if let Some(audit) = persist_investigation_artifacts(paths, &bundle, &response)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?
    {
        response["audit"] = audit;
    }
    response["focus"] = JsonValue::String(focus.to_string());
    response["mode"] = JsonValue::String(mode.to_string());
    if !question_text.is_empty() {
        response["question"] = JsonValue::String(question_text.clone());
    }
    if report {
        response["report"] = JsonValue::Bool(true);
    }
    let generation = persist_ai_generation(
        paths,
        &scope_key,
        focus,
        mode,
        &question_text,
        &bundle,
        &response,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
    response["ai_generation"] = ai_generation_metadata_json(&generation);
    let _ = persist_ui_snapshot(
        paths,
        SNAPSHOT_AI_INVESTIGATION,
        &json!({
            "focus": focus,
            "mode": mode,
            "response": response.clone(),
        }),
        UI_SNAPSHOT_SOURCE_CORE,
        None,
    );
    Ok(Json(response))
}

async fn api_ai_context_get(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let scope = params
        .get("scope")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "scope query parameter is required".into(),
            )
        })?;
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let store = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "incidents database missing".into()))?;
    let body = store
        .get_operator_context(scope)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .unwrap_or_default();
    Ok(Json(json!({ "scope": scope, "body": body })))
}

async fn api_ai_context_put(
    State(state): State<AppState>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let scope = payload
        .get("scope")
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "scope field is required".into()))?
        .to_string();
    let body = payload
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or("")
        .to_string();
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let store = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "incidents database missing".into()))?;
    store
        .set_operator_context(&scope, &body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "stored": true, "scope": scope })))
}

async fn api_ai_investigate_stream(
    State(state): State<AppState>,
    Json(payload): Json<JsonValue>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let paths = state.paths.clone();
    let cfg = state.config.read().await.clone();
    let stream = stream! {
        let scope = payload.get("scope").and_then(|s| s.as_str()).unwrap_or("overview");
        let focus = match resolve_focus_from_scope(paths.as_ref(), scope) {
            Ok(f) => f,
            Err(e) => {
                yield Ok(Event::default().event("error").data(e.to_string()));
                return;
            }
        };
        let question = payload
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let mode = current_mode(&cfg, payload.get("mode").and_then(|v| v.as_str()));
        let monitor = resolve_monitor_seconds(&cfg, None, Some(&payload));
        let bundle = match investigation_bundle_enriched(paths.as_ref(), &cfg, &focus, &question, &mode, monitor).await {
            Ok(b) => b,
            Err(e) => {
                yield Ok(Event::default().event("error").data(e.to_string()));
                return;
            }
        };
        yield Ok(Event::default().event("meta").data(
            json!({
                "focus": &focus,
                "mode": &mode,
                "monitor_seconds": monitor,
            })
            .to_string(),
        ));

        let redacted = redact_bundle_for_ai(&bundle, &cfg);
        let provider = probe_ai_provider(&cfg, AiProbePurpose::Investigate).await;
        if !provider.enabled || !provider.available {
            let reason = if !provider.enabled {
                "AI is disabled in config."
            } else {
                provider.reason.as_deref().unwrap_or("AI provider unavailable.")
            };
            let resp = deterministic_investigation_response(
                &redacted,
                provider.provider_payload(),
                reason,
                Vec::new(),
                1,
                Some(fallback_trace(
                    if !provider.enabled { "provider_disabled" } else { "provider_unavailable" },
                    reason,
                )),
            );
            let resp = finalize_ai_generation_response_lossy(
                paths.as_ref(),
                &bundle,
                resp,
                &focus,
                &mode,
                &question,
                false,
            );
            yield Ok(Event::default().event("done").data(resp.to_string()));
            return;
        }

        let mode_str = redacted.get("mode").and_then(JsonValue::as_str).unwrap_or("operator");
        let bundle_json = match serde_json::to_string(&redacted) {
            Ok(s) => s,
            Err(e) => {
                yield Ok(Event::default().event("error").data(e.to_string()));
                return;
            }
        };
        let user_prompt = INVESTIGATION_USER_TEMPLATE
            .replace("{mode}", mode_str)
            .replace("{bundle_json}", &bundle_json);
        let messages = json!([
            {"role": "system", "content": INVESTIGATION_SYSTEM_PROMPT},
            {"role": "user", "content": user_prompt},
        ]);
        let ai = cfg.get("ai").and_then(|v| v.as_table());
        let base_url = ai_table_string(ai, "base_url", "http://127.0.0.1:11434");
        let token_env = ai_table_string(ai, "token_env", "");
        let connect_timeout = ai_table_f64(ai, "connect_timeout_seconds", 5.0);
        let read_timeout = ai_table_f64(ai, "read_timeout_seconds", 120.0);
        let client = match reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs_f64(connect_timeout.max(0.1)))
            .timeout(std::time::Duration::from_secs_f64(read_timeout.max(30.0)))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                yield Ok(Event::default().event("error").data(e.to_string()));
                return;
            }
        };
        let url = format!("{}/api/chat", base_url.trim_end_matches('/'));
        let ollama_body = json!({
            "model": provider.resolved_model.clone().unwrap_or_else(|| provider.model.clone()),
            "messages": messages,
            "stream": true,
            "options": {
                "temperature": ai_table_f64(ai, "temperature", 1.0),
                "top_p": ai_table_f64(ai, "top_p", 0.95),
                "top_k": ai_table_u64(ai, "top_k", 64) as i64,
                "num_predict": ai_table_u64(ai, "max_tokens", 2048) as i64,
            }
        });
        let mut request = client.post(&url).json(&ollama_body);
        if !token_env.is_empty() {
            if let Ok(token) = std::env::var(&token_env) {
                request = request.bearer_auth(token);
            }
        }
        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                yield Ok(Event::default().event("error").data(e.to_string()));
                return;
            }
        };
        if !response.status().is_success() {
            let t = response.text().await.unwrap_or_default();
            yield Ok(Event::default().event("error").data(format!("Ollama HTTP error: {t}")));
            return;
        }
        let mut assembled = String::new();
        let mut buf = String::new();
        let mut body_stream = response.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = body_stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    yield Ok(Event::default().event("error").data(e.to_string()));
                    return;
                }
            };
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf.drain(..=pos);
                if line.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<JsonValue>(&line) else { continue };
                if let Some(p) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                {
                    if !p.is_empty() {
                        assembled.push_str(p);
                        yield Ok(Event::default().event("delta").data(p));
                    }
                }
            }
        }
        let parsed = extract_json_object(&assembled);
        let response = if let Some(mut output) = parsed.and_then(normalize_investigation_output) {
            if output_has_signal(&output) {
                let trace = json!({
                    "trace_kind": "investigate_stream",
                    "sanitized_system_prompt": INVESTIGATION_SYSTEM_PROMPT,
                    "sanitized_user_prompt": user_prompt,
                    "allowed_fields": ["mode", "incident", "hypotheses", "events", "services", "runtime", "workspace", "user_question", "constraints", "runtime_monitor", "host_resources", "evidence_digest", "similar_incidents", "operator_memory"],
                    "blocked_fields": ["raw_event_messages", "env_values", "ip_addresses", "secrets"],
                    "raw_logs_sent": raw_workspace_logs_sent_to_ai(&bundle, &cfg),
                    "schema_version": 1,
                });
                let grounding = apply_output_grounding(&mut output, &redacted);
                let mut warnings = Vec::new();
                if let Some(w) = hypothesis_rank_alignment_warning(&output, &redacted) {
                    warnings.push(w);
                }
                json!({
                    "schema_version": 1,
                    "output": output,
                    "used_ai": true,
                    "fallback_reason": "",
                    "warnings": warnings,
                    "attempts": 1,
                    "provider": provider.provider_payload(),
                    "trace": trace,
                    "bundle": redacted,
                    "grounding": grounding,
                })
            } else {
                deterministic_investigation_response(
                    &redacted,
                    provider.provider_payload(),
                    "AI returned an investigation payload without meaningful content",
                    vec!["stream_parse: empty signal".into()],
                    1,
                    None,
                )
            }
        } else {
            deterministic_investigation_response(
                &redacted,
                provider.provider_payload(),
                "AI stream did not yield valid JSON",
                vec!["stream_parse: invalid json".into()],
                1,
                None,
            )
        };
        let response = finalize_ai_generation_response_lossy(
            paths.as_ref(),
            &bundle,
            response,
            &focus,
            &mode,
            &question,
            false,
        );
        yield Ok(Event::default().event("done").data(response.to_string()));
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn api_collectors(State(state): State<AppState>) -> Json<CollectorsResponse> {
    let cfg = state.config.read().await.clone();
    let runtime_rows = state.collectors.collector_rows(&cfg).await;
    let runtime_by_id: std::collections::HashMap<_, _> = runtime_rows
        .into_iter()
        .map(|row| (row.collector_id.clone(), row))
        .collect();
    let response = CollectorsResponse {
        collectors: configured_collectors(&cfg)
            .into_iter()
            .map(|c| {
                let runtime = runtime_by_id.get(&c.collector_id);
                let source_type = runtime
                    .map(|row| row.source_type.clone())
                    .unwrap_or_else(|| c.source_type.clone());
                let last_error = runtime.and_then(|row| row.last_error.clone());
                CollectorRow {
                    collector_id: c.collector_id.clone(),
                    status: runtime.map(|row| row.status.clone()).or(Some(c.status)),
                    source_type: Some(source_type.clone()),
                    is_running: runtime.map(|row| row.is_running),
                    events_emitted: runtime.map(|row| row.events_emitted),
                    events_per_second: runtime.map(|row| row.events_per_second),
                    last_event_at: runtime.and_then(|row| row.last_event_at.clone()),
                    error_count: runtime.map(|row| row.error_count),
                    dropped_events: runtime.map(|row| row.dropped_events),
                    last_error: last_error.clone(),
                    last_error_at: runtime.and_then(|row| row.last_error_at.clone()),
                    error_hint: collector_error_hint(
                        &c.collector_id,
                        Some(source_type.as_str()),
                        last_error.as_deref(),
                    ),
                    log_query: Some(format!("/api/logs?search={}&limit=100", c.collector_id)),
                    lag_seconds: runtime.and_then(|row| row.lag_seconds),
                }
            })
            .collect(),
        queue_depth: state.collectors.queue_depth(),
    };
    let _ = persist_ui_snapshot(
        state.paths.as_ref(),
        SNAPSHOT_CONTROL,
        &serde_json::to_value(&response).unwrap_or_else(|_| json!({})),
        UI_SNAPSHOT_SOURCE_CORE,
        None,
    );
    Json(response)
}

fn collector_error_hint(
    collector_id: &str,
    source_type: Option<&str>,
    last_error: Option<&str>,
) -> Option<String> {
    let has_error = last_error
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let lower = has_error.to_ascii_lowercase();
    if lower.contains("access is denied")
        || lower.contains("permission denied")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
    {
        return Some(
            "The collector is being blocked by permissions. Run Inferra with the needed OS/runtime permissions or narrow this collector's configured scope."
                .into(),
        );
    }
    if lower.contains("no such file")
        || lower.contains("cannot find")
        || lower.contains("not found")
        || lower.contains("does not exist")
    {
        return Some(match collector_id {
            "file" | "linux_syslog" => {
                "A configured log path is missing or unreachable. Check the collector paths in inferra.toml and remove stale targets.".into()
            }
            "journald" => {
                "The journald collector cannot find journalctl or journal data on this host. Disable it on non-systemd hosts or install journalctl.".into()
            }
            "kubernetes" => {
                "The Kubernetes collector cannot find kubectl or cluster data. Check kubectl installation, kubeconfig, and cluster access.".into()
            }
            _ => "A required collector resource was not found. Check this collector's configuration and host dependencies.".into(),
        });
    }
    match collector_id {
        "docker" => Some(
            "Docker collection depends on a reachable Docker daemon or socket. Start Docker Desktop/daemon if this host uses Docker, or disable the Docker collector for this host."
                .into(),
        ),
        "kubernetes" => Some(
            "Kubernetes collection depends on kubectl, kubeconfig, and RBAC permissions for pods/events in the configured namespaces."
                .into(),
        ),
        "windows_eventlog" => Some(
            "Windows Event Log collection can fail when channels are missing or restricted. Verify configured channels and run with sufficient permissions."
                .into(),
        ),
        "windows_service" => Some(
            "Windows service collection can fail when service queries are restricted. Verify service names and local service-control permissions."
                .into(),
        ),
        "journald" => Some(
            "Journald collection requires journalctl and readable systemd journal data. Disable it on Windows/non-systemd hosts."
                .into(),
        ),
        "linux_syslog" => Some(
            "Linux syslog collection requires readable syslog files. Disable it on Windows or update paths for this host."
                .into(),
        ),
        "file" => Some(
            "File collection tails configured paths. Confirm each path exists, is readable, and is not locked by another process."
                .into(),
        ),
        "app" if lower.contains("address already in use") || lower.contains("bind") => Some(
            "The app ingest listener could not bind its configured address. Change the app collector port or stop the process using it."
                .into(),
        ),
        _ => source_type.map(|source| {
            format!(
                "The {source} collector reported an error. Copy the collector report and inspect the log query linked for this collector."
            )
        }),
    }
}

async fn api_collectors_start(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let current = state.config.read().await.clone();
    if let Ok(next) = apply_config_put(current, &json!({ "collectors": { "auto_start": true } })) {
        let config_path = state.paths.config_path.clone();
        let next_for_write = next.clone();
        let _ = tokio::task::spawn_blocking(move || inferra_config::write_config(&config_path, &next_for_write))
            .await;
        *state.config.write().await = next;
    }
    let cfg = state.config.read().await.clone();
    state
        .collectors
        .start(
            &cfg,
            state.paths.events_db.clone(),
            state.paths.incidents_db.clone(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "started": true, "desired_state": "running" })))
}

async fn api_collectors_stop(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let current = state.config.read().await.clone();
    if let Ok(next) = apply_config_put(current, &json!({ "collectors": { "auto_start": false } })) {
        let config_path = state.paths.config_path.clone();
        let next_for_write = next.clone();
        let _ = tokio::task::spawn_blocking(move || inferra_config::write_config(&config_path, &next_for_write))
            .await;
        *state.config.write().await = next;
    }
    state
        .collectors
        .stop()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "stopped": true, "desired_state": "stopped" })))
}

async fn api_scanner_status(State(state): State<AppState>) -> Json<JsonValue> {
    let cfg = state.config.read().await.clone();
    let workspace_interval = workspace_scan_interval_seconds(&cfg);
    let cache = state.scanner_cache.read().await;
    let workspace = cache.workspace.as_ref();
    let db_workspace = read_ui_snapshot(state.paths.as_ref(), SNAPSHOT_WORKSPACE)
        .ok()
        .flatten();
    let db_snapshots = read_ui_snapshots(state.paths.as_ref()).unwrap_or_default();
    let age_seconds = workspace
        .map(|cached| cached.cached_at.elapsed().as_secs())
        .or_else(|| {
            db_workspace
                .as_ref()
                .and_then(|snapshot| ui_snapshot_age_seconds(&snapshot.updated_at))
        })
        .unwrap_or_default();
    Json(json!({
        "scanner": {
            "workspace": {
                "data_type": "workspace",
                "mode": "database_snapshot",
                "interval_seconds": workspace_interval,
                "min_interval_seconds": SCANNER_WORKSPACE_MIN_INTERVAL_SECONDS,
                "max_interval_seconds": SCANNER_WORKSPACE_MAX_INTERVAL_SECONDS,
                "last_scanned_at": db_workspace
                    .as_ref()
                    .map(|snapshot| snapshot.updated_at.clone())
                    .or_else(|| workspace.map(|cached| format_offset_datetime(cached.scanned_at))),
                "age_seconds": age_seconds,
                "next_scan_in_seconds": workspace
                    .map(|cached| cached.interval_seconds.saturating_sub(age_seconds))
                    .or_else(|| Some(workspace_interval.saturating_sub(age_seconds)))
                    .unwrap_or(0),
                "cached": workspace.is_some() || db_workspace.is_some(),
                "route": "/api/workspace/map"
            },
            "events": { "data_type": "events", "mode": "database_table", "route": "/api/logs" },
            "incidents": { "data_type": "incidents", "mode": "database_table", "route": "/api/incidents" },
            "services": { "data_type": "services", "mode": "database_projection", "route": "/api/services" },
            "evidence": { "data_type": "evidence", "mode": "database_table", "route": "/api/events" },
            "graph": { "data_type": "graph", "mode": "database_snapshot", "route": "/api/topology" },
            "ai": { "data_type": "ai", "mode": "database_snapshot", "route": "/api/ai/status" },
            "learning_review": { "data_type": "learning_review", "mode": "database_table", "route": "/api/adaptive-learning/review" },
            "control": { "data_type": "control", "mode": "database_snapshot", "route": "/api/collectors" },
            "settings": { "data_type": "settings", "mode": "database_snapshot", "route": "/api/config" }
        },
        "snapshots": db_snapshots
    }))
}

async fn api_scanner_run(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let workspace = refresh_workspace_scan_cache(&state, &cfg)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "ok": true,
        "workspace": {
            "projects": workspace.projects.len(),
            "runtime_apps": workspace.runtime_apps.len(),
            "service_mappings": workspace.service_mappings.len()
        }
    })))
}

async fn api_ingest(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let accepted = ingest_payload(&state, &headers, payload).await?;
    Ok(Json(accepted))
}

async fn api_otlp_logs(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: Body,
) -> Response {
    let cfg = state.config.read().await.clone();
    if !observability_otlp_logs_enabled(&cfg) {
        return (StatusCode::NOT_FOUND, "otlp logs ingest is disabled").into_response();
    }
    let max_payload = observability_otlp_max_payload_bytes(&cfg);
    let max_records = observability_otlp_max_logs_per_request(&cfg);
    let ingest = app_ingest_config(&cfg);
    if !ingest.shared_token.is_empty() {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let expected = format!("Bearer {}", ingest.shared_token);
        if auth != expected {
            return (StatusCode::UNAUTHORIZED, "missing or invalid bearer token").into_response();
        }
    }
    let ct = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_protobuf =
        ct.contains("application/x-protobuf") || ct.contains("application/protobuf");
    if ct.contains("application/grpc") {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Json(json!({
                "partialSuccess": {
                    "rejectedLogRecords": 0u64,
                    "errorMessage": "OTLP gRPC is not supported on /v1/logs; use Content-Type: application/json or application/x-protobuf with ExportLogsServiceRequest"
                }
            })),
        )
            .into_response();
    }
    let bytes = match axum::body::to_bytes(body, max_payload).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("failed to read OTLP body: {error}"),
            )
                .into_response();
        }
    };
    let payload: JsonValue = if is_protobuf {
        match normalize_otlp_logs_protobuf_request(&bytes) {
            Ok(payload) => payload,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "partialSuccess": {
                            "rejectedLogRecords": 0u64,
                            "errorMessage": format!("invalid OTLP protobuf: {error}")
                        }
                    })),
                )
                    .into_response();
            }
        }
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(payload) => payload,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "partialSuccess": {
                            "rejectedLogRecords": 0u64,
                            "errorMessage": format!("invalid OTLP JSON: {error}")
                        }
                    })),
                )
                    .into_response();
            }
        }
    };
    if let Err(error) =
        initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
    {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            error.to_string(),
        )
            .into_response();
    }
    let ingest_result = match state
        .collectors
        .ingest_otlp_logs_json(
            &state.paths.events_db,
            &state.paths.incidents_db,
            &cfg,
            &payload,
            max_records,
        )
        .await
    {
        Ok(result) => result,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                error.to_string(),
            )
                .into_response();
        }
    };
    let msg = if ingest_result.inserted == 0 && ingest_result.rejected_log_records > 0 {
        "No log records were stored (empty batch, parse errors, over limit, or governance suppression)"
    } else if ingest_result.rejected_log_records > 0 {
        "Some log records were rejected or suppressed; see rejectedLogRecords"
    } else {
        ""
    };
    (
        StatusCode::OK,
        Json(json!({
            "partialSuccess": {
                "rejectedLogRecords": ingest_result.rejected_log_records,
                "errorMessage": msg
            }
        })),
    )
        .into_response()
}

async fn cached_workspace_map(
    state: &AppState,
    config: &TomlValue,
    force: bool,
) -> Result<WorkspaceMapResponse> {
    let interval_seconds = workspace_scan_interval_seconds(config);
    if !force {
        let cache = state.scanner_cache.read().await;
        if let Some(cached) = cache.workspace.as_ref() {
            if cached.cached_at.elapsed() < Duration::from_secs(interval_seconds) {
                let mut value = cached.value.clone();
                hydrate_workspace_runtime_app_traces(state.paths.as_ref(), &mut value)?;
                return Ok(value);
            }
        }
        drop(cache);
        if let Some(snapshot) = read_ui_snapshot(state.paths.as_ref(), SNAPSHOT_WORKSPACE)? {
            let snapshot_age = ui_snapshot_age_seconds(&snapshot.updated_at).unwrap_or(u64::MAX);
            if snapshot_age < interval_seconds {
                let mut value: WorkspaceMapResponse = serde_json::from_value(snapshot.payload.clone())
                    .context("deserialize workspace snapshot")?;
                hydrate_workspace_runtime_app_traces(state.paths.as_ref(), &mut value)?;
                state.scanner_cache.write().await.workspace = Some(CachedWorkspaceMap {
                    value: value.clone(),
                    scanned_at: parse_offset_datetime(&snapshot.updated_at)
                        .unwrap_or_else(OffsetDateTime::now_utc),
                    cached_at: Instant::now()
                        .checked_sub(Duration::from_secs(snapshot_age))
                        .unwrap_or_else(Instant::now),
                    interval_seconds,
                });
                return Ok(value);
            }
        }
    }
    refresh_workspace_scan_cache(state, config).await
}

fn workspace_app_service_candidates(
    app: &WorkspaceRuntimeApp,
    service_mappings: &[WorkspaceMapping],
) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Some(project_path) = app.project_path.as_deref() {
        for mapping in service_mappings
            .iter()
            .filter(|mapping| mapping.project_path == project_path)
        {
            if seen.insert(mapping.service_id.clone()) {
                out.push(mapping.service_id.clone());
            }
        }
    }
    if seen.insert(app.name.clone()) {
        out.push(app.name.clone());
    }
    if let Some(display_name) = app.display_name.as_ref() {
        let display_name = display_name.trim();
        if !display_name.is_empty() && seen.insert(display_name.to_string()) {
            out.push(display_name.to_string());
        }
    }
    out
}

fn workspace_app_source_candidates(app: &WorkspaceRuntimeApp) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if seen.insert(app.name.clone()) {
        out.push(app.name.clone());
    }
    if let Some(display_name) = app.display_name.as_ref() {
        let display_name = display_name.trim();
        if !display_name.is_empty() && seen.insert(display_name.to_string()) {
            out.push(display_name.to_string());
        }
    }
    out
}

fn workspace_app_events(
    store: &EventsStore,
    app: &WorkspaceRuntimeApp,
    service_mappings: &[WorkspaceMapping],
    retention_hours: i64,
    limit: usize,
) -> Result<Vec<EventRow>> {
    for service_id in workspace_app_service_candidates(app, service_mappings) {
        let rows = store.query_logs(&LogsQuery {
            limit,
            retention_hours,
            service_id: Some(service_id),
            ..Default::default()
        })?;
        if !rows.is_empty() {
            return Ok(rows);
        }
    }
    for source_type in workspace_app_source_candidates(app) {
        let rows = store.query_logs(&LogsQuery {
            limit,
            retention_hours,
            source_type: Some(source_type),
            ..Default::default()
        })?;
        if !rows.is_empty() {
            return Ok(rows);
        }
    }
    Ok(Vec::new())
}

fn workspace_app_latest_trace_summary(
    store: &EventsStore,
    app: &WorkspaceRuntimeApp,
    service_mappings: &[WorkspaceMapping],
) -> Result<Option<TraceSummary>> {
    let mut latest: Option<TraceSummary> = None;
    for service_id in workspace_app_service_candidates(app, service_mappings) {
        let Some(candidate) = store.latest_trace_summary_for_service(&service_id)? else {
            continue;
        };
        let replace = match latest.as_ref() {
            None => true,
            Some(current) => {
                let candidate_seen = candidate.last_seen_at.as_deref().unwrap_or("");
                let current_seen = current.last_seen_at.as_deref().unwrap_or("");
                candidate_seen > current_seen
                    || (candidate_seen == current_seen && candidate.event_count > current.event_count)
            }
        };
        if replace {
            latest = Some(candidate);
        }
    }
    Ok(latest)
}

fn enrich_workspace_runtime_apps_with_latest_traces(
    workspace: &mut WorkspaceMapResponse,
    store: &EventsStore,
) -> Result<()> {
    let service_mappings = workspace.service_mappings.clone();
    for app in &mut workspace.runtime_apps {
        app.latest_trace_summary =
            workspace_app_latest_trace_summary(store, app, &service_mappings)?;
    }
    Ok(())
}

fn hydrate_workspace_runtime_app_traces(
    paths: &Paths,
    workspace: &mut WorkspaceMapResponse,
) -> Result<()> {
    if workspace.runtime_apps.is_empty() {
        return Ok(());
    }
    if let Some(store) = EventsStore::open(&paths.events_db)? {
        enrich_workspace_runtime_apps_with_latest_traces(workspace, &store)?;
    }
    Ok(())
}

async fn refresh_workspace_scan_cache(
    state: &AppState,
    config: &TomlValue,
) -> Result<WorkspaceMapResponse> {
    let interval_seconds = workspace_scan_interval_seconds(config);
    let paths = state.paths.clone();
    let config_for_scan = config.clone();
    let mut value = tokio::task::spawn_blocking(move || build_workspace_map(&config_for_scan, paths.as_ref()))
        .await
        .context("workspace scan task failed")??;
    hydrate_workspace_runtime_app_traces(state.paths.as_ref(), &mut value)?;
    let cached = CachedWorkspaceMap {
        value: value.clone(),
        scanned_at: OffsetDateTime::now_utc(),
        cached_at: Instant::now(),
        interval_seconds,
    };
    let snapshot_value = serde_json::to_value(&value).unwrap_or_else(|_| json!({}));
    let paths = state.paths.clone();
    tokio::task::spawn_blocking(move || {
        persist_ui_snapshot(
            paths.as_ref(),
            SNAPSHOT_WORKSPACE,
            &snapshot_value,
            UI_SNAPSHOT_SOURCE_CORE,
            Some(interval_seconds),
        )
    })
    .await
    .context("workspace snapshot task failed")??;
    state.scanner_cache.write().await.workspace = Some(cached);
    Ok(value)
}

fn workspace_scan_interval_seconds(config: &TomlValue) -> u64 {
    config
        .get("scanner")
        .and_then(|value| value.get("workspace_interval_seconds"))
        .and_then(TomlValue::as_integer)
        .or_else(|| {
            config
                .get("workspace")
                .and_then(|value| value.get("scan_interval_seconds"))
                .and_then(TomlValue::as_integer)
        })
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(SCANNER_WORKSPACE_DEFAULT_INTERVAL_SECONDS)
        .clamp(
            SCANNER_WORKSPACE_MIN_INTERVAL_SECONDS,
            SCANNER_WORKSPACE_MAX_INTERVAL_SECONDS,
        )
}

async fn api_workspace_map(
    State(state): State<AppState>,
    Query(query): Query<ScanQuery>,
) -> Result<Json<WorkspaceMapResponse>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    cached_workspace_map(&state, &cfg, query.force.unwrap_or(false))
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn api_workspace_projects(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<JsonValue> {
    let max_depth = params
        .get("max_depth")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(3)
        .clamp(1, 10);
    let max_results = params
        .get("max_results")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(50)
        .clamp(1, 500);
    let root = state
        .paths
        .config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    Json(json!({
        "projects": discover_projects_with_limits(root, max_depth, max_results),
    }))
}

async fn api_workspace_services(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let workspace = cached_workspace_map(&state, &cfg, false)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "service_mappings": workspace.service_mappings,
        "unmapped_services": workspace.unmapped_services,
    })))
}

async fn api_workspace_app_resources(
    State(state): State<AppState>,
    AxumPath(app_name): AxumPath<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let workspace = cached_workspace_map(&state, &cfg, false)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let app = workspace
        .runtime_apps
        .iter()
        .find(|item| {
            item.name == app_name || item.display_name.as_deref() == Some(app_name.as_str())
        })
        .ok_or_else(|| (StatusCode::NOT_FOUND, "workspace app not found".to_string()))?;
    let pid = params
        .get("pid")
        .and_then(|value| value.parse::<u32>().ok())
        .or(app.pid);
    let live_resources = workspace_app_live_resources(pid, Some(&app.name));
    let live = live_resources.is_some();
    let resources = live_resources.or_else(|| app.resources.clone());
    Ok(Json(json!({
        "app_name": app.name,
        "pid": pid,
        "sampled_at": now_iso(),
        "live": live,
        "resources": resources,
    })))
}

async fn api_workspace_app_logs(
    State(state): State<AppState>,
    AxumPath(app_name): AxumPath<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(80)
        .clamp(1, 300);
    let workspace = cached_workspace_map(&state, &cfg, false)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let app = workspace
        .runtime_apps
        .iter()
        .find(|item| {
            item.name == app_name || item.display_name.as_deref() == Some(app_name.as_str())
        })
        .ok_or_else(|| (StatusCode::NOT_FOUND, "workspace app not found".to_string()))?;
    let retention = storage_retention_hours(&cfg);
    let events = if let Some(store) = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        workspace_app_events(&store, app, &workspace.service_mappings, retention, limit)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        Vec::new()
    };
    let raw_logs = workspace_app_raw_logs(app, limit);
    Ok(Json(json!({
        "app_name": app.name,
        "events": events,
        "raw_logs": raw_logs,
        "log_sources": app.log_sources,
        "sampled_at": now_iso(),
    })))
}

fn workspace_app_raw_logs(app: &WorkspaceRuntimeApp, limit: usize) -> Vec<JsonValue> {
    let mut logs = Vec::new();
    let mut read_pm2 = false;
    for source in &app.log_sources {
        match source.kind.as_str() {
            "file" => append_file_raw_logs(&mut logs, source, limit),
            "manager" if source.source == "pm2" && !read_pm2 => {
                logs.extend(pm2_app_raw_logs(app, limit.saturating_sub(logs.len())));
                read_pm2 = true;
            }
            _ => {}
        }
        if logs.len() >= limit {
            return logs;
        }
    }
    if matches!(app.manager.as_deref(), Some("pm2")) && !read_pm2 {
        logs.extend(pm2_app_raw_logs(app, limit.saturating_sub(logs.len())));
    }
    logs
}

fn append_file_raw_logs(
    logs: &mut Vec<JsonValue>,
    source: &inferra_contracts::WorkspaceLogSource,
    limit: usize,
) {
    let Some(path) = source.path.as_deref() else {
        return;
    };
    if is_sensitive_workspace_log_path(path) {
        return;
    }
    let path = Path::new(path);
    if !path.is_file() {
        return;
    }
    let Ok(lines) = tail_text_lines(path, limit.min(160), 512 * 1024) else {
        return;
    };
    let rendered_path = clean_display_path(&path.to_string_lossy());
    for (index, line) in lines.into_iter().enumerate() {
        logs.push(json!({
            "source": {
                "kind": source.kind,
                "label": source.label,
                "path": rendered_path,
                "source": source.source,
                "confidence": source.confidence,
            },
            "line": line,
            "line_number_from_tail": index + 1,
            "sampled_at": now_iso(),
        }));
        if logs.len() >= limit {
            return;
        }
    }
}

fn pm2_app_raw_logs(app: &WorkspaceRuntimeApp, limit: usize) -> Vec<JsonValue> {
    if limit == 0 {
        return Vec::new();
    }
    let lines_arg = limit.min(160).to_string();
    let args = [
        "logs",
        app.name.as_str(),
        "--nostream",
        "--lines",
        &lines_arg,
    ];
    let Some(output) = command_output_with_timeout("pm2", &args, 5_000) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (stream, bytes) in [("stdout", output.stdout), ("stderr", output.stderr)] {
        let body = String::from_utf8_lossy(&bytes);
        for (index, line) in body
            .lines()
            .rev()
            .take(limit.saturating_sub(out.len()))
            .enumerate()
        {
            let line = line.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            out.push(json!({
                "source": {
                    "kind": "manager",
                    "label": "PM2 logs",
                    "command": format!("pm2 logs {} --nostream --lines {}", app.name, lines_arg),
                    "stream": stream,
                    "source": "pm2",
                    "confidence": 0.92,
                },
                "line": line,
                "line_number_from_tail": index + 1,
                "sampled_at": now_iso(),
            }));
            if out.len() >= limit {
                break;
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.reverse();
    out
}

fn command_output_with_timeout(
    program: &str,
    args: &[&str],
    timeout_ms: u64,
) -> Option<std::process::Output> {
    let mut child = spawn_command(program, args).ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if started.elapsed() >= std::time::Duration::from_millis(timeout_ms) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(25)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn spawn_command(program: &str, args: &[&str]) -> std::io::Result<std::process::Child> {
    let mut last_error = None;
    for candidate in command_candidates(program) {
        match Command::new(&candidate)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => return Ok(child),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .unwrap_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "command not found")))
}

fn command_candidates(program: &str) -> Vec<String> {
    let mut candidates = vec![program.to_string()];
    if cfg!(windows) && Path::new(program).extension().is_none() {
        candidates.push(format!("{program}.cmd"));
        candidates.push(format!("{program}.exe"));
        candidates.push(format!("{program}.bat"));
    }
    candidates
}

fn tail_text_lines(path: &Path, limit: usize, max_bytes: u64) -> std::io::Result<Vec<String>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(max_bytes)))?;
    let mut body = String::new();
    file.read_to_string(&mut body)?;
    let mut lines: Vec<String> = body
        .lines()
        .rev()
        .take(limit)
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    lines.reverse();
    Ok(lines)
}

fn is_sensitive_workspace_log_path(path: &str) -> bool {
    let Some(name) = Path::new(path).file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower == ".env" || lower == ".env.local" || lower.starts_with(".env.")
}

async fn api_workspace_inspect(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let Some(path) = params.get("path").cloned() else {
        return Err((StatusCode::BAD_REQUEST, "path is required".to_string()));
    };
    if path.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "path is required".to_string()));
    }
    let cfg = state.config.read().await;
    let requested = PathBuf::from(path.trim());
    let canonical = requested
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "workspace path not found".to_string()))?;
    let roots = workspace_inspect_allowed_roots(&cfg, state.paths.as_ref());
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err((
            StatusCode::FORBIDDEN,
            "workspace path is outside configured workspace roots".to_string(),
        ));
    }
    let redact_env_files = cfg
        .get("workspace")
        .and_then(|value| value.get("redact_env_files"))
        .and_then(TomlValue::as_bool)
        .unwrap_or(true);
    Ok(Json(inspect_workspace_project(
        canonical.as_path(),
        redact_env_files,
    )))
}

async fn api_workspace_add_mapping(
    State(state): State<AppState>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let service_id = payload
        .get("service_id")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let project_path = payload
        .get("project_path")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if service_id.is_empty() || project_path.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "service_id and project_path are required".into(),
        ));
    }
    let confidence = payload
        .get("confidence")
        .and_then(|value| value.as_f64())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let notes = payload
        .get("notes")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    let mut config = state.config.read().await.clone();
    let mappings = ensure_toml_array_path(&mut config, &["workspace", "service_mappings"])?;
    mappings.retain(|value| {
        let Some(table) = value.as_table() else {
            return true;
        };
        table
            .get("service_id")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            != service_id
            || table
                .get("project_path")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                != project_path
    });

    let mut row = toml::map::Map::new();
    row.insert("service_id".into(), TomlValue::String(service_id.clone()));
    row.insert(
        "project_path".into(),
        TomlValue::String(project_path.clone()),
    );
    row.insert("confidence".into(), TomlValue::Float(confidence));
    row.insert("source".into(), TomlValue::String("user".into()));
    if !notes.is_empty() {
        row.insert("notes".into(), TomlValue::String(notes.clone()));
    }
    mappings.push(TomlValue::Table(row));
    inferra_config::write_config(&state.paths.config_path, &config)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    *state.config.write().await = config;
    Ok(Json(json!({
        "stored": true,
        "service_id": service_id,
        "project_path": project_path,
        "confidence": confidence,
        "persisted": true,
    })))
}

async fn api_topology(State(state): State<AppState>) -> Json<JsonValue> {
    let cfg = state.config.read().await.clone();
    let payload = json!({ "edges": topology_edges(&cfg) });
    let _ = persist_ui_snapshot(
        state.paths.as_ref(),
        SNAPSHOT_GRAPH,
        &payload,
        UI_SNAPSHOT_SOURCE_CORE,
        None,
    );
    Json(payload)
}

async fn api_topology_add_edge(
    State(state): State<AppState>,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let source = payload
        .get("source")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let target = payload
        .get("target")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if source.is_empty() || target.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'source' and 'target' are required".into(),
        ));
    }
    let relation_type = payload
        .get("relation_type")
        .or_else(|| payload.get("type"))
        .and_then(|value| value.as_str())
        .unwrap_or("depends_on")
        .trim()
        .to_string();

    let mut config = state.config.read().await.clone();
    let edges = ensure_toml_array_path(&mut config, &["topology", "edges"])?;
    let exists = edges.iter().any(|value| {
        let Some(table) = value.as_table() else {
            return false;
        };
        table
            .get("source")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            == source
            && table
                .get("target")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                == target
            && table
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("depends_on")
                == relation_type
    });
    if !exists {
        let mut row = toml::map::Map::new();
        row.insert("source".into(), TomlValue::String(source.clone()));
        row.insert("target".into(), TomlValue::String(target.clone()));
        row.insert("type".into(), TomlValue::String(relation_type.clone()));
        edges.push(TomlValue::Table(row));
        inferra_config::write_config(&state.paths.config_path, &config)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        *state.config.write().await = config;
    }
    Ok(Json(json!({
        "added": true,
        "edge": {
            "source": source,
            "target": target,
            "relation_type": relation_type,
        }
    })))
}

async fn run_investigation_response(
    _paths: &Paths,
    config: &TomlValue,
    bundle: &JsonValue,
) -> Result<JsonValue> {
    let redacted_bundle = redact_bundle_for_ai(bundle, config);
    let provider = probe_ai_provider(config, AiProbePurpose::Investigate).await;
    if !provider.enabled {
        return Ok(deterministic_investigation_response(
            &redacted_bundle,
            provider.provider_payload(),
            "AI is disabled in config.",
            Vec::new(),
            1,
            Some(fallback_trace(
                "provider_disabled",
                "AI is disabled in config.",
            )),
        ));
    }
    if !provider.available {
        let reason = provider
            .reason
            .clone()
            .unwrap_or_else(|| "AI provider unavailable.".to_string());
        return Ok(deterministic_investigation_response(
            &redacted_bundle,
            provider.provider_payload(),
            &reason,
            Vec::new(),
            1,
            Some(fallback_trace("provider_unavailable", &reason)),
        ));
    }

    let mode = bundle
        .get("mode")
        .and_then(JsonValue::as_str)
        .unwrap_or("operator");
    let bundle_json = serde_json::to_string(&redacted_bundle)?;
    let user_prompt = INVESTIGATION_USER_TEMPLATE
        .replace("{mode}", mode)
        .replace("{bundle_json}", &bundle_json);
    let messages = json!([
        {"role": "system", "content": INVESTIGATION_SYSTEM_PROMPT},
        {"role": "user", "content": user_prompt},
    ]);
    let trace = json!({
        "trace_kind": "investigate",
        "sanitized_system_prompt": INVESTIGATION_SYSTEM_PROMPT,
        "sanitized_user_prompt": user_prompt,
        "allowed_fields": ["mode", "incident", "hypotheses", "events", "services", "runtime", "workspace", "user_question", "constraints", "runtime_monitor", "host_resources", "evidence_digest", "similar_incidents", "operator_memory"],
        "blocked_fields": ["raw_event_messages", "env_values", "ip_addresses", "secrets"],
        "raw_logs_sent": raw_workspace_logs_sent_to_ai(bundle, config),
        "schema_version": 1,
    });
    let mut warnings = Vec::new();

    for attempt in 1..=INVESTIGATION_MAX_AI_ATTEMPTS {
        match ollama_chat(config, &provider, &messages).await {
            Ok(raw) => {
                let parsed = extract_json_object(&raw);
                if let Some(mut output) = parsed.and_then(normalize_investigation_output) {
                    if output_has_signal(&output) {
                        let grounding = apply_output_grounding(&mut output, &redacted_bundle);
                        if let Some(w) =
                            hypothesis_rank_alignment_warning(&output, &redacted_bundle)
                        {
                            warnings.push(w);
                        }
                        return Ok(json!({
                            "schema_version": 1,
                            "output": output,
                            "used_ai": true,
                            "fallback_reason": "",
                            "warnings": warnings,
                            "attempts": attempt,
                            "provider": provider.provider_payload(),
                            "trace": trace,
                            "bundle": redacted_bundle,
                            "grounding": grounding,
                        }));
                    }
                }
                warnings.push(format!(
                    "Attempt {attempt}: AI returned an investigation payload without meaningful content"
                ));
            }
            Err(error) => warnings.push(format!("Attempt {attempt}: {error}")),
        }
    }

    let fallback_reason = warnings
        .last()
        .cloned()
        .unwrap_or_else(|| "AI output was unusable after repeated attempts.".to_string());
    Ok(deterministic_investigation_response(
        &redacted_bundle,
        provider.provider_payload(),
        &fallback_reason,
        warnings,
        INVESTIGATION_MAX_AI_ATTEMPTS,
        Some(trace),
    ))
}

fn persist_investigation_artifacts(
    paths: &Paths,
    bundle: &JsonValue,
    response: &JsonValue,
) -> Result<Option<JsonValue>> {
    initialize_databases(&paths.events_db, &paths.incidents_db)?;
    let Some(incident_id) = investigation_incident_id(bundle) else {
        return Ok(None);
    };
    let Some(incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(None);
    };
    let hypotheses_hash = stable_hash_json(bundle.get("hypotheses").unwrap_or(&JsonValue::Null));
    let events_hash_head = stable_hash_json(bundle.get("events").unwrap_or(&JsonValue::Null));
    let explanation = build_stored_explanation(
        &incident_id,
        bundle,
        response,
        &hypotheses_hash,
        &events_hash_head,
    )?;
    incidents.add_explanation(&explanation)?;
    if let Some(trace_value) = response.get("trace").filter(|value| !value.is_null()) {
        incidents.add_ai_trace(&build_stored_trace(&incident_id, trace_value)?)?;
    }
    Ok(Some(json!({
        "incident_id": incident_id,
        "explanation": incidents.latest_explanation(&incident_id)?,
        "latest_trace": incidents.latest_ai_trace(&incident_id)?,
        "feedback": incidents.list_feedback(&incident_id)?,
        "state_log": incidents.list_state_log(&incident_id)?,
    })))
}

fn investigation_incident_id(bundle: &JsonValue) -> Option<String> {
    bundle
        .get("incident")
        .and_then(JsonValue::as_object)
        .and_then(|incident| incident.get("incident_id"))
        .and_then(JsonValue::as_str)
        .map(str::to_string)
}

fn build_stored_explanation(
    incident_id: &str,
    bundle: &JsonValue,
    response: &JsonValue,
    hypotheses_hash: &str,
    events_hash_head: &str,
) -> Result<StoredExplanation> {
    let output = response.get("output").cloned().unwrap_or_else(|| json!({}));
    let headline = output
        .get("headline")
        .and_then(JsonValue::as_str)
        .unwrap_or("Investigation summary")
        .to_string();
    let what_happened = string_array(output.get("what_happened"));
    let likely_causes = string_array(output.get("likely_causes"));
    let evidence_lines = output
        .get("evidence")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .take(8)
                .map(|item| {
                    format!(
                        "{}:{} {}",
                        item.get("type")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("evidence"),
                        item.get("id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("unknown"),
                        item.get("summary")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|value| !value.is_empty());
    let timeline_text = bundle
        .get("events")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .take(10)
                .map(|item| {
                    format!(
                        "{} {}",
                        item.get("timestamp")
                            .and_then(JsonValue::as_str)
                            .unwrap_or(""),
                        item.get("summary")
                            .or_else(|| item.get("message"))
                            .and_then(JsonValue::as_str)
                            .unwrap_or("")
                    )
                    .trim()
                    .to_string()
                })
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|value| !value.is_empty());
    let uncertainty = string_array(output.get("uncertainty"));
    let created_at = now_iso();
    let primary_text = std::iter::once(headline.clone())
        .chain(what_happened.iter().cloned())
        .chain(
            likely_causes
                .iter()
                .map(|cause| format!("Likely cause: {cause}")),
        )
        .collect::<Vec<_>>()
        .join("\n");
    Ok(StoredExplanation {
        explanation_id: artifact_id("exp", incident_id, &headline),
        incident_id: incident_id.to_string(),
        summary: headline,
        primary_text,
        evidence_text: evidence_lines,
        timeline_text,
        alternatives: string_array(output.get("missing_evidence")),
        actions: output
            .get("next_steps")
            .and_then(JsonValue::as_array)
            .map(|steps| {
                steps
                    .iter()
                    .map(|step| {
                        let title = step
                            .get("title")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("Next step");
                        let reason = step
                            .get("reason")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default();
                        if reason.is_empty() {
                            title.to_string()
                        } else {
                            format!("{title}: {reason}")
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        uncertainty,
        model_used: response
            .get("provider")
            .and_then(|provider| provider.get("model"))
            .and_then(JsonValue::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                if response.get("used_ai") == Some(&JsonValue::Bool(true)) {
                    "native_ai"
                } else {
                    "deterministic_fallback"
                }
            })
            .to_string(),
        guardrail_flags: string_array(response.get("warnings")),
        created_at,
        explanation_schema_version: 1,
        hypotheses_hash: hypotheses_hash.to_string(),
        events_hash_head: events_hash_head.to_string(),
        quality: if response.get("used_ai") == Some(&JsonValue::Bool(true)) {
            "ai".into()
        } else {
            "fallback".into()
        },
    })
}

fn build_stored_trace(incident_id: &str, trace: &JsonValue) -> Result<StoredAiTrace> {
    let trace_kind = trace
        .get("trace_kind")
        .and_then(JsonValue::as_str)
        .unwrap_or("investigate")
        .to_string();
    Ok(StoredAiTrace {
        trace_id: artifact_id("trace", incident_id, &trace_kind),
        incident_id: incident_id.to_string(),
        trace_kind,
        sanitized_system_prompt: trace
            .get("sanitized_system_prompt")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string(),
        sanitized_user_prompt: trace
            .get("sanitized_user_prompt")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string(),
        allowed_fields: string_array(trace.get("allowed_fields")),
        blocked_fields: string_array(trace.get("blocked_fields")),
        raw_logs_sent: trace
            .get("raw_logs_sent")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        trace_schema_version: trace
            .get("schema_version")
            .and_then(JsonValue::as_i64)
            .unwrap_or(1),
        created_at: now_iso(),
    })
}

fn string_array(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn stable_hash_json(value: &JsonValue) -> String {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    stable_hash_text(&serialized)
}

fn ai_generation_scope_key(focus: &str, mode: &str, question: &str, report: bool) -> String {
    let normalized_question = question.trim();
    format!(
        "{}|mode={}|report={}|q={:016x}",
        focus,
        mode,
        report,
        stable_hash_u64(normalized_question)
    )
}

fn artifact_id(prefix: &str, incident_id: &str, seed: &str) -> String {
    format!(
        "{prefix}-{incident_id}-{}-{:016x}",
        unix_seconds(),
        stable_hash_u64(seed)
    )
}

fn stable_hash_text(text: &str) -> String {
    format!("{:016x}", stable_hash_u64(text))
}

fn stable_hash_u64(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn fallback_trace(kind: &str, reason: &str) -> JsonValue {
    json!({
        "trace_kind": kind,
        "sanitized_system_prompt": INVESTIGATION_SYSTEM_PROMPT,
        "sanitized_user_prompt": reason,
        "allowed_fields": ["incident", "hypotheses", "events", "services", "runtime", "workspace", "runtime_monitor", "host_resources", "evidence_digest", "similar_incidents", "operator_memory"],
        "blocked_fields": ["raw_event_messages", "env_values", "ip_addresses", "secrets"],
        "raw_logs_sent": false,
        "schema_version": 1,
    })
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

fn format_offset_datetime(value: OffsetDateTime) -> String {
    value
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

fn parse_offset_datetime(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

fn query_bool(params: &HashMap<String, String>, key: &str) -> Option<bool> {
    params.get(key).map(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn persist_ui_snapshot(
    paths: &Paths,
    data_type: &str,
    payload: &JsonValue,
    source: &str,
    interval_seconds: Option<u64>,
) -> Result<()> {
    initialize_databases(&paths.events_db, &paths.incidents_db)?;
    if let Some(store) = IncidentsStore::open(&paths.incidents_db)? {
        store.upsert_ui_snapshot(
            data_type,
            payload,
            source,
            interval_seconds.and_then(|value| i64::try_from(value).ok()),
        )?;
    }
    Ok(())
}

fn persist_ai_generation(
    paths: &Paths,
    scope_key: &str,
    focus: &str,
    mode: &str,
    question: &str,
    bundle: &JsonValue,
    response: &JsonValue,
) -> Result<StoredAiGeneration> {
    initialize_databases(&paths.events_db, &paths.incidents_db)?;
    let Some(store) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(StoredAiGeneration {
            generation_id: artifact_id(
                "ai-gen",
                focus,
                &format!("{scope_key}:{}", stable_hash_json(response)),
            ),
            scope_key: scope_key.to_string(),
            focus: focus.to_string(),
            mode: mode.to_string(),
            question: question.to_string(),
            response: response.clone(),
            bundle_hash: stable_hash_json(bundle),
            used_ai: response
                .get("used_ai")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            provider: response.get("provider").cloned().unwrap_or(JsonValue::Null),
            created_at: now_iso(),
        });
    };
    let provider = response.get("provider").cloned().unwrap_or(JsonValue::Null);
    let generation = StoredAiGeneration {
        generation_id: artifact_id(
            "ai-gen",
            focus,
            &format!("{scope_key}:{}", stable_hash_json(response)),
        ),
        scope_key: scope_key.to_string(),
        focus: focus.to_string(),
        mode: mode.to_string(),
        question: question.to_string(),
        response: response.clone(),
        bundle_hash: stable_hash_json(bundle),
        used_ai: response
            .get("used_ai")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        provider,
        created_at: now_iso(),
    };
    store.add_ai_generation(&generation)?;
    Ok(generation)
}

fn finalize_ai_generation_response_lossy(
    paths: &Paths,
    bundle: &JsonValue,
    mut response: JsonValue,
    focus: &str,
    mode: &str,
    question: &str,
    report: bool,
) -> JsonValue {
    if let Ok(Some(audit)) = persist_investigation_artifacts(paths, bundle, &response) {
        response["audit"] = audit;
    }
    response["focus"] = JsonValue::String(focus.to_string());
    response["mode"] = JsonValue::String(mode.to_string());
    if !question.trim().is_empty() {
        response["question"] = JsonValue::String(question.to_string());
    }
    if report {
        response["report"] = JsonValue::Bool(true);
    }
    let scope_key = ai_generation_scope_key(focus, mode, question, report);
    if let Ok(generation) =
        persist_ai_generation(paths, &scope_key, focus, mode, question, bundle, &response)
    {
        response["ai_generation"] = ai_generation_metadata_json(&generation);
    }
    let _ = persist_ui_snapshot(
        paths,
        SNAPSHOT_AI_INVESTIGATION,
        &json!({
            "focus": focus,
            "mode": mode,
            "response": response.clone(),
        }),
        UI_SNAPSHOT_SOURCE_CORE,
        None,
    );
    response
}

fn ai_generation_metadata_json(generation: &StoredAiGeneration) -> JsonValue {
    json!({
        "generation_id": generation.generation_id,
        "scope_key": generation.scope_key,
        "focus": generation.focus,
        "mode": generation.mode,
        "question": generation.question,
        "bundle_hash": generation.bundle_hash,
        "used_ai": generation.used_ai,
        "provider": generation.provider,
        "created_at": generation.created_at,
    })
}

fn load_saved_ai_generation(paths: &Paths, scope_key: &str) -> Result<Option<JsonValue>> {
    initialize_databases(&paths.events_db, &paths.incidents_db)?;
    let Some(store) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(None);
    };
    store.latest_ai_generation(scope_key)
}

fn read_ui_snapshot(paths: &Paths, data_type: &str) -> Result<Option<StoredUiSnapshot>> {
    initialize_databases(&paths.events_db, &paths.incidents_db)?;
    let Some(store) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(None);
    };
    store.ui_snapshot(data_type)
}

fn read_ui_snapshots(paths: &Paths) -> Result<Vec<JsonValue>> {
    initialize_databases(&paths.events_db, &paths.incidents_db)?;
    let Some(store) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(Vec::new());
    };
    Ok(store
        .ui_snapshots()?
        .into_iter()
        .map(|snapshot| {
            json!({
                "data_type": snapshot.data_type,
                "source": snapshot.source,
                "updated_at": snapshot.updated_at,
                "schema_version": snapshot.schema_version,
                "interval_seconds": snapshot.interval_seconds,
                "age_seconds": ui_snapshot_age_seconds(&snapshot.updated_at),
            })
        })
        .collect())
}

fn ui_snapshot_age_seconds(updated_at: &str) -> Option<u64> {
    let timestamp = parse_offset_datetime(updated_at)?;
    let delta = OffsetDateTime::now_utc() - timestamp;
    Some(delta.whole_seconds().max(0) as u64)
}

fn severity_option_is_error(value: Option<&SeverityValue>) -> bool {
    match value {
        Some(SeverityValue::Level(level)) => *level >= 3,
        Some(SeverityValue::Label(label)) => matches!(
            label.to_ascii_lowercase().as_str(),
            "error" | "critical" | "fatal" | "panic"
        ),
        None => false,
    }
}

fn current_mode(config: &TomlValue, override_mode: Option<&str>) -> String {
    if let Some(mode) = override_mode {
        if matches!(mode, "operator" | "expert" | "developer") {
            return mode.to_string();
        }
    }
    experience_from_config(config).mode
}

fn build_investigation_bundle(
    paths: &Paths,
    config: &TomlValue,
    focus: &str,
    question: &str,
    mode: &str,
) -> Result<JsonValue> {
    let overview = build_overview(config, paths)?;
    let workspace = build_workspace_map(config, paths)?;
    match focus_to_scope(focus) {
        InvestigationScope::Overview => {
            let mut hypotheses_json = json!([]);
            let mut events_preview = json!([]);
            if let Ok(Some(incidents)) = IncidentsStore::open(&paths.incidents_db) {
                if let Ok(Some(latest_id)) = incidents.latest_active_incident_id() {
                    if let Ok(hrows) = incidents.hypotheses(&latest_id) {
                        hypotheses_json =
                            serde_json::to_value(&hrows).unwrap_or_else(|_| json!([]));
                    }
                }
            }
            if let Ok(Some(ev)) = EventsStore::open(&paths.events_db) {
                if let Ok(rows) = ev.latest_events(40) {
                    let preview: Vec<JsonValue> = rows
                        .into_iter()
                        .map(|e| {
                            let msg = e.message.clone().unwrap_or_default();
                            let summary = if msg.len() > 200 {
                                format!("{}...", &msg[..197])
                            } else {
                                msg
                            };
                            json!({
                                "event_id": e.event_id,
                                "timestamp": e.timestamp,
                                "service_id": e.service_id,
                                "severity": e.severity,
                                "summary": summary,
                                "tags": e.tags.unwrap_or_default(),
                            })
                        })
                        .collect();
                    events_preview = json!(preview);
                }
            }
            Ok(json!({
                "mode": mode,
                "focus": "overview",
                "context_summary": {
                    "scope_kind": "overview",
                    "active_incident_count": overview.dashboard.incidents.as_ref().map(Vec::len).unwrap_or_default(),
                    "service_count": overview.dashboard.services.as_ref().map(Vec::len).unwrap_or_default(),
                    "event_preview_count": events_preview.as_array().map(Vec::len).unwrap_or_default(),
                    "workspace_project_count": workspace.projects.len(),
                    "workspace_runtime_app_count": workspace.runtime_apps.len(),
                    "operator_question": question,
                },
                "incident": overview.dashboard.incidents.as_ref().and_then(|items| items.first().cloned()),
                "hypotheses": hypotheses_json,
                "events": events_preview,
                "services": overview.dashboard.services.unwrap_or_default(),
                "runtime": overview.runtime,
                "workspace": workspace,
                "user_question": question,
                "constraints": {},
            }))
        }
        InvestigationScope::Incident(incident_id) => {
            let incidents =
                IncidentsStore::open(&paths.incidents_db)?.context("incident store not found")?;
            let events = EventsStore::open(&paths.events_db)?.context("event store not found")?;
            let incident = incidents
                .get_incident(&incident_id)?
                .context("incident not found")?;
            let event_ids = incidents.incident_event_ids(&incident_id)?;
            let event_rows = events.get_events(&event_ids)?;
            let hypotheses = incidents.hypotheses(&incident_id)?;
            Ok(json!({
                "mode": mode,
                "focus": format!("incident:{incident_id}"),
                "context_summary": {
                    "scope_kind": "incident",
                    "incident_id": incident_id,
                    "primary_service": incident.primary_service.clone(),
                    "affected_services": incident.affected_services.clone(),
                    "severity": incident.severity,
                    "state": incident.state.clone(),
                    "event_count": event_rows.len(),
                    "hypothesis_count": hypotheses.len(),
                    "operator_question": question,
                },
                "incident": incident,
                "hypotheses": hypotheses,
                "events": event_rows,
                "services": overview.dashboard.services.unwrap_or_default(),
                "runtime": overview.runtime,
                "workspace": workspace,
                "user_question": question,
                "constraints": {},
            }))
        }
        InvestigationScope::Service(service_id) => {
            let events = if let Some(store) = EventsStore::open(&paths.events_db)? {
                store.events_for_service(&service_id, 50)?
            } else {
                vec![]
            };
            let services = overview.dashboard.services.unwrap_or_default();
            let service = services
                .iter()
                .find(|item| item.service_id == service_id)
                .cloned()
                .context("service not found")?;
            Ok(json!({
                "mode": mode,
                "focus": format!("service:{service_id}"),
                "context_summary": {
                    "scope_kind": "service",
                    "service_id": service_id,
                    "status": service.status.clone(),
                    "event_count": events.len(),
                    "recent_error_count": events.iter().filter(|event| severity_option_is_error(event.severity.as_ref())).count(),
                    "operator_question": question,
                },
                "incident": JsonValue::Null,
                "hypotheses": [],
                "events": events,
                "services": [service],
                "runtime": overview.runtime,
                "workspace": workspace,
                "user_question": question,
                "constraints": {},
            }))
        }
        InvestigationScope::WorkspaceApp(app_name) => {
            let app = workspace
                .runtime_apps
                .iter()
                .find(|item| item.name == app_name)
                .cloned()
                .with_context(|| format!("workspace app not found: {app_name}"))?;
            let retention = storage_retention_hours(config);
            let events = if let Some(store) = EventsStore::open(&paths.events_db)? {
                let by_service = store.query_logs(&LogsQuery {
                    limit: 50,
                    retention_hours: retention,
                    service_id: Some(app.name.clone()),
                    ..Default::default()
                })?;
                if by_service.is_empty() {
                    store.query_logs(&LogsQuery {
                        limit: 50,
                        retention_hours: retention,
                        source_type: Some(app.name.clone()),
                        ..Default::default()
                    })?
                } else {
                    by_service
                }
            } else {
                vec![]
            };
            let raw_logs = workspace_app_raw_logs(&app, 80);
            let services = overview.dashboard.services.unwrap_or_default();
            Ok(json!({
                "mode": mode,
                "focus": format!("workspace_app:{app_name}"),
                "context_summary": {
                    "scope_kind": "workspace_app",
                    "app_name": app.name.clone(),
                    "display_name": app.name.clone(),
                    "runtime": app.runtime.clone(),
                    "framework": app.framework.clone(),
                    "manager": app.manager.clone(),
                    "pid": app.pid,
                    "project_path": app.project_path.clone(),
                    "app_url": app.app_url.clone(),
                    "health_endpoint": app.health_endpoint.clone(),
                    "endpoint_count": app.endpoints.len(),
                    "log_source_count": app.log_sources.len(),
                    "app_structure_count": app.app_structure.len(),
                    "log_hint_count": app.log_hints.len(),
                    "detected_signal_count": app.signals.len(),
                    "app_state": app.app_state.clone(),
                    "resources": app.resources.clone(),
                    "resource_semantics": workspace_resource_semantics(),
                    "location": app.app_location.clone(),
                    "context_capabilities": app.context_capabilities.clone(),
                    "matching_event_count": events.len(),
                    "raw_log_sample_count": raw_logs.len(),
                    "operator_question": question,
                },
                "incident": JsonValue::Null,
                "hypotheses": [],
                "events": events,
                "services": services
                    .into_iter()
                    .filter(|service| service.service_id == app.name)
                    .collect::<Vec<_>>(),
                "runtime": overview.runtime,
                "workspace": {
                    "projects": workspace.projects,
                    "runtime_apps": workspace.runtime_apps,
                    "service_mappings": workspace.service_mappings,
                    "selected_app": app.clone(),
                    "selected_app_context": {
                        "logs": app.log_sources.clone(),
                        "endpoints": app.endpoints.clone(),
                        "health_endpoint": app.health_endpoint.clone(),
                        "state": app.app_state.clone(),
                        "resources": app.resources.clone(),
                        "resource_semantics": workspace_resource_semantics(),
                        "location": app.app_location.clone(),
                        "capabilities": app.context_capabilities.clone(),
                        "app_structure": app.app_structure.clone(),
                        "raw_logs": raw_logs,
                    },
                },
                "user_question": question,
                "constraints": {
                    "scope_kind": "workspace_app",
                    "note": "This is a workspace runtime app. It may not exist as a service in the event database yet."
                },
            }))
        }
    }
}

fn resolve_monitor_seconds(
    config: &TomlValue,
    query_param: Option<&String>,
    payload: Option<&JsonValue>,
) -> u64 {
    if let Some(p) = payload {
        if let Some(v) = p.get("monitor_seconds").and_then(|x| x.as_u64()) {
            return v.min(180);
        }
    }
    if let Some(s) = query_param {
        if let Ok(v) = s.parse::<u64>() {
            return v.min(180);
        }
    }
    ai_table_u64(
        config.get("ai").and_then(|a| a.as_table()),
        "investigation_monitor_seconds",
        5,
    )
}

fn workspace_resource_semantics() -> JsonValue {
    json!({
        "resources.cpu_percent": "estimated percentage of total host CPU used by this app/process",
        "resources.cpu_raw_percent": "raw process CPU reading from the runtime, usually single-core-equivalent percent",
        "resources.cpu_percent_scope": "host_total means the displayed cpu_percent is normalized against logical processors",
        "host_resources.global_cpu_percent": "authoritative total-machine CPU usage for the investigation window",
    })
}

fn operator_memory_scope_keys(focus: &str) -> Vec<String> {
    let mut keys = vec!["global".to_string()];
    if let Some(id) = focus.strip_prefix("incident:") {
        if !id.is_empty() {
            keys.push(format!("incident:{id}"));
        }
    }
    if let Some(id) = focus.strip_prefix("service:") {
        if !id.is_empty() {
            keys.push(format!("service:{id}"));
        }
    }
    keys
}

fn load_operator_memory(paths: &Paths, focus: &str) -> JsonValue {
    let Ok(Some(store)) = IncidentsStore::open(&paths.incidents_db) else {
        return json!({});
    };
    let mut out = serde_json::Map::new();
    for key in operator_memory_scope_keys(focus) {
        if let Ok(Some(body)) = store.get_operator_context(&key) {
            out.insert(key, JsonValue::String(body));
        }
    }
    JsonValue::Object(out)
}

fn build_fleet_evidence_digest(events: &EventsStore, limit: usize) -> Result<JsonValue> {
    let rows = events.latest_events(limit)?;
    let mut by_service: HashMap<String, u64> = HashMap::new();
    let mut sev_counts: HashMap<String, u64> = HashMap::new();
    for e in &rows {
        if let Some(ref sid) = e.service_id {
            *by_service.entry(sid.clone()).or_insert(0) += 1;
        }
        let sev_label = e
            .severity
            .as_ref()
            .map(|value| match value {
                inferra_contracts::SeverityValue::Level(level) => level.to_string(),
                inferra_contracts::SeverityValue::Label(label) => label.clone(),
            })
            .unwrap_or_else(|| "unknown".into());
        *sev_counts.entry(sev_label).or_insert(0) += 1;
    }
    let mut top: Vec<(String, u64)> = by_service.into_iter().collect();
    top.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    top.truncate(12);
    Ok(json!({
        "window_events_sampled": rows.len(),
        "top_services_by_recent_event_volume": top.into_iter().map(|(k, v)| json!({"service_id": k, "count": v})).collect::<Vec<_>>(),
        "severity_counts_recent": sev_counts,
    }))
}

fn attach_similar_incidents(paths: &Paths, bundle: &mut JsonValue) -> Result<()> {
    let Some(inc_id) = investigation_incident_id(bundle) else {
        return Ok(());
    };
    let Some(store) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(());
    };
    let Some(current) = store.get_incident(&inc_id)? else {
        return Ok(());
    };
    let candidates = store.recent_incidents_excluding(&inc_id, 80)?;
    let scored = score_similar_incidents(&current, candidates);
    if let Some(obj) = bundle.as_object_mut() {
        obj.insert("similar_incidents".into(), json!(scored));
    }
    Ok(())
}

fn score_similar_incidents(current: &IncidentRow, candidates: Vec<IncidentRow>) -> Vec<JsonValue> {
    let mut rows: Vec<(i32, IncidentRow)> = candidates
        .into_iter()
        .map(|c| {
            let mut s = 0_i32;
            if !c.primary_service.is_empty() && c.primary_service == current.primary_service {
                s += 4;
            }
            if c.severity == current.severity {
                s += 1;
            }
            if c.state == current.state {
                s += 1;
            }
            (s, c)
        })
        .collect();
    rows.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    rows.into_iter()
        .take(6)
        .filter(|(s, _)| *s > 0)
        .map(|(score, c)| {
            json!({
                "incident_id": c.incident_id,
                "similarity_score": score,
                "primary_service": c.primary_service,
                "severity": c.severity,
                "state": c.state,
                "updated_at": c.updated_at,
            })
        })
        .collect()
}

async fn investigation_bundle_enriched(
    paths: &Paths,
    config: &TomlValue,
    focus: &str,
    question: &str,
    mode: &str,
    monitor_seconds: u64,
) -> Result<JsonValue> {
    initialize_databases(&paths.events_db, &paths.incidents_db)?;
    let mut bundle = build_investigation_bundle(paths, config, focus, question, mode)?;
    let interval_ms = ai_table_u64(
        config.get("ai").and_then(|a| a.as_table()),
        "investigation_monitor_interval_ms",
        500,
    );
    let monitor_json = if monitor_seconds > 0 {
        collect_runtime_monitor_window(monitor_seconds, interval_ms).await
    } else {
        json!({
            "skipped": true,
            "reason": "monitor_seconds is 0 (instant snapshot only; host_resources still captured once).",
        })
    };
    let mut host = collect_host_resources_snapshot();
    let gpu = try_collect_gpu_summary().await;
    if let Some(obj) = host.as_object_mut() {
        obj.insert("gpu".into(), gpu);
    }
    if let Some(obj) = bundle.as_object_mut() {
        obj.insert("runtime_monitor".into(), monitor_json);
        obj.insert("host_resources".into(), host);
        if let Ok(Some(ev)) = EventsStore::open(&paths.events_db) {
            if let Ok(digest) = build_fleet_evidence_digest(&ev, 160) {
                obj.insert("evidence_digest".into(), digest);
            }
        }
        let mem = load_operator_memory(paths, focus);
        if !mem.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            obj.insert("operator_memory".into(), mem);
        }
    }
    let _ = attach_similar_incidents(paths, &mut bundle);
    Ok(bundle)
}

fn collect_allowed_ids_from_bundle(
    bundle: &JsonValue,
) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
    let mut events = HashSet::new();
    let mut services = HashSet::new();
    let mut incidents = HashSet::new();
    if let Some(inc) = bundle.get("incident").and_then(JsonValue::as_object) {
        if let Some(id) = inc.get("incident_id").and_then(JsonValue::as_str) {
            incidents.insert(id.to_string());
        }
    }
    if let Some(arr) = bundle.get("events").and_then(JsonValue::as_array) {
        for e in arr {
            if let Some(id) = e.get("event_id").and_then(JsonValue::as_str) {
                events.insert(id.to_string());
            }
        }
    }
    if let Some(arr) = bundle.get("services").and_then(JsonValue::as_array) {
        for s in arr {
            if let Some(id) = s.get("service_id").and_then(JsonValue::as_str) {
                services.insert(id.to_string());
            }
        }
    }
    if let Some(arr) = bundle
        .get("similar_incidents")
        .and_then(JsonValue::as_array)
    {
        for s in arr {
            if let Some(id) = s.get("incident_id").and_then(JsonValue::as_str) {
                incidents.insert(id.to_string());
            }
        }
    }
    (events, services, incidents)
}

fn apply_output_grounding(output: &mut JsonValue, bundle: &JsonValue) -> JsonValue {
    let (allowed_ev, allowed_svc, allowed_inc) = collect_allowed_ids_from_bundle(bundle);
    let mut removed_evidence = Vec::new();
    let mut removed_citations = Vec::new();
    if let Some(arr) = output.get_mut("evidence").and_then(JsonValue::as_array_mut) {
        arr.retain(|item| {
            let t = item.get("type").and_then(JsonValue::as_str).unwrap_or("");
            let id = item.get("id").and_then(JsonValue::as_str).unwrap_or("");
            let ok = match t {
                "event" => allowed_ev.contains(id),
                "service" => allowed_svc.contains(id),
                "incident" => allowed_inc.contains(id),
                "workspace" => true,
                _ => false,
            };
            if !ok && !id.is_empty() {
                removed_evidence.push(format!("{t}:{id}"));
            }
            ok || id.is_empty()
        });
    }
    if let Some(arr) = output
        .get_mut("citations")
        .and_then(JsonValue::as_array_mut)
    {
        arr.retain(|c| {
            let id = c.as_str().unwrap_or("");
            let ok =
                allowed_ev.contains(id) || allowed_svc.contains(id) || allowed_inc.contains(id);
            if !ok && !id.is_empty() {
                removed_citations.push(id.to_string());
            }
            ok || id.is_empty()
        });
    }
    json!({
        "removed_evidence_ids": removed_evidence,
        "removed_citation_ids": removed_citations,
    })
}

fn hypothesis_rank_alignment_warning(output: &JsonValue, bundle: &JsonValue) -> Option<String> {
    let hypo = bundle.get("hypotheses").and_then(JsonValue::as_array)?;
    if hypo.is_empty() {
        return None;
    }
    let top_desc = hypo
        .first()?
        .get("description")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let causes = output.get("likely_causes").and_then(JsonValue::as_array)?;
    let first_cause = causes.first()?.as_str()?.trim();
    let needle = top_desc.to_ascii_lowercase();
    let hay = first_cause.to_ascii_lowercase();
    if needle.len() > 8 && hay.contains(needle.as_str()) {
        return None;
    }
    Some(
        "Top likely_cause text does not resemble the highest-ranked hypothesis description; verify alignment with hypotheses[]."
            .into(),
    )
}

enum InvestigationScope {
    Overview,
    Incident(String),
    Service(String),
    WorkspaceApp(String),
}

fn resolve_focus_from_scope(paths: &Paths, scope: &str) -> Result<String> {
    let trimmed = scope.trim();
    if trimmed.eq_ignore_ascii_case("latest") {
        return Ok(latest_incident_focus(paths)?.unwrap_or_else(|| "latest:none".to_string()));
    }
    if trimmed.starts_with("incident:")
        || trimmed.starts_with("service:")
        || trimmed.starts_with("workspace_app:")
    {
        return Ok(trimmed.to_string());
    }
    Ok("overview".to_string())
}

fn latest_incident_focus(paths: &Paths) -> Result<Option<String>> {
    let Some(incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(None);
    };
    Ok(incidents
        .latest_active_incident_id()?
        .or(incidents.latest_incident_id()?)
        .map(|incident_id| format!("incident:{incident_id}")))
}

fn focus_to_scope(focus: &str) -> InvestigationScope {
    if focus == "latest" || focus == "latest:none" {
        return InvestigationScope::Overview;
    }
    if let Some(id) = focus.strip_prefix("incident:") {
        return InvestigationScope::Incident(id.to_string());
    }
    if let Some(id) = focus.strip_prefix("service:") {
        return InvestigationScope::Service(id.to_string());
    }
    if let Some(id) = focus.strip_prefix("workspace_app:") {
        return InvestigationScope::WorkspaceApp(id.to_string());
    }
    InvestigationScope::Overview
}

fn ai_enabled(config: &TomlValue) -> bool {
    config
        .get("ai")
        .and_then(|value| value.get("enabled"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

async fn probe_ai_provider(config: &TomlValue, purpose: AiProbePurpose) -> AiProviderProbe {
    if !ai_enabled(config) {
        return AiProviderProbe::disabled(config, purpose);
    }
    let ai = config.get("ai").and_then(|value| value.as_table());
    let provider = ai_table_string(ai, "provider", "ollama");
    if provider != "ollama" {
        return AiProviderProbe::unavailable(
            config,
            purpose,
            format!("Unsupported AI provider {provider}; only ollama is implemented in Rust."),
        );
    }
    let base_url = ai_table_string(ai, "base_url", "http://127.0.0.1:11434");
    let model = probe_model_name(ai, purpose);
    let allow_remote = ai_table_bool(ai, "allow_remote", false);
    if !allow_remote && !is_local_base_url(&base_url) {
        return AiProviderProbe::unavailable(
            config,
            purpose,
            "Refusing to connect to non-local Ollama server while ai.allow_remote is false",
        );
    }
    let tags = match ollama_request_json(config, "GET", "/api/tags", None).await {
        Ok(payload) => payload,
        Err(error) => return AiProviderProbe::unavailable(config, purpose, error.to_string()),
    };
    let models = tags
        .get("models")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("name").and_then(JsonValue::as_str))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let installed = !model.is_empty() && models.iter().any(|candidate| candidate == &model);
    AiProviderProbe {
        enabled: true,
        provider,
        base_url,
        model: model.clone(),
        allow_remote,
        available: installed,
        installed,
        resolved_model: (!model.is_empty()).then_some(model.clone()),
        reason: if installed {
            None
        } else {
            Some(format!(
                "Configured model {model} is not installed in Ollama."
            ))
        },
        error: if installed {
            None
        } else {
            Some("model_not_installed".to_string())
        },
    }
}

async fn ollama_chat(
    config: &TomlValue,
    provider: &AiProviderProbe,
    messages: &JsonValue,
) -> Result<String> {
    let payload = json!({
        "model": provider.resolved_model.clone().unwrap_or_else(|| provider.model.clone()),
        "messages": messages,
        "stream": false,
        "options": {
            "temperature": ai_table_f64(config.get("ai").and_then(|value| value.as_table()), "temperature", 1.0),
            "top_p": ai_table_f64(config.get("ai").and_then(|value| value.as_table()), "top_p", 0.95),
            "top_k": ai_table_u64(config.get("ai").and_then(|value| value.as_table()), "top_k", 64) as i64,
            "num_predict": ai_table_u64(config.get("ai").and_then(|value| value.as_table()), "max_tokens", 2048) as i64,
        }
    });
    let response = ollama_request_json(config, "POST", "/api/chat", Some(payload)).await?;
    response
        .get("message")
        .and_then(JsonValue::as_object)
        .and_then(|message| message.get("content"))
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .filter(|content| !content.trim().is_empty())
        .context("Ollama chat response did not include message.content")
}

async fn ollama_request_json(
    config: &TomlValue,
    method: &str,
    path: &str,
    payload: Option<JsonValue>,
) -> Result<JsonValue> {
    let ai = config.get("ai").and_then(|value| value.as_table());
    let base_url = ai_table_string(ai, "base_url", "http://127.0.0.1:11434");
    let token_env = ai_table_string(ai, "token_env", "");
    let max_retries = ai_table_u64(ai, "max_retries", 0);
    let connect_timeout = ai_table_f64(ai, "connect_timeout_seconds", 5.0);
    let total_timeout = ai_table_f64(ai, "timeout_seconds", 30.0);
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs_f64(connect_timeout.max(0.1)))
        .timeout(std::time::Duration::from_secs_f64(total_timeout.max(0.1)))
        .build()
        .context("build Ollama HTTP client")?;
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    );
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 0..=max_retries {
        let request = match method {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            other => bail!("unsupported Ollama request method: {other}"),
        };
        let request = if token_env.is_empty() {
            request
        } else if let Ok(token) = std::env::var(&token_env) {
            request.bearer_auth(token)
        } else {
            request
        };
        let request = if let Some(body) = payload.clone() {
            request.json(&body)
        } else {
            request
        };
        match request.send().await {
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                if !status.is_success() {
                    last_error = Some(anyhow::anyhow!("Ollama HTTP {}: {}", status.as_u16(), body));
                } else {
                    let parsed: JsonValue =
                        serde_json::from_str(&body).context("Ollama returned invalid JSON")?;
                    if parsed.is_object() {
                        return Ok(parsed);
                    }
                    last_error = Some(anyhow::anyhow!(
                        "Ollama returned an unexpected JSON payload"
                    ));
                }
            }
            Err(error) => {
                last_error = Some(anyhow::anyhow!(
                    "Could not connect to Ollama at {}: {}",
                    base_url,
                    error
                ));
            }
        }
        if attempt < max_retries {
            tokio::time::sleep(std::time::Duration::from_millis(250 * (1_u64 << attempt))).await;
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("Could not connect to Ollama at {}", base_url)))
}

fn is_local_base_url(base_url: &str) -> bool {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .map(|host| matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1"))
        .unwrap_or(false)
}

fn redact_bundle_for_ai(bundle: &JsonValue, config: &TomlValue) -> JsonValue {
    let mut redacted = bundle.clone();
    if ai_redact_raw_logs(config) {
        if let Some(events) = redacted.get_mut("events").and_then(JsonValue::as_array_mut) {
            for event in events.iter_mut() {
                *event = summarize_event_for_ai(event);
            }
        }
        if let Some(raw_logs) = redacted
            .pointer_mut("/workspace/selected_app_context/raw_logs")
            .and_then(JsonValue::as_array_mut)
        {
            for log in raw_logs.iter_mut() {
                *log = summarize_raw_log_for_ai(log);
            }
        }
    }
    redacted
}

fn ai_redact_raw_logs(config: &TomlValue) -> bool {
    config
        .get("ai")
        .and_then(|value| value.get("redact_raw_logs"))
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

fn raw_workspace_logs_sent_to_ai(bundle: &JsonValue, config: &TomlValue) -> bool {
    !ai_redact_raw_logs(config)
        && bundle
            .pointer("/workspace/selected_app_context/raw_logs")
            .and_then(JsonValue::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false)
}

fn summarize_event_for_ai(event: &JsonValue) -> JsonValue {
    let message = event
        .get("message")
        .and_then(JsonValue::as_str)
        .or_else(|| event.get("summary").and_then(JsonValue::as_str))
        .unwrap_or_default();
    let summary = if message.len() > 240 {
        format!("{}...", &message[..237])
    } else {
        message.to_string()
    };
    json!({
        "event_id": event.get("event_id").cloned().unwrap_or(JsonValue::Null),
        "timestamp": event.get("timestamp").cloned().unwrap_or(JsonValue::Null),
        "service_id": event.get("service_id").cloned().unwrap_or(JsonValue::Null),
        "severity": event.get("severity").cloned().unwrap_or(JsonValue::Null),
        "summary": summary,
        "tags": event.get("tags").cloned().unwrap_or_else(|| json!([])),
        "source_type": event
            .get("source_ref")
            .and_then(|value| value.get("source_type"))
            .cloned()
            .unwrap_or(JsonValue::Null),
    })
}

fn summarize_raw_log_for_ai(log: &JsonValue) -> JsonValue {
    let line = log
        .get("line")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let summary = mask_sensitive_log_text(if line.len() > 320 {
        format!("{}...", &line[..317])
    } else {
        line.to_string()
    });
    let source = log
        .get("source")
        .and_then(JsonValue::as_object)
        .map(|source| {
            json!({
                "kind": source.get("kind").cloned().unwrap_or(JsonValue::Null),
                "label": source.get("label").cloned().unwrap_or(JsonValue::Null),
                "source": source.get("source").cloned().unwrap_or(JsonValue::Null),
                "path": source.get("path").cloned().unwrap_or(JsonValue::Null),
                "stream": source.get("stream").cloned().unwrap_or(JsonValue::Null),
            })
        })
        .unwrap_or(JsonValue::Null);
    json!({
        "source": source,
        "summary": summary,
        "line_number_from_tail": log.get("line_number_from_tail").cloned().unwrap_or(JsonValue::Null),
        "sampled_at": log.get("sampled_at").cloned().unwrap_or(JsonValue::Null),
    })
}

fn mask_sensitive_log_text(text: String) -> String {
    let mut redact_next = false;
    text.split_whitespace()
        .map(|token| {
            if redact_next {
                redact_next = false;
                return "[REDACTED]".to_string();
            }
            let lower = token.to_ascii_lowercase();
            if contains_sensitive_key(&lower) {
                if let Some(pos) = token.find('=') {
                    return format!("{}=[REDACTED]", &token[..pos]);
                }
                if let Some(pos) = token.find(':') {
                    return format!("{}:[REDACTED]", &token[..pos]);
                }
                return "[REDACTED]".to_string();
            }
            if lower == "bearer" {
                redact_next = true;
                return "Bearer".to_string();
            }
            token.to_string()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_sensitive_key(value: &str) -> bool {
    [
        "password",
        "passwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_key",
        "authorization",
        "cookie",
        "session",
    ]
    .iter()
    .any(|needle| value.contains(needle))
}

fn extract_json_object(raw: &str) -> Option<JsonValue> {
    let trimmed = raw.trim();
    serde_json::from_str::<JsonValue>(trimmed)
        .ok()
        .filter(JsonValue::is_object)
        .or_else(|| {
            let start = trimmed.find('{')?;
            let end = trimmed.rfind('}')?;
            serde_json::from_str::<JsonValue>(&trimmed[start..=end])
                .ok()
                .filter(JsonValue::is_object)
        })
}

fn normalize_investigation_output(raw: JsonValue) -> Option<JsonValue> {
    let object = raw.as_object()?;
    let risk_level = object
        .get("risk_level")
        .and_then(JsonValue::as_str)
        .filter(|value| matches!(*value, "low" | "medium" | "high" | "critical"))
        .unwrap_or("low");
    let confidence = object
        .get("confidence")
        .and_then(JsonValue::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    Some(json!({
        "headline": object.get("headline").and_then(JsonValue::as_str).unwrap_or_default(),
        "risk_level": risk_level,
        "confidence": confidence,
        "what_happened": json_string_array(object.get("what_happened")),
        "why_it_matters": json_string_array(object.get("why_it_matters")),
        "likely_causes": json_string_array(object.get("likely_causes")),
        "evidence": json_evidence_array(object.get("evidence")),
        "missing_evidence": json_string_array(object.get("missing_evidence")),
        "next_steps": json_next_steps(object.get("next_steps")),
        "uncertainty": json_string_array(object.get("uncertainty")),
        "citations": json_string_array(object.get("citations")),
    }))
}

fn json_string_array(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn json_evidence_array(value: Option<&JsonValue>) -> Vec<JsonValue> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items.iter()
                .filter_map(JsonValue::as_object)
                .map(|item| {
                    json!({
                        "type": item.get("type").and_then(JsonValue::as_str).unwrap_or("event"),
                        "id": item.get("id").and_then(JsonValue::as_str).unwrap_or_default(),
                        "summary": item.get("summary").and_then(JsonValue::as_str).unwrap_or_default(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn json_next_steps(value: Option<&JsonValue>) -> Vec<JsonValue> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items.iter()
                .filter_map(JsonValue::as_object)
                .map(|item| {
                    json!({
                        "title": item.get("title").and_then(JsonValue::as_str).unwrap_or_default(),
                        "reason": item.get("reason").and_then(JsonValue::as_str).unwrap_or_default(),
                        "safety": "read_only",
                        "command": "",
                        "requires_user_action": true,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn output_has_signal(output: &JsonValue) -> bool {
    output
        .get("headline")
        .and_then(JsonValue::as_str)
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
        || output
            .get("what_happened")
            .and_then(JsonValue::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false)
        || output
            .get("why_it_matters")
            .and_then(JsonValue::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false)
        || output
            .get("evidence")
            .and_then(JsonValue::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false)
        || output
            .get("next_steps")
            .and_then(JsonValue::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false)
}

fn ai_table_string(ai: Option<&toml::value::Table>, key: &str, default: &str) -> String {
    ai.and_then(|table| table.get(key))
        .and_then(TomlValue::as_str)
        .unwrap_or(default)
        .to_string()
}

fn ai_table_bool(ai: Option<&toml::value::Table>, key: &str, default: bool) -> bool {
    ai.and_then(|table| table.get(key))
        .and_then(TomlValue::as_bool)
        .unwrap_or(default)
}

fn ai_table_u64(ai: Option<&toml::value::Table>, key: &str, default: u64) -> u64 {
    ai.and_then(|table| table.get(key))
        .and_then(|value| value.as_integer().map(|value| value as u64))
        .unwrap_or(default)
}

fn ai_table_f64(ai: Option<&toml::value::Table>, key: &str, default: f64) -> f64 {
    ai.and_then(|table| table.get(key))
        .and_then(|value| {
            value
                .as_float()
                .or_else(|| value.as_integer().map(|value| value as f64))
        })
        .unwrap_or(default)
}

fn deterministic_investigation_response(
    bundle: &JsonValue,
    provider: JsonValue,
    reason: &str,
    warnings: Vec<String>,
    attempts: usize,
    trace: Option<JsonValue>,
) -> JsonValue {
    let incident = bundle.get("incident").cloned().unwrap_or(JsonValue::Null);
    let services = bundle
        .get("services")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let events = bundle
        .get("events")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let workspace = bundle
        .get("workspace")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let context_summary = bundle
        .get("context_summary")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let scope_kind = context_summary
        .get("scope_kind")
        .and_then(JsonValue::as_str)
        .unwrap_or("overview");

    let mut headline_parts = Vec::new();
    let mut risk_level = "low".to_string();

    if let Some(incident_obj) = incident.as_object() {
        let severity = incident_obj
            .get("severity")
            .and_then(JsonValue::as_i64)
            .unwrap_or(0);
        risk_level = if severity >= 3 {
            "high".to_string()
        } else if severity >= 2 {
            "medium".to_string()
        } else {
            "low".to_string()
        };
        headline_parts.push(format!(
            "Incident {} on {}",
            incident_obj
                .get("incident_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown"),
            incident_obj
                .get("primary_service")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
        ));
    } else if scope_kind == "workspace_app" {
        let app_name = context_summary
            .get("app_name")
            .and_then(JsonValue::as_str)
            .unwrap_or("workspace app");
        let runtime = context_summary
            .get("runtime")
            .and_then(JsonValue::as_str)
            .unwrap_or("runtime");
        let matching_event_count = context_summary
            .get("matching_event_count")
            .and_then(JsonValue::as_u64)
            .unwrap_or(events.len() as u64);
        let severe_events = events
            .iter()
            .filter(|event| {
                event
                    .get("severity")
                    .and_then(JsonValue::as_i64)
                    .unwrap_or_default()
                    >= 3
            })
            .count();
        risk_level = if severe_events > 0 { "medium" } else { "low" }.to_string();
        headline_parts.push(format!(
            "Workspace app {app_name} observed as {runtime} with {matching_event_count} matching event(s)"
        ));
    } else if !services.is_empty() {
        let affected: Vec<&JsonValue> = services
            .iter()
            .filter(|item| {
                item.get("status")
                    .and_then(JsonValue::as_str)
                    .map(|status| matches!(status, "degraded" | "critical"))
                    .unwrap_or(false)
            })
            .collect();
        if !affected.is_empty() {
            risk_level = if affected.iter().any(|item| {
                item.get("status")
                    .and_then(JsonValue::as_str)
                    .map(|status| status == "critical")
                    .unwrap_or(false)
            }) {
                "high".to_string()
            } else {
                "medium".to_string()
            };
            headline_parts.push(format!("{} services need attention", affected.len()));
        } else {
            headline_parts.push(format!("{} services observed", services.len()));
        }
    } else {
        headline_parts.push("No active incident; collectors may be quiet".to_string());
    }

    let mut next_steps = Vec::new();
    if let Some(incident_id) = incident
        .as_object()
        .and_then(|item| item.get("incident_id"))
        .and_then(JsonValue::as_str)
    {
        next_steps.push(json!({
            "title": format!("Inspect incident {incident_id}"),
            "reason": "Review hypotheses and supporting evidence locally before any change.",
            "safety": "read_only",
            "command": "",
            "requires_user_action": true,
        }));
    }
    if scope_kind == "workspace_app" {
        let app_name = context_summary
            .get("app_name")
            .and_then(JsonValue::as_str)
            .unwrap_or("this app");
        next_steps.push(json!({
            "title": format!("Review logs and detected signals for {app_name}"),
            "reason": "This scope is a workspace runtime app, so app logs, runtime metadata, and detected project signals are the most relevant evidence.",
            "safety": "read_only",
            "command": "",
            "requires_user_action": true,
        }));
    }
    if let Some(service_id) = services
        .iter()
        .find(|item| {
            item.get("status")
                .and_then(JsonValue::as_str)
                .map(|status| matches!(status, "critical" | "degraded"))
                .unwrap_or(false)
        })
        .or_else(|| services.first())
        .and_then(|item| item.get("service_id"))
        .and_then(JsonValue::as_str)
    {
        next_steps.push(json!({
            "title": format!("Look at recent events for {service_id}"),
            "reason": "Recent events often clarify whether the service is failing or noisy.",
            "safety": "read_only",
            "command": "",
            "requires_user_action": true,
        }));
    }
    if next_steps.is_empty() {
        next_steps.push(json!({
            "title": "Review the latest normalized events",
            "reason": "No active incident is selected, so recent normalized events are the best read-only starting point.",
            "safety": "read_only",
            "command": "",
            "requires_user_action": true,
        }));
    }

    let mut evidence = Vec::new();
    if let Some(incident_id) = incident
        .as_object()
        .and_then(|item| item.get("incident_id"))
        .and_then(JsonValue::as_str)
    {
        evidence.push(json!({
            "type": "incident",
            "id": incident_id,
            "summary": "active incident",
        }));
    }
    for service in services.iter().take(5) {
        evidence.push(json!({
            "type": "service",
            "id": service.get("service_id").and_then(JsonValue::as_str).unwrap_or_default(),
            "summary": service.get("status").and_then(JsonValue::as_str).unwrap_or_default(),
        }));
    }
    for event in events.iter().take(5) {
        evidence.push(json!({
            "type": "event",
            "id": event.get("event_id").and_then(JsonValue::as_str).unwrap_or_default(),
            "summary": event
                .get("summary")
                .or_else(|| event.get("message"))
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
        }));
    }
    if let Some(project_count) = workspace
        .get("projects")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .filter(|count| *count > 0)
    {
        evidence.push(json!({
            "type": "workspace",
            "id": "projects",
            "summary": format!("{project_count} projects detected"),
        }));
    }
    if let Some(selected_app) = workspace.get("selected_app").and_then(JsonValue::as_object) {
        let app_name = selected_app
            .get("name")
            .and_then(JsonValue::as_str)
            .unwrap_or("workspace_app");
        let log_sources = selected_app
            .get("log_sources")
            .and_then(JsonValue::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        let endpoints = selected_app
            .get("endpoints")
            .and_then(JsonValue::as_array)
            .map(Vec::len)
            .unwrap_or_default();
        evidence.push(json!({
            "type": "workspace",
            "id": app_name,
            "summary": format!("{log_sources} log source(s), {endpoints} endpoint(s), runtime metadata attached"),
        }));
    }

    let likely_from_hypotheses: Vec<String> = bundle
        .get("hypotheses")
        .and_then(JsonValue::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|h| {
                    h.get("description")
                        .and_then(JsonValue::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .take(8)
                .collect()
        })
        .unwrap_or_default();

    json!({
        "schema_version": 1,
        "output": {
            "headline": headline_parts.join(" · "),
            "risk_level": risk_level,
            "confidence": 0.4,
            "what_happened": headline_parts,
            "why_it_matters": if matches!(risk_level.as_str(), "high" | "critical") {
                vec!["Severity warrants prompt inspection."]
            } else {
                vec!["No urgent failure observed."]
            },
            "likely_causes": likely_from_hypotheses,
            "evidence": evidence,
            "missing_evidence": vec!["AI provider unavailable; reasoning is deterministic."],
            "next_steps": next_steps,
            "uncertainty": vec![reason.to_string()],
            "citations": Vec::<String>::new(),
        },
        "used_ai": false,
        "fallback_reason": reason,
        "warnings": warnings,
        "attempts": attempts,
        "provider": provider,
        "trace": trace.unwrap_or(JsonValue::Null),
        "bundle": bundle.clone(),
        "grounding": {
            "removed_evidence_ids": [],
            "removed_citation_ids": [],
        },
    })
}

fn investigation_status(error: &anyhow::Error) -> StatusCode {
    let text = error.to_string().to_ascii_lowercase();
    if text.contains("not found") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

async fn proxy_or_static_handler(State(state): State<AppState>, req: Request<Body>) -> Response {
    let uri = req.uri().clone();
    let path = uri.path();
    let method = req.method().clone();
    let cfg = state.config.read().await.clone();
    let ingest = app_ingest_config(&cfg);
    if ingest.enable_main_api && method == axum::http::Method::POST && path == ingest.mount_path {
        let (parts, body) = req.into_parts();
        let headers = parts.headers.clone();
        let bytes = match axum::body::to_bytes(body, ingest.max_payload_bytes).await {
            Ok(bytes) => bytes,
            Err(error) => {
                return (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!("failed to read ingest body: {error}"),
                )
                    .into_response()
            }
        };
        let payload: JsonValue = match serde_json::from_slice(&bytes) {
            Ok(payload) => payload,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("invalid ingest json: {error}"),
                )
                    .into_response()
            }
        };
        return match ingest_payload(&state, &headers, payload).await {
            Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
            Err((status, message)) => (status, message).into_response(),
        };
    }
    if path.starts_with("/api") {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let req_path = uri.path().trim_start_matches('/');
    let file_path = state.ui_dist.join(if req_path.is_empty() {
        "index.html"
    } else {
        req_path
    });
    if file_path.is_file() {
        if let Ok(bytes) = tokio::fs::read(&file_path).await {
            let mime = guess_mime(file_path.extension().and_then(|s| s.to_str()));
            return (StatusCode::OK, [(header::CONTENT_TYPE, mime)], bytes).into_response();
        }
    }
    let index = state.ui_dist.join("index.html");
    if let Ok(bytes) = tokio::fs::read(&index).await {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, HeaderValue::from_static("text/html"))],
            bytes,
        )
            .into_response();
    }
    (StatusCode::NOT_FOUND, "UI bundle not found").into_response()
}

#[derive(Clone)]
struct AppIngestConfig {
    enable_main_api: bool,
    mount_path: String,
    shared_token: String,
    max_payload_bytes: usize,
}

fn app_ingest_config(config: &TomlValue) -> AppIngestConfig {
    let app = config
        .get("collectors")
        .and_then(|value| value.get("app"))
        .and_then(TomlValue::as_table);
    AppIngestConfig {
        enable_main_api: app
            .and_then(|table| table.get("enabled"))
            .and_then(TomlValue::as_bool)
            .unwrap_or(true)
            && app
                .and_then(|table| table.get("enable_main_api"))
                .and_then(TomlValue::as_bool)
                .unwrap_or(true),
        mount_path: app
            .and_then(|table| table.get("mount_path"))
            .and_then(TomlValue::as_str)
            .map(normalize_mount_path)
            .unwrap_or_else(|| "/api/ingest".to_string()),
        shared_token: app
            .and_then(|table| table.get("shared_token"))
            .and_then(TomlValue::as_str)
            .unwrap_or_default()
            .to_string(),
        max_payload_bytes: app
            .and_then(|table| table.get("max_payload_bytes"))
            .and_then(TomlValue::as_integer)
            .unwrap_or(65_536) as usize,
    }
}

fn normalize_mount_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "/api/ingest".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

async fn ingest_payload(
    state: &AppState,
    headers: &axum::http::HeaderMap,
    payload: JsonValue,
) -> Result<JsonValue, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let ingest = app_ingest_config(&cfg);
    if !ingest.enable_main_api {
        return Err((
            StatusCode::NOT_FOUND,
            "main API ingest is disabled".to_string(),
        ));
    }
    if !ingest.shared_token.is_empty() {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let expected = format!("Bearer {}", ingest.shared_token);
        if auth != expected {
            return Err((
                StatusCode::UNAUTHORIZED,
                "missing or invalid bearer token".to_string(),
            ));
        }
    }
    let payload_size = serde_json::to_vec(&payload)
        .map(|bytes| bytes.len())
        .unwrap_or(ingest.max_payload_bytes + 1);
    if payload_size > ingest.max_payload_bytes {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "ingest payload exceeds configured size limit".to_string(),
        ));
    }
    initialize_databases(&state.paths.events_db, &state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let ingest_result = state
        .collectors
        .ingest_app_event(
            &state.paths.events_db,
            &state.paths.incidents_db,
            &cfg,
            &payload,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(json!({
        "accepted": ingest_result.accepted,
        "event_id": ingest_result.event_id,
        "suppressed_duplicates": ingest_result.suppressed_duplicates,
        "suppressed_noise": ingest_result.suppressed_noise,
    }))
}

fn guess_mime(ext: Option<&str>) -> HeaderValue {
    HeaderValue::from_static(match ext {
        Some("js") => "application/javascript",
        Some("css") => "text/css",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("html") => "text/html",
        _ => "application/octet-stream",
    })
}

fn severity_value_to_i64(value: &inferra_contracts::SeverityValue) -> Option<i64> {
    match value {
        inferra_contracts::SeverityValue::Level(level) => Some(*level),
        inferra_contracts::SeverityValue::Label(label) => {
            label
                .parse::<i64>()
                .ok()
                .or_else(|| match label.trim().to_ascii_lowercase().as_str() {
                    "trace" | "debug" => Some(0),
                    "info" | "informational" => Some(1),
                    "warn" | "warning" => Some(2),
                    "error" => Some(3),
                    "critical" | "fatal" | "panic" => Some(4),
                    _ => None,
                })
        }
    }
}

fn discover_projects_with_limits(
    root: &std::path::Path,
    max_depth: usize,
    max_results: usize,
) -> Vec<JsonValue> {
    let markers: &[(&str, &str)] = &[
        ("pnpm-workspace.yaml", "pnpm_workspace"),
        ("package.json", "node"),
        ("yarn.lock", "yarn"),
        ("package-lock.json", "npm"),
        ("pyproject.toml", "python"),
        ("requirements.txt", "python"),
        ("setup.py", "python"),
        ("Pipfile", "python"),
        ("poetry.lock", "python"),
        ("Cargo.toml", "rust"),
        ("go.mod", "go"),
        ("composer.json", "php"),
        ("Gemfile", "ruby"),
        ("pom.xml", "maven"),
        ("build.gradle", "gradle"),
        ("build.gradle.kts", "gradle"),
        ("*.csproj", "dotnet"),
        ("*.sln", "dotnet"),
        ("Makefile", "make"),
        ("compose.yaml", "compose"),
        ("compose.yml", "compose"),
        ("docker-compose.yaml", "compose"),
        ("docker-compose.yml", "compose"),
        ("Dockerfile", "docker"),
        (".git", "git"),
    ];
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .max_depth(max_depth)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_scan_workspace_entry(entry.path()))
        .filter_map(|entry| entry.ok())
    {
        if out.len() >= max_results {
            break;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        for (marker, kind) in markers {
            if workspace_marker_exists(path, marker) {
                let rendered = clean_display_path(&path.to_string_lossy());
                if out.iter().any(|item: &JsonValue| {
                    item.get("path")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        == rendered
                }) {
                    break;
                }
                out.push(json!({
                    "path": rendered,
                    "kind": kind,
                    "marker": marker,
                }));
                break;
            }
        }
    }
    out
}

fn should_scan_workspace_entry(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return true;
    };
    !matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | ".venv"
            | "venv"
            | "__pycache__"
            | "node_modules"
            | "dist"
            | "build"
            | "target"
            | ".cargo"
            | ".npm"
            | ".yarn"
            | ".next"
            | ".nuxt"
            | "coverage"
            | "Library"
            | "AppData"
    )
}

fn workspace_marker_exists(path: &std::path::Path, marker: &str) -> bool {
    if let Some(suffix) = marker.strip_prefix("*.") {
        return std::fs::read_dir(path)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.flatten())
            .any(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case(suffix))
                    .unwrap_or(false)
            });
    }
    path.join(marker).exists()
}

fn clean_display_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

fn workspace_inspect_allowed_roots(config: &TomlValue, paths: &Paths) -> Vec<PathBuf> {
    let base = paths
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut roots = Vec::new();
    if let Ok(root) = base.canonicalize() {
        roots.push(root);
    }
    if let Some(items) = config
        .get("workspace")
        .and_then(|value| value.get("roots"))
        .and_then(TomlValue::as_array)
    {
        for item in items.iter().filter_map(TomlValue::as_str) {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                continue;
            }
            let candidate = PathBuf::from(trimmed);
            let resolved = if candidate.is_absolute() {
                candidate
            } else {
                base.join(candidate)
            };
            if let Ok(root) = resolved.canonicalize() {
                if !roots.iter().any(|existing| existing == &root) {
                    roots.push(root);
                }
            }
        }
    }
    roots
}

fn inspect_workspace_project(path: &std::path::Path, redact_env_files: bool) -> JsonValue {
    let exists = path.exists();
    let is_dir = path.is_dir();
    let mut entries = Vec::new();
    let mut markers = Vec::new();
    for marker in [
        "pnpm-workspace.yaml",
        "package.json",
        "yarn.lock",
        "package-lock.json",
        "pyproject.toml",
        "requirements.txt",
        "setup.py",
        "Pipfile",
        "poetry.lock",
        "Cargo.toml",
        "go.mod",
        "composer.json",
        "Gemfile",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "Makefile",
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
        "Dockerfile",
        ".env",
        ".env.example",
        ".git",
    ] {
        if redact_env_files && marker.starts_with(".env") {
            continue;
        }
        if path.join(marker).exists() {
            markers.push(marker.to_string());
        }
    }
    if exists && is_dir {
        if let Ok(read_dir) = std::fs::read_dir(path) {
            for entry in read_dir.flatten().take(50) {
                let child_path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if redact_env_files && name.to_ascii_lowercase().starts_with(".env") {
                    continue;
                }
                entries.push(json!({
                    "name": name,
                    "kind": if child_path.is_dir() { "dir" } else { "file" },
                }));
            }
        }
    }
    json!({
        "path": clean_display_path(&path.to_string_lossy()),
        "exists": exists,
        "is_dir": is_dir,
        "markers": markers,
        "has_compose": markers.iter().any(|name| matches!(name.as_str(), "compose.yaml" | "compose.yml" | "docker-compose.yaml" | "docker-compose.yml")),
        "has_dockerfile": markers.iter().any(|name| name == "Dockerfile"),
        "has_env_file": markers.iter().any(|name| matches!(name.as_str(), ".env" | ".env.example")),
        "entries": entries,
    })
}

fn ensure_toml_array_path<'a>(
    root: &'a mut TomlValue,
    path: &[&str],
) -> Result<&'a mut Vec<TomlValue>, (StatusCode, String)> {
    if path.is_empty() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "config path must not be empty".into(),
        ));
    }
    let mut current = root;
    for segment in path.iter().take(path.len().saturating_sub(1)) {
        let table = match current {
            TomlValue::Table(table) => table,
            _ => {
                *current = TomlValue::Table(toml::map::Map::new());
                match current {
                    TomlValue::Table(table) => table,
                    _ => {
                        return Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "failed to normalize config table".into(),
                        ));
                    }
                }
            }
        };
        current = table
            .entry((*segment).to_string())
            .or_insert_with(|| TomlValue::Table(toml::map::Map::new()));
    }
    let last = path.last().copied().unwrap();
    let table = match current {
        TomlValue::Table(table) => table,
        _ => {
            *current = TomlValue::Table(toml::map::Map::new());
            match current {
                TomlValue::Table(table) => table,
                _ => {
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "failed to normalize config table".into(),
                    ));
                }
            }
        }
    };
    let slot = table
        .entry((*last).to_string())
        .or_insert_with(|| TomlValue::Array(Vec::new()));
    if !matches!(slot, TomlValue::Array(_)) {
        *slot = TomlValue::Array(Vec::new());
    }
    match slot {
        TomlValue::Array(array) => Ok(array),
        _ => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to allocate config array".into(),
        )),
    }
}

fn topology_edges(config: &TomlValue) -> Vec<JsonValue> {
    config
        .get("topology")
        .and_then(|value| value.get("edges"))
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_table())
        .filter_map(|table| {
            let source = table.get("source").and_then(|value| value.as_str())?;
            let target = table.get("target").and_then(|value| value.as_str())?;
            let relation_type = table
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or("depends_on");
            Some(json!({
                "source": source,
                "target": target,
                "relation_type": relation_type,
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use rusqlite::Connection;
    use serde_json::Value as JsonValue;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};
    use toml::Value as TomlValue;
    use tower::util::ServiceExt;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn test_root(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("inferra-api-{name}-{unique}"))
    }

    fn test_state(name: &str, ai_enabled: bool) -> AppState {
        let root = test_root(name);
        let data_dir = root.join("data");
        let ui_dist = root.join("ui");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&ui_dist).expect("create ui dist");
        std::fs::write(ui_dist.join("index.html"), "<html></html>").expect("write index");

        let config_path = root.join("inferra.toml");
        let config = if ai_enabled {
            load_merged_config(&config_path).expect("load default config")
        } else {
            r#"
[storage]
data_dir = "data"

[collectors]
auto_start = false

[collectors.host_metrics]
enabled = true
poll_interval_seconds = 10.0
warn_cpu_percent = 85.0
warn_memory_percent = 85.0
warn_disk_percent = 90.0

[collectors.process]
enabled = true
poll_interval_seconds = 10.0
top_n = 20
min_cpu_percent = 75.0
min_memory_mb = 512.0
watch_processes = []
watch_pids = []

[collectors.app]
enabled = true
enable_main_api = true

[observability.otlp]
enabled = true

[observability.fts]
enabled = true

[ai]
enabled = false
"#
            .parse::<TomlValue>()
            .expect("parse inline config")
        };

        AppState {
            paths: Arc::new(Paths {
                config_path,
                data_dir: data_dir.clone(),
                events_db: data_dir.join("events.db"),
                incidents_db: data_dir.join("incidents.db"),
            }),
            config: Arc::new(RwLock::new(config)),
            collectors: CollectorRuntime::default(),
            scanner_cache: Arc::new(RwLock::new(ScannerCache::default())),
            ui_dist,
            rate_limits: Arc::new(middleware::RateLimitState::new(30.0, 15.0)),
        }
    }

    fn seeded_test_state(name: &str, ai_enabled: bool) -> AppState {
        let state = test_state(name, ai_enabled);
        seed_test_databases(state.paths.as_ref());
        state
    }

    fn write_adaptive_learning_fixture(paths: &Paths) {
        std::fs::write(
            paths.data_dir.join("adaptive_learning.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "last_updated": "2026-05-08T10:00:00Z",
                "processed_feedback_ids": [],
                "learned_detectors": [{
                    "detector_id": "det-1",
                    "requirement_name": "learned_postgres_timeout",
                    "cause_type": "dependency_failure",
                    "positive_terms": ["postgres", "timeout"],
                    "tags": ["database"],
                    "source_types": ["app"],
                    "min_severity": 3,
                    "confirmations": 2,
                    "false_positives": 0,
                    "created_from_feedback_id": "fb-1",
                    "updated_at": "2026-05-08T10:00:00Z",
                    "manually_disabled": false,
                    "status_reason": null
                }],
                "learned_templates": [],
                "learned_compositions": [],
                "learned_edge_profiles": []
            }))
            .expect("serialize adaptive learning fixture"),
        )
        .expect("write adaptive learning fixture");
    }

    fn write_adaptive_learning_bulk_fixture(paths: &Paths) {
        std::fs::write(
            paths.data_dir.join("adaptive_learning.json"),
            serde_json::to_string_pretty(&json!({
                "schema_version": 1,
                "last_updated": "2026-05-08T10:00:00Z",
                "processed_feedback_ids": [],
                "learned_detectors": [{
                    "detector_id": "det-1",
                    "requirement_name": "learned_postgres_timeout",
                    "cause_type": "dependency_failure",
                    "positive_terms": ["postgres", "timeout"],
                    "tags": ["database"],
                    "source_types": ["app"],
                    "min_severity": 3,
                    "confirmations": 2,
                    "false_positives": 0,
                    "created_from_feedback_id": "fb-1",
                    "updated_at": "2026-05-08T10:00:00Z",
                    "manually_disabled": false,
                    "status_reason": null
                }],
                "learned_templates": [{
                    "template_id": "tpl-1",
                    "template_name": "database timeout chain",
                    "cause_type": "dependency_failure",
                    "cause_subtype": "database",
                    "title_template": "Database dependency timeout",
                    "confidence": 0.84,
                    "requires": ["learned_postgres_timeout"],
                    "requires_same_service": true,
                    "requires_temporal_order": false,
                    "confirmations": 3,
                    "false_positives": 0,
                    "created_from_feedback_id": "fb-1",
                    "updated_at": "2026-05-08T10:01:00Z",
                    "manually_disabled": false,
                    "status_reason": null
                }],
                "learned_compositions": [],
                "learned_edge_profiles": []
            }))
            .expect("serialize adaptive bulk learning fixture"),
        )
        .expect("write adaptive bulk learning fixture");
    }

    fn write_adaptive_learning_history_fixture(paths: &Paths) {
        initialize_databases(&paths.events_db, &paths.incidents_db)
            .expect("initialize adaptive history fixture tables");
        let mut incidents = IncidentsStore::open(&paths.incidents_db)
            .expect("open incidents db")
            .expect("incidents store");
        incidents
            .add_adaptive_learning_history_entries(&[
                inferra_storage::StoredAdaptiveLearningHistoryEntry {
                    entry_id: "hist-1".into(),
                    artifact_kind: "detector".into(),
                    artifact_id: "det-1".into(),
                    artifact_label: "learned_postgres_timeout".into(),
                    incident_id: "inc-1".into(),
                    cause_type: "database".into(),
                    hypothesis_id: "hyp-1".into(),
                    observed_at: "2026-05-08T10:00:00Z".into(),
                    score: Some(0.62),
                    rank: Some(2),
                    estimated_impact: 1.5,
                    impact_metric: Some("matched_events".into()),
                    score_delta: Some(0.12),
                    rank_delta: Some(1),
                    edge_delta: None,
                },
                inferra_storage::StoredAdaptiveLearningHistoryEntry {
                    entry_id: "hist-2".into(),
                    artifact_kind: "edge_profile".into(),
                    artifact_id: "edge-1".into(),
                    artifact_label: "timeout_chain".into(),
                    incident_id: "inc-1".into(),
                    cause_type: "database".into(),
                    hypothesis_id: "hyp-1".into(),
                    observed_at: "2026-05-08T10:05:00Z".into(),
                    score: Some(0.71),
                    rank: Some(1),
                    estimated_impact: 0.08,
                    impact_metric: Some("plausibility_delta".into()),
                    score_delta: Some(0.09),
                    rank_delta: Some(1),
                    edge_delta: Some(0.08),
                },
            ])
            .expect("write adaptive learning history fixture");
    }

    fn write_legacy_adaptive_learning_history_fixture(paths: &Paths) {
        std::fs::write(
            paths.data_dir.join("adaptive_learning_history.jsonl"),
            "{\"entry_id\":\"hist-legacy-1\",\"artifact_kind\":\"detector\",\"artifact_id\":\"det-legacy\",\"artifact_label\":\"legacy_detector\",\"incident_id\":\"inc-1\",\"cause_type\":\"database\",\"hypothesis_id\":\"hyp-1\",\"observed_at\":\"2026-05-08T09:55:00Z\",\"score\":0.58,\"rank\":2,\"estimated_impact\":1.2,\"impact_metric\":\"matched_events\",\"score_delta\":0.08,\"rank_delta\":1,\"edge_delta\":null}\n",
        )
        .expect("write legacy adaptive learning history fixture");
    }

    fn seed_test_databases(paths: &Paths) {
        let now = time::OffsetDateTime::now_utc();
        let fmt = time::format_description::well_known::Rfc3339;
        let ts_evt1 = (now - time::Duration::minutes(5))
            .format(&fmt)
            .expect("format evt1 timestamp");
        let ts_evt2 = (now - time::Duration::minutes(2))
            .format(&fmt)
            .expect("format evt2 timestamp");
        let events = Connection::open(&paths.events_db).expect("open test events db");
        events
            .execute_batch(
                "CREATE TABLE events (
                    event_id TEXT PRIMARY KEY,
                    timestamp TEXT,
                    severity INTEGER,
                    service_id TEXT,
                    message TEXT,
                    source_type TEXT,
                    tags TEXT,
                    trace_id TEXT,
                    span_id TEXT,
                    signal_kind TEXT NOT NULL DEFAULT 'log',
                    deployment_environment TEXT,
                    severity_text TEXT
                );
                DROP TRIGGER IF EXISTS events_fts_ai;
                DROP TRIGGER IF EXISTS events_fts_au;
                DROP TRIGGER IF EXISTS events_fts_ad;
                CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
                    event_id UNINDEXED,
                    message,
                    tokenize = 'unicode61'
                );
                CREATE TRIGGER events_fts_ai AFTER INSERT ON events BEGIN
                    INSERT INTO events_fts(event_id, message)
                    VALUES (new.event_id, substr(new.message, 1, 8192));
                END;
                CREATE TRIGGER events_fts_au AFTER UPDATE ON events BEGIN
                    INSERT INTO events_fts(events_fts, event_id, message)
                    VALUES ('delete', old.event_id, old.message);
                    INSERT INTO events_fts(event_id, message)
                    VALUES (new.event_id, substr(new.message, 1, 8192));
                END;
                CREATE TRIGGER events_fts_ad AFTER DELETE ON events BEGIN
                    INSERT INTO events_fts(events_fts, event_id, message)
                    VALUES ('delete', old.event_id, old.message);
                END;",
            )
            .expect("create events schema");
        events
            .execute(
                "INSERT INTO events (event_id, timestamp, severity, service_id, message, source_type, tags, trace_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    "evt-1",
                    ts_evt1.as_str(),
                    3,
                    "api",
                    "timeout calling postgres",
                    "app",
                    "[\"database\"]",
                    "aabbccdd0011223344556677889900aa",
                ],
            )
            .expect("insert event 1");
        events
            .execute(
                "INSERT INTO events (event_id, timestamp, severity, service_id, message, source_type, tags, trace_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    "evt-2",
                    ts_evt2.as_str(),
                    4,
                    "api",
                    "connection refused from postgres",
                    "app",
                    "[\"database\",\"critical\"]",
                    "aabbccdd0011223344556677889900aa",
                ],
            )
            .expect("insert event 2");
        events
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS event_attributes (
                    event_id TEXT NOT NULL,
                    attr_key TEXT NOT NULL,
                    attr_value_text TEXT,
                    attr_value_num REAL,
                    attr_value_int INTEGER,
                    PRIMARY KEY (event_id, attr_key)
                );
                CREATE INDEX IF NOT EXISTS idx_event_attr_key_text ON event_attributes(attr_key, attr_value_text);
                CREATE INDEX IF NOT EXISTS idx_event_attr_key_num ON event_attributes(attr_key, attr_value_num);
                INSERT OR REPLACE INTO event_attributes (event_id, attr_key, attr_value_text, attr_value_num, attr_value_int)
                    VALUES ('evt-2', 'http.status_code', NULL, NULL, 503);",
            )
            .expect("seed event_attributes");

        let incidents = Connection::open(&paths.incidents_db).expect("open test incidents db");
        incidents
            .execute_batch(
                "CREATE TABLE incidents (
                    incident_id TEXT PRIMARY KEY,
                    state TEXT,
                    severity INTEGER,
                    primary_service TEXT,
                    affected_services TEXT,
                    created_at TEXT,
                    updated_at TEXT,
                    event_count INTEGER
                );
                CREATE TABLE incident_events (
                    incident_id TEXT,
                    event_id TEXT,
                    added_at TEXT
                );
                CREATE TABLE hypotheses (
                    hypothesis_id TEXT PRIMARY KEY,
                    incident_id TEXT,
                    cause_type TEXT,
                    description TEXT,
                    total_score REAL,
                    score_breakdown TEXT NOT NULL DEFAULT '{}',
                    confidence_label TEXT,
                    suggested_checks TEXT,
                    rank INTEGER
                );
                CREATE TABLE incident_clusters (
                    incident_id TEXT,
                    cluster_id TEXT,
                    cluster_data TEXT
                );
                CREATE TABLE IF NOT EXISTS ai_operator_context (
                    scope_key TEXT PRIMARY KEY,
                    body TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                );",
            )
            .expect("create incidents schema");
        incidents
            .execute(
                "INSERT INTO incidents (incident_id, state, severity, primary_service, affected_services, created_at, updated_at, event_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    "inc-1",
                    "open",
                    3,
                    "api",
                    "[\"api\"]",
                    ts_evt1.as_str(),
                    ts_evt2.as_str(),
                    2
                ],
            )
            .expect("insert incident");
        incidents
            .execute(
                "INSERT INTO incident_events (incident_id, event_id, added_at) VALUES (?1, ?2, ?3)",
                rusqlite::params!["inc-1", "evt-1", ts_evt1.as_str()],
            )
            .expect("insert incident event 1");
        incidents
            .execute(
                "INSERT INTO incident_events (incident_id, event_id, added_at) VALUES (?1, ?2, ?3)",
                rusqlite::params!["inc-1", "evt-2", ts_evt2.as_str()],
            )
            .expect("insert incident event 2");
        incidents
            .execute(
                "INSERT INTO hypotheses (hypothesis_id, incident_id, cause_type, description, total_score, score_breakdown, confidence_label, suggested_checks, rank)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    "hyp-1",
                    "inc-1",
                    "database",
                    "Primary datastore is timing out",
                    0.92,
                    "{\"provenance\":{\"has_learned_influence\":true,\"estimated_total_impact\":2.5,\"artifacts\":[{\"kind\":\"detector\",\"artifact_id\":\"det-seeded\",\"label\":\"learned_postgres_timeout\",\"reason\":\"seeded test provenance\",\"impact_metric\":\"matched_events\",\"impact_value\":2.5}]}}",
                    "high",
                    "[\"check postgres latency\"]",
                    1
                ],
            )
            .expect("insert hypothesis");
        incidents
            .execute(
                "INSERT INTO incident_clusters (incident_id, cluster_id, cluster_data) VALUES (?1, ?2, ?3)",
                rusqlite::params!["inc-1", "cluster-1", "{\"kind\":\"database\"}"],
            )
            .expect("insert cluster");
    }

    fn workspace_test_app(name: &str, project_path: &str) -> WorkspaceRuntimeApp {
        WorkspaceRuntimeApp {
            pid: None,
            name: name.into(),
            display_name: Some(name.into()),
            runtime: "nodejs".into(),
            language: Some("nodejs".into()),
            process_kind: Some("server".into()),
            framework: Some("nextjs".into()),
            libraries: Vec::new(),
            log_hints: Vec::new(),
            log_sources: Vec::new(),
            app_url: None,
            endpoints: Vec::new(),
            health_endpoint: None,
            app_location: None,
            resources: None,
            app_state: None,
            context_capabilities: Vec::new(),
            app_structure: Vec::new(),
            manager: None,
            status: Some("running".into()),
            cwd: Some(project_path.into()),
            script: None,
            command: None,
            project_path: Some(project_path.into()),
            latest_trace_summary: None,
            confidence: 0.9,
            source: "process".into(),
            signals: Vec::new(),
        }
    }

    #[test]
    fn workspace_log_tail_reads_bounded_text_and_blocks_env_files() {
        let root = test_root("workspace-log-tail");
        std::fs::create_dir_all(&root).expect("create root");
        let log_path = root.join("app.log");
        std::fs::write(&log_path, "one\ntwo\nthree\n").expect("write log");
        let lines = tail_text_lines(&log_path, 2, 1024).expect("tail log");
        assert_eq!(lines, vec!["two".to_string(), "three".to_string()]);
        assert!(is_sensitive_workspace_log_path(
            &root.join(".env.local").to_string_lossy()
        ));
        assert!(!is_sensitive_workspace_log_path(
            &log_path.to_string_lossy()
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_inspect_roots_and_redaction_are_enforced() {
        let root = test_root("workspace-inspect");
        let project = root.join("project");
        let outside = test_root("workspace-inspect-outside");
        std::fs::create_dir_all(&project).expect("create project");
        std::fs::create_dir_all(&outside).expect("create outside");
        std::fs::write(project.join("package.json"), "{}").expect("write marker");
        std::fs::write(project.join(".env"), "SECRET=value").expect("write env");
        std::fs::write(project.join(".env.example"), "SECRET=").expect("write env example");

        let paths = Paths {
            config_path: root.join("inferra.toml"),
            data_dir: root.join("data"),
            events_db: root.join("data").join("events.db"),
            incidents_db: root.join("data").join("incidents.db"),
        };
        let cfg: TomlValue = r#"
[workspace]
roots = ["project"]
redact_env_files = true
"#
        .parse()
        .expect("parse workspace config");
        let roots = workspace_inspect_allowed_roots(&cfg, &paths);
        let canonical_project = project.canonicalize().expect("canonical project");
        let canonical_outside = outside.canonicalize().expect("canonical outside");
        assert!(roots.iter().any(|root| canonical_project.starts_with(root)));
        assert!(!roots.iter().any(|root| canonical_outside.starts_with(root)));

        let redacted = inspect_workspace_project(&project, true);
        let markers = redacted
            .get("markers")
            .and_then(JsonValue::as_array)
            .expect("markers");
        assert!(markers.iter().any(|item| item.as_str() == Some("package.json")));
        assert!(!markers.iter().any(|item| item.as_str() == Some(".env")));
        assert_eq!(redacted.get("has_env_file"), Some(&JsonValue::Bool(false)));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[test]
    fn redacted_ai_bundle_keeps_workspace_raw_log_context_without_secrets() {
        let config: TomlValue = r#"
[ai]
redact_raw_logs = true
"#
        .parse()
        .expect("parse config");
        let bundle = json!({
            "workspace": {
                "selected_app_context": {
                    "raw_logs": [{
                        "source": {"kind": "file", "label": "App log", "path": "logs/app.log", "source": "project"},
                        "line": "failed login password=hunter2 bearer abc123 token:secret",
                        "line_number_from_tail": 1,
                        "sampled_at": "2026-05-14T00:00:00Z"
                    }]
                }
            }
        });
        let redacted = redact_bundle_for_ai(&bundle, &config);
        let raw_logs = redacted
            .pointer("/workspace/selected_app_context/raw_logs")
            .and_then(JsonValue::as_array)
            .expect("raw logs");
        let summary = raw_logs[0]
            .get("summary")
            .and_then(JsonValue::as_str)
            .expect("summary");
        assert!(summary.contains("password=[REDACTED]"));
        assert!(summary.contains("Bearer [REDACTED]"));
        assert!(summary.contains("token:[REDACTED]"));
        assert!(!summary.contains("hunter2"));
        assert!(!summary.contains("abc123"));
        assert!(raw_logs[0].get("line").is_none());
    }

    async fn get_json(app: Router, path: &str) -> JsonValue {
        let response = app
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        assert_eq!(
            status,
            StatusCode::OK,
            "unexpected status for {path}: {}",
            String::from_utf8_lossy(&body)
        );
        serde_json::from_slice(&body).expect("parse json")
    }

    async fn request_json(
        app: Router,
        method: &str,
        path: &str,
        auth: Option<&str>,
    ) -> (StatusCode, JsonValue) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(auth) = auth {
            builder = builder.header(header::AUTHORIZATION, auth);
        }
        let response = app
            .oneshot(builder.body(Body::empty()).expect("build request"))
            .await
            .expect("router response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let parsed = serde_json::from_slice(&body).unwrap_or_else(|_| {
            json!({
                "text": String::from_utf8_lossy(&body).to_string()
            })
        });
        (status, parsed)
    }

    async fn require_bearer_token(state: &AppState, env_name: &str) {
        let mut cfg = state.config.write().await;
        let root = cfg.as_table_mut().expect("test config table");
        let server = root
            .entry("server".to_string())
            .or_insert_with(|| TomlValue::Table(toml::map::Map::new()))
            .as_table_mut()
            .expect("server table");
        server.insert("require_loopback".into(), TomlValue::Boolean(false));
        server.insert("auth_token_env".into(), TomlValue::String(env_name.into()));
    }

    async fn post_json(
        app: Router,
        path: &str,
        payload: JsonValue,
        auth: Option<&str>,
    ) -> (StatusCode, JsonValue) {
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(auth) = auth {
            builder = builder.header(header::AUTHORIZATION, auth);
        }
        let response = app
            .oneshot(
                builder
                    .body(Body::from(payload.to_string()))
                    .expect("build post request"),
            )
            .await
            .expect("post response");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read post body");
        let parsed = serde_json::from_slice(&body).unwrap_or_else(|_| {
            json!({
                "text": String::from_utf8_lossy(&body).to_string()
            })
        });
        (status, parsed)
    }

    async fn start_mock_ollama(chat_response: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock ollama");
        let addr = listener.local_addr().expect("mock ollama addr");
        let app = Router::new()
            .route(
                "/api/tags",
                get(|| async { Json(json!({ "models": [{ "name": "gemma4:e4b" }] })) }),
            )
            .route(
                "/api/chat",
                post(
                    move || async move { Json(json!({ "message": { "content": chat_response } })) },
                ),
            );
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve mock ollama");
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn middleware_allows_config_put_without_deadlock() {
        let state = test_state("middleware-config-put", false);
        let app = middleware::apply_http_middleware(state.clone(), app_router(state));
        let payload = json!({
            "config": {
                "experience": {
                    "mode": "developer",
                    "show_raw_evidence_by_default": true
                }
            }
        });
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload.to_string()))
                    .expect("build put request"),
            )
            .await
            .expect("put response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read put body");
        let parsed: JsonValue = serde_json::from_slice(&body).expect("parse put json");
        assert_eq!(
            parsed
                .get("config")
                .and_then(|value| value.get("experience"))
                .and_then(|value| value.get("mode"))
                .and_then(|value| value.as_str()),
            Some("developer")
        );
    }

    #[tokio::test]
    async fn probe_routes_remain_minimal_when_api_auth_is_enabled() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let env_name = "INFERRA_TEST_PROBE_TOKEN";
        std::env::set_var(env_name, "correct-token");
        let state = seeded_test_state("probe-auth", false);
        require_bearer_token(&state, env_name).await;
        let app = middleware::apply_http_middleware(state.clone(), app_router(state));

        let (healthz_status, healthz) = request_json(app.clone(), "GET", "/healthz", None).await;
        assert_eq!(healthz_status, StatusCode::OK);
        assert_eq!(healthz.get("runtime").and_then(JsonValue::as_str), Some("rust"));
        assert!(healthz.get("config_path").is_none());

        let (readyz_status, readyz) = request_json(app.clone(), "GET", "/readyz", None).await;
        assert_eq!(readyz_status, StatusCode::OK);
        assert_eq!(readyz.get("storage_writes_ok"), Some(&JsonValue::Bool(true)));
        assert!(readyz.get("events_db").is_none());

        let (api_status, api_body) = request_json(app, "GET", "/api/health", None).await;
        assert_eq!(api_status, StatusCode::UNAUTHORIZED);
        assert_eq!(
            api_body.get("detail").and_then(JsonValue::as_str),
            Some("unauthorized")
        );
        std::env::remove_var(env_name);
    }

    #[tokio::test]
    async fn api_auth_fails_closed_when_env_is_unset_and_accepts_exact_bearer() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let env_name = "INFERRA_TEST_API_TOKEN";
        std::env::remove_var(env_name);
        let state = seeded_test_state("api-auth-unset", false);
        require_bearer_token(&state, env_name).await;
        let app = middleware::apply_http_middleware(state.clone(), app_router(state));

        let (unset_status, unset_body) = request_json(app.clone(), "GET", "/api/health", None).await;
        assert_eq!(unset_status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(unset_body
            .get("detail")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .contains(env_name));

        std::env::set_var(env_name, "correct-token");
        let state = seeded_test_state("api-auth-set", false);
        require_bearer_token(&state, env_name).await;
        let app = middleware::apply_http_middleware(state.clone(), app_router(state));

        let (wrong_status, _) =
            request_json(app.clone(), "GET", "/api/health", Some("Bearer wrong-token")).await;
        assert_eq!(wrong_status, StatusCode::UNAUTHORIZED);

        let (ok_status, ok_body) =
            request_json(app, "GET", "/api/health", Some("Bearer correct-token")).await;
        assert_eq!(ok_status, StatusCode::OK);
        assert_eq!(ok_body.get("runtime").and_then(JsonValue::as_str), Some("rust"));
        std::env::remove_var(env_name);
    }

    #[tokio::test]
    async fn contract_routes_match_frontend_shapes() {
        let app = app_router(seeded_test_state("contracts", false));

        let version = get_json(app.clone(), "/api/version").await;
        assert!(version.get("name").is_some());
        assert!(version.get("version").is_some());
        assert!(version.get("api").is_some());

        let health = get_json(app.clone(), "/api/health").await;
        assert!(health.get("status").is_some());
        assert!(health.get("runtime").is_some());
        assert!(health.get("storage_writes_ok").is_some());
        assert!(health.get("ai_enabled").is_some());

        let config = get_json(app.clone(), "/api/config").await;
        assert!(config.get("config").is_some());

        let overview = get_json(app.clone(), "/api/overview").await;
        assert!(overview.get("quick_analysis").is_some());
        assert!(overview.get("dashboard").is_some());
        assert!(overview.get("runtime").is_some());
        assert!(overview.get("workspace_projects").is_some());
        assert!(overview.get("experience").is_some());
        assert!(overview
            .get("dashboard")
            .and_then(|dashboard| dashboard.get("incidents"))
            .and_then(|items| items.as_array())
            .and_then(|items| items.first())
            .and_then(|incident| incident.get("latest_trace_summary"))
            .and_then(JsonValue::as_object)
            .is_some());
        assert!(overview
            .get("dashboard")
            .and_then(|dashboard| dashboard.get("services"))
            .and_then(|items| items.as_array())
            .and_then(|items| items.first())
            .and_then(|service| service.get("latest_trace_summary"))
            .and_then(JsonValue::as_object)
            .is_some());

        let incidents = get_json(app.clone(), "/api/incidents").await;
        assert!(incidents
            .get("incidents")
            .and_then(|v| v.as_array())
            .is_some());
        assert!(incidents
            .get("incidents")
            .and_then(|items| items.as_array())
            .and_then(|items| items.first())
            .and_then(|incident| incident.get("latest_trace_summary"))
            .and_then(JsonValue::as_object)
            .is_some());

        let services = get_json(app.clone(), "/api/services").await;
        assert!(services
            .get("services")
            .and_then(|v| v.as_array())
            .is_some());
        assert!(services
            .get("services")
            .and_then(|items| items.as_array())
            .and_then(|items| items.first())
            .and_then(|service| service.get("latest_trace_summary"))
            .and_then(JsonValue::as_object)
            .is_some());

        let logs = get_json(app.clone(), "/api/logs?limit=5").await;
        assert!(logs.get("logs").and_then(|v| v.as_array()).is_some());

        let ai_generations = get_json(app.clone(), "/api/ai/generations?limit=5").await;
        assert!(ai_generations
            .get("generations")
            .and_then(|v| v.as_array())
            .is_some());

        let collectors = get_json(app.clone(), "/api/collectors").await;
        assert!(collectors
            .get("collectors")
            .and_then(|v| v.as_array())
            .is_some());
        assert!(collectors.get("queue_depth").is_some());

        let scanner = get_json(app.clone(), "/api/scanner/status").await;
        assert!(scanner.get("scanner").and_then(|v| v.as_object()).is_some());
        assert!(scanner
            .get("scanner")
            .and_then(|v| v.get("workspace"))
            .is_some());

        let workspace_map = get_json(app.clone(), "/api/workspace/map").await;
        assert!(workspace_map.get("enabled").is_some());
        assert!(workspace_map
            .get("projects")
            .and_then(|v| v.as_array())
            .is_some());
        assert!(workspace_map
            .get("service_mappings")
            .and_then(|v| v.as_array())
            .is_some());
        assert!(workspace_map
            .get("unmapped_services")
            .and_then(|v| v.as_array())
            .is_some());
        assert!(workspace_map
            .get("config_mappings")
            .and_then(|v| v.as_array())
            .is_some());
    }

    #[test]
    fn workspace_trace_summary_enrichment_uses_project_service_mapping() {
        let state = seeded_test_state("workspace-trace-summary", false);
        let project_path = "D:/workspace/api";
        let mut workspace = WorkspaceMapResponse {
            enabled: true,
            support_layers: Vec::new(),
            projects: Vec::new(),
            runtime_apps: vec![workspace_test_app("frontend", project_path)],
            service_mappings: vec![WorkspaceMapping {
                service_id: "api".into(),
                project_path: project_path.into(),
                confidence: 0.98,
                source: "test".into(),
                notes: None,
                signals: Vec::new(),
            }],
            unmapped_services: Vec::new(),
            config_mappings: Vec::new(),
        };
        let store = EventsStore::open(&state.paths.events_db)
            .expect("open events db")
            .expect("events store");

        enrich_workspace_runtime_apps_with_latest_traces(&mut workspace, &store)
            .expect("enrich workspace apps");

        let summary = workspace.runtime_apps[0]
            .latest_trace_summary
            .as_ref()
            .expect("latest trace summary");
        assert_eq!(summary.trace_id, "aabbccdd0011223344556677889900aa");
        assert_eq!(summary.event_count, 2);
        assert_eq!(
            summary.sample_message.as_deref(),
            Some("connection refused from postgres")
        );
    }

    #[test]
    fn workspace_app_events_fallback_to_mapped_service_when_app_name_has_no_rows() {
        let state = seeded_test_state("workspace-app-events", false);
        let project_path = "D:/workspace/api";
        let app = workspace_test_app("frontend", project_path);
        let mappings = vec![WorkspaceMapping {
            service_id: "api".into(),
            project_path: project_path.into(),
            confidence: 0.98,
            source: "test".into(),
            notes: None,
            signals: Vec::new(),
        }];
        let store = EventsStore::open(&state.paths.events_db)
            .expect("open events db")
            .expect("events store");

        let rows = workspace_app_events(&store, &app, &mappings, 72, 10)
            .expect("workspace app events");

        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].event_id.as_deref(),
            Some("evt-2"),
            "newest api event should be reused for the mapped workspace app"
        );
        assert!(rows.iter().all(|row| row.service_id.as_deref() == Some("api")));
    }

    #[tokio::test]
    async fn parity_routes_expose_legacy_read_models() {
        let app = app_router(seeded_test_state("parity-routes", false));

        let events = get_json(app.clone(), "/api/events?limit=2").await;
        assert_eq!(
            events["events"].as_array().map(|items| items.len()),
            Some(2)
        );

        let event = get_json(app.clone(), "/api/events/evt-1").await;
        assert_eq!(
            event["event"]["event_id"],
            JsonValue::String("evt-1".into())
        );

        let anomaly = get_json(app.clone(), "/api/anomaly/api/status").await;
        assert_eq!(anomaly["service_id"], JsonValue::String("api".into()));
        assert!(anomaly.get("buckets").is_some());

        let incident_logs = get_json(app.clone(), "/api/incidents/inc-1/logs").await;
        assert_eq!(
            incident_logs["logs"]
                .as_array()
                .map(|items| items.len()),
            Some(2)
        );
        let log_ids: std::collections::HashSet<String> = incident_logs["logs"]
            .as_array()
            .expect("logs")
            .iter()
            .filter_map(|row| row.get("event_id").and_then(|v| v.as_str()).map(str::to_owned))
            .collect();
        assert!(log_ids.contains("evt-1"));
        assert!(log_ids.contains("evt-2"));

        let trace_timeline = get_json(
            app.clone(),
            "/api/traces/aabbccdd0011223344556677889900aa?limit=10",
        )
        .await;
        assert_eq!(
            trace_timeline["items"].as_array().map(|items| items.len()),
            Some(2)
        );

        let workspace_services = get_json(app.clone(), "/api/workspace/services").await;
        assert!(workspace_services
            .get("unmapped_services")
            .and_then(|value| value.as_array())
            .is_some());

        let topology = get_json(app, "/api/topology").await;
        assert!(topology
            .get("edges")
            .and_then(|value| value.as_array())
            .is_some());
    }

    #[tokio::test]
    async fn app_ingest_route_persists_native_event() {
        let state = test_state("app-ingest", false);
        let events_db = state.paths.events_db.clone();
        let incidents_db = state.paths.incidents_db.clone();
        let app = app_router(state);
        let (status, _) = post_json(
            app.clone(),
            "/api/ingest",
            serde_json::json!({
                "timestamp": "2026-05-07T10:10:00Z",
                "level": "error",
                "service": "api",
                "message": "application ingest path",
                "tags": ["app", "ingest"]
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let events = EventsStore::open(&events_db)
            .expect("open events db")
            .expect("events store");
        let latest = events.latest_events(10).expect("latest events");
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].service_id.as_deref(), Some("api"));
        assert_eq!(
            latest[0].message.as_deref(),
            Some("application ingest path")
        );

        let incidents = IncidentsStore::open(&incidents_db)
            .expect("open incidents db")
            .expect("incidents store");
        let active = incidents.active_incidents(10).expect("active incidents");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].primary_service, "api");
        let hypotheses = incidents
            .hypotheses(&active[0].incident_id)
            .expect("incident hypotheses");
        assert!(!hypotheses.is_empty());
    }

    #[tokio::test]
    async fn app_ingest_route_derives_trace_fields_from_traceparent() {
        let state = test_state("app-ingest-traceparent", false);
        let events_db = state.paths.events_db.clone();
        let app = app_router(state);
        let (status, _) = post_json(
            app,
            "/api/ingest",
            serde_json::json!({
                "timestamp": "2026-05-07T10:10:00Z",
                "level": "error",
                "service": "api",
                "message": "application ingest traceparent path",
                "headers": {
                    "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                }
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let events = EventsStore::open(&events_db)
            .expect("open events db")
            .expect("events store");
        let latest = events.latest_events(10).expect("latest events");
        assert_eq!(latest.len(), 1);
        assert_eq!(
            latest[0].trace_id.as_deref(),
            Some("4bf92f3577b34da6a3ce929d0e0e4736")
        );
        assert_eq!(latest[0].span_id.as_deref(), Some("00f067aa0ba902b7"));
    }

    #[tokio::test]
    async fn app_ingest_route_promotes_inferra_trace_keys_from_attributes() {
        let state = test_state("app-ingest-inferra-trace-id", false);
        let events_db = state.paths.events_db.clone();
        let app = app_router(state);
        let (status, _) = post_json(
            app,
            "/api/ingest",
            serde_json::json!({
                "timestamp": "2026-05-07T10:10:00Z",
                "level": "info",
                "service": "worker",
                "message": "background job correlation path",
                "attributes": {
                    "inferra.trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
                    "inferra.span_id": "00f067aa0ba902b7"
                }
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let events = EventsStore::open(&events_db)
            .expect("open events db")
            .expect("events store");
        let latest = events.latest_events(10).expect("latest events");
        assert_eq!(latest.len(), 1);
        assert_eq!(
            latest[0].trace_id.as_deref(),
            Some("4bf92f3577b34da6a3ce929d0e0e4736")
        );
        assert_eq!(latest[0].span_id.as_deref(), Some("00f067aa0ba902b7"));
    }

    #[tokio::test]
    async fn custom_app_mount_path_and_bearer_token_are_honored() {
        let state = test_state("custom-app-ingest", false);
        {
            let mut cfg = state.config.write().await;
            *cfg = apply_config_put(
                cfg.clone(),
                &json!({
                    "collectors": {
                        "app": {
                            "enabled": true,
                            "enable_main_api": true,
                            "mount_path": "/ingest/app",
                            "shared_token": "secret-token",
                            "max_payload_bytes": 4096
                        }
                    }
                }),
            )
            .expect("configure app ingest");
        }
        let app = app_router(state.clone());

        let (unauthorized_status, _) = post_json(
            app.clone(),
            "/ingest/app",
            json!({"message": "unauthorized"}),
            None,
        )
        .await;
        assert_eq!(unauthorized_status, StatusCode::UNAUTHORIZED);

        let (status, body) = post_json(
            app,
            "/ingest/app",
            json!({"service": "gateway", "level": "warn", "message": "custom mount path"}),
            Some("Bearer secret-token"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.get("accepted"), Some(&JsonValue::Bool(true)));

        let events = EventsStore::open(&state.paths.events_db)
            .expect("open events db")
            .expect("events store");
        let latest = events.latest_events(10).expect("latest events");
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].service_id.as_deref(), Some("gateway"));
    }

    #[tokio::test]
    async fn unknown_api_route_returns_not_found_without_proxy() {
        let app = app_router(test_state("api-not-found", false));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/not-real")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        assert_eq!(std::str::from_utf8(&body).expect("utf8 body"), "not found");
    }

    #[tokio::test]
    async fn ai_doctor_reports_disabled_worker_without_proxying() {
        let app = app_router(test_state("ai-doctor", false));
        let doctor = get_json(app, "/api/ai/doctor").await;
        assert_eq!(doctor.get("enabled"), Some(&JsonValue::Bool(false)));
        assert_eq!(doctor.get("ok"), Some(&JsonValue::Bool(true)));
        assert!(doctor.get("provider").is_some());
        assert!(doctor.get("warnings").and_then(|v| v.as_array()).is_some());
    }

    #[tokio::test]
    async fn investigate_now_uses_deterministic_fallback_when_ai_disabled() {
        let app = app_router(seeded_test_state("investigate-now", false));
        let payload = get_json(app, "/api/investigate/now?monitor_seconds=0").await;
        assert_eq!(payload.get("used_ai"), Some(&JsonValue::Bool(false)));
        assert_eq!(
            payload.get("focus"),
            Some(&JsonValue::String("overview".into()))
        );
        assert_eq!(
            payload.get("fallback_reason"),
            Some(&JsonValue::String("AI is disabled in config.".into()))
        );
    }

    #[tokio::test]
    async fn ai_status_and_investigation_use_native_ollama_when_available() {
        let state = seeded_test_state("ai-native", true);
        let base_url = start_mock_ollama(
            "Result: {\"headline\":\"Database saturation on api\",\"risk_level\":\"high\",\"confidence\":0.82,\"what_happened\":[\"Postgres-related failures are clustering on api\"],\"why_it_matters\":[\"User traffic is seeing connection failures\"],\"likely_causes\":[\"Database latency or exhaustion\"],\"evidence\":[{\"type\":\"service\",\"id\":\"api\",\"summary\":\"critical\"}],\"missing_evidence\":[],\"next_steps\":[{\"title\":\"Inspect recent database errors\",\"reason\":\"The latest failures point at postgres connectivity\",\"command\":\"inferra services events api --limit 25\"}],\"uncertainty\":[\"Need direct database saturation metrics to confirm root cause\"],\"citations\":[\"inc-1\",\"evt-2\"]}",
        )
        .await;
        {
            let mut cfg = state.config.write().await;
            *cfg = apply_config_put(
                cfg.clone(),
                &json!({
                    "ai": {
                        "enabled": true,
                        "base_url": base_url,
                        "model": "gemma4:e4b",
                    }
                }),
            )
            .expect("enable native ai");
        }
        let app = app_router(state);

        let status = get_json(app.clone(), "/api/ai/status").await;
        assert_eq!(status.get("available"), Some(&JsonValue::Bool(true)));
        assert_eq!(status.get("installed"), Some(&JsonValue::Bool(true)));

        let payload = get_json(app, "/api/investigate/now?monitor_seconds=0").await;
        assert_eq!(payload.get("used_ai"), Some(&JsonValue::Bool(true)));
        assert_eq!(
            payload
                .get("output")
                .and_then(|value| value.get("headline"))
                .and_then(JsonValue::as_str),
            Some("Database saturation on api")
        );
        assert_eq!(
            payload
                .get("output")
                .and_then(|value| value.get("next_steps"))
                .and_then(JsonValue::as_array)
                .and_then(|steps| steps.first())
                .and_then(|step| step.get("safety"))
                .and_then(JsonValue::as_str),
            Some("read_only")
        );
        assert!(payload.get("trace").is_some());
    }

    #[tokio::test]
    async fn incident_investigation_persists_explanation_and_trace() {
        let state = seeded_test_state("ai-persist", true);
        let base_url = start_mock_ollama(
            "Result: {\"headline\":\"Database saturation on api\",\"risk_level\":\"high\",\"confidence\":0.82,\"what_happened\":[\"Postgres-related failures are clustering on api\"],\"why_it_matters\":[\"User traffic is seeing connection failures\"],\"likely_causes\":[\"Database latency or exhaustion\"],\"evidence\":[{\"type\":\"service\",\"id\":\"api\",\"summary\":\"critical\"}],\"missing_evidence\":[],\"next_steps\":[{\"title\":\"Inspect recent database errors\",\"reason\":\"The latest failures point at postgres connectivity\",\"command\":\"inferra services events api --limit 25\"}],\"uncertainty\":[\"Need direct database saturation metrics to confirm root cause\"],\"citations\":[\"inc-1\",\"evt-2\"]}",
        )
        .await;
        {
            let mut cfg = state.config.write().await;
            *cfg = apply_config_put(
                cfg.clone(),
                &json!({
                    "ai": {
                        "enabled": true,
                        "base_url": base_url,
                        "model": "gemma4:e4b",
                    }
                }),
            )
            .expect("enable native ai");
        }
        let app = app_router(state);
        let payload = get_json(
            app.clone(),
            "/api/investigate/incident/inc-1?monitor_seconds=0",
        )
        .await;
        assert!(payload
            .get("audit")
            .and_then(|audit| audit.get("explanation"))
            .and_then(JsonValue::as_object)
            .is_some());
        assert!(payload
            .get("audit")
            .and_then(|audit| audit.get("latest_trace"))
            .and_then(JsonValue::as_object)
            .is_some());
        assert!(payload.get("cached").is_none());
        assert!(payload
            .get("ai_generation")
            .and_then(JsonValue::as_object)
            .is_some());

        let cached = get_json(
            app.clone(),
            "/api/investigate/incident/inc-1?monitor_seconds=0",
        )
        .await;
        assert_eq!(cached.get("cached"), Some(&JsonValue::Bool(true)));
        assert!(cached
            .get("ai_generation")
            .and_then(JsonValue::as_object)
            .is_some());

        let detail = get_json(app, "/api/incidents/inc-1").await;
        assert!(detail
            .get("explanation")
            .and_then(JsonValue::as_object)
            .is_some());
        assert!(detail
            .get("latest_trace")
            .and_then(JsonValue::as_object)
            .is_some());
    }

    #[tokio::test]
    async fn ai_ask_latest_scope_resolves_to_latest_incident_in_rust() {
        let app = app_router(seeded_test_state("ai-ask-latest", false));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ai/ask")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "question": "what changed most recently?",
                            "scope": "latest",
                            "mode": "expert",
                            "monitor_seconds": 0,
                        }))
                        .expect("serialize request"),
                    ))
                    .expect("build request"),
            )
            .await
            .expect("ai ask response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: JsonValue = serde_json::from_slice(&body).expect("parse json");
        assert_eq!(
            payload.get("focus"),
            Some(&JsonValue::String("incident:inc-1".into()))
        );
        assert_eq!(
            payload.get("question"),
            Some(&JsonValue::String("what changed most recently?".into()))
        );
        assert_eq!(payload.get("used_ai"), Some(&JsonValue::Bool(false)));
        assert!(payload
            .get("ai_generation")
            .and_then(JsonValue::as_object)
            .is_some());
        let saved_generations = get_json(
            app.clone(),
            "/api/ai/generations?scope=incident:inc-1&limit=5",
        )
        .await;
        assert!(saved_generations
            .get("generations")
            .and_then(JsonValue::as_array)
            .is_some_and(|items| !items.is_empty()));

        let cached = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ai/ask")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "question": "what changed most recently?",
                            "scope": "latest",
                            "mode": "expert",
                            "monitor_seconds": 0,
                        }))
                        .expect("serialize request"),
                    ))
                    .expect("build request"),
            )
            .await
            .expect("cached ai ask response");
        assert_eq!(cached.status(), StatusCode::OK);
        let cached_body = axum::body::to_bytes(cached.into_body(), usize::MAX)
            .await
            .expect("read cached body");
        let cached_payload: JsonValue =
            serde_json::from_slice(&cached_body).expect("parse cached json");
        assert_eq!(cached_payload.get("cached"), Some(&JsonValue::Bool(true)));
    }

    #[tokio::test]
    async fn ai_investigate_stream_persists_disabled_fallback_generation() {
        let app = app_router(seeded_test_state("ai-stream-fallback-save", false));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ai/investigate-stream")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "question": "what should I inspect first?",
                            "scope": "latest",
                            "mode": "operator",
                            "monitor_seconds": 0,
                        }))
                        .expect("serialize request"),
                    ))
                    .expect("build request"),
            )
            .await
            .expect("stream response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read stream body");
        let text = String::from_utf8(body.to_vec()).expect("stream body utf8");
        let done_block = text
            .split("\n\n")
            .find(|block| block.contains("event: done"))
            .expect("done event");
        let done_data = done_block
            .lines()
            .find_map(|line| line.strip_prefix("data: "))
            .expect("done data");
        let payload: JsonValue = serde_json::from_str(done_data).expect("parse done data");
        assert_eq!(payload.get("used_ai"), Some(&JsonValue::Bool(false)));
        assert_eq!(
            payload.get("focus"),
            Some(&JsonValue::String("incident:inc-1".into()))
        );
        assert!(payload
            .get("ai_generation")
            .and_then(JsonValue::as_object)
            .is_some());

        let saved_generations =
            get_json(app, "/api/ai/generations?scope=incident:inc-1&limit=5").await;
        assert!(saved_generations
            .get("generations")
            .and_then(JsonValue::as_array)
            .is_some_and(|items| !items.is_empty()));
    }

    #[tokio::test]
    async fn ai_ask_workspace_app_scope_does_not_require_service_row() {
        let state = test_state("ai-ask-workspace-app-scope", false);
        let app_root = state.paths.config_path.parent().unwrap().join("inferra");
        std::fs::create_dir_all(&app_root).expect("create app root");
        std::fs::write(
            app_root.join("package.json"),
            r#"{"name":"inferra","dependencies":{"next":"latest"}}"#,
        )
        .expect("write package");
        std::fs::write(
            &state.paths.config_path,
            format!(
                r#"
[storage]
data_dir = "data"

[ai]
enabled = false

[workspace]
roots = ["{}"]
"#,
                app_root.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .expect("write config");
        let parsed = load_merged_config(&state.paths.config_path).expect("reload config");
        *state.config.write().await = parsed;

        let app = app_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ai/ask")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&json!({
                            "scope": "workspace_app:inferra",
                            "question": "Monitor this workspace app.",
                            "monitor_seconds": 0,
                        }))
                        .expect("serialize request"),
                    ))
                    .expect("build request"),
            )
            .await
            .expect("ai ask response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn logs_route_filters_by_service_and_search() {
        let app = app_router(seeded_test_state("logs", false));
        let payload = get_json(app, "/api/logs?service=api&search=refused&limit=10").await;
        let logs = payload["logs"].as_array().expect("logs array");
        assert_eq!(logs.len(), 1);
        assert_eq!(
            logs[0].get("event_id"),
            Some(&JsonValue::String("evt-2".into()))
        );
    }

    #[tokio::test]
    async fn logs_v2_route_filters_by_indexed_attribute() {
        let app = app_router(seeded_test_state("logs-v2-attr", false));
        let payload = get_json(
            app,
            "/api/v2/logs?service=api&attr_key=http.status_code&attr_value=503&limit=10",
        )
        .await;
        let items = payload["items"].as_array().expect("items");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("event_id"),
            Some(&JsonValue::String("evt-2".into()))
        );
    }

    #[tokio::test]
    async fn logs_v2_q_uses_fts_when_enabled() {
        let app = app_router(seeded_test_state("logs-v2-fts", false));
        let payload = get_json(
            app,
            "/api/v2/logs?service=api&q=refused&limit=10",
        )
        .await;
        assert_eq!(
            payload.get("log_fts_enabled").and_then(JsonValue::as_bool),
            Some(true)
        );
        let items = payload["items"].as_array().expect("items");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("event_id"),
            Some(&JsonValue::String("evt-2".into()))
        );
    }

    #[tokio::test]
    async fn trace_timeline_returns_chronological_items() {
        let app = app_router(seeded_test_state("trace-timeline", false));
        let payload = get_json(
            app,
            "/api/traces/AABBCCDD0011223344556677889900AA?limit=20",
        )
        .await;
        assert_eq!(payload.get("count"), Some(&JsonValue::Number(2.into())));
        let items = payload["items"].as_array().expect("items");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["event_id"], JsonValue::String("evt-1".into()));
        assert_eq!(items[1]["event_id"], JsonValue::String("evt-2".into()));
    }

    #[tokio::test]
    async fn trace_timeline_rejects_malformed_trace_id() {
        let app = app_router(seeded_test_state("trace-bad", false));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/traces/not-a-valid-trace-id")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn otlp_logs_returns_404_when_disabled() {
        let state = test_state("otlp-disabled", false);
        {
            let mut cfg = state.config.write().await;
            let table = cfg.as_table_mut().expect("root table");
            let obs = table
                .entry("observability".to_string())
                .or_insert(TomlValue::Table(Default::default()));
            let obs_table = obs.as_table_mut().expect("observability table");
            let otlp = obs_table
                .entry("otlp".to_string())
                .or_insert(TomlValue::Table(Default::default()));
            let otlp_table = otlp.as_table_mut().expect("otlp table");
            otlp_table.insert("enabled".to_string(), TomlValue::Boolean(false));
        }
        let app = app_router(state);
        let (status, _) = post_json(app, "/v1/logs", json!({ "resourceLogs": [] }), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn otlp_logs_protobuf_ingest_returns_partial_success() {
        use prost::Message;

        let app = app_router(test_state("otlp-proto", false));
        let trace_id = [
            0xca, 0xfe, 0xba, 0xbe, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
            0x00, 0xaa,
        ];
        let request =
            opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest {
                resource_logs: vec![opentelemetry_proto::tonic::logs::v1::ResourceLogs {
                    resource: Some(opentelemetry_proto::tonic::resource::v1::Resource {
                        attributes: vec![opentelemetry_proto::tonic::common::v1::KeyValue {
                            key: "service.name".into(),
                            value: Some(opentelemetry_proto::tonic::common::v1::AnyValue {
                                value: Some(
                                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                                        "api-otlp-proto".into(),
                                    ),
                                ),
                            }),
                            key_strindex: 0,
                        }],
                        dropped_attributes_count: 0,
                        entity_refs: Vec::new(),
                    }),
                    scope_logs: vec![opentelemetry_proto::tonic::logs::v1::ScopeLogs {
                        scope: Some(
                            opentelemetry_proto::tonic::common::v1::InstrumentationScope {
                                name: "api-test".into(),
                                version: "1.0.0".into(),
                                attributes: Vec::new(),
                                dropped_attributes_count: 0,
                            },
                        ),
                        log_records: vec![opentelemetry_proto::tonic::logs::v1::LogRecord {
                            time_unix_nano: 1_715_689_200_000_000_000,
                            observed_time_unix_nano: 0,
                            severity_number: 9,
                            severity_text: "INFO".into(),
                            body: Some(opentelemetry_proto::tonic::common::v1::AnyValue {
                                value: Some(
                                    opentelemetry_proto::tonic::common::v1::any_value::Value::StringValue(
                                        "otlp protobuf ingest probe".into(),
                                    ),
                                ),
                            }),
                            attributes: Vec::new(),
                            dropped_attributes_count: 0,
                            flags: 0,
                            trace_id: trace_id.to_vec(),
                            span_id: Vec::new(),
                            event_name: String::new(),
                        }],
                        schema_url: String::new(),
                    }],
                    schema_url: String::new(),
                }],
            };
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/logs")
                    .header(header::CONTENT_TYPE, "application/x-protobuf")
                    .body(Body::from(request.encode_to_vec()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: JsonValue = serde_json::from_slice(&body).expect("parse payload");
        assert_eq!(
            payload["partialSuccess"]["rejectedLogRecords"],
            JsonValue::Number(0.into())
        );
        let events = get_json(app, "/api/events?limit=20").await;
        let arr = events["events"].as_array().expect("events");
        assert!(arr.iter().any(|e| {
            e.get("message").and_then(|m| m.as_str()) == Some("otlp protobuf ingest probe")
                && e.get("trace_id").and_then(|t| t.as_str())
                    == Some("cafebabe0011223344556677889900aa")
        }));
    }

    #[tokio::test]
    async fn otlp_logs_rejects_invalid_protobuf_payload() {
        let app = app_router(test_state("otlp-proto-invalid", false));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/logs")
                    .header(header::CONTENT_TYPE, "application/x-protobuf")
                    .body(Body::from(vec![0x01, 0x02, 0x03]))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn otlp_logs_returns_415_for_grpc_content_type() {
        let app = app_router(test_state("otlp-grpc", false));
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/logs")
                    .header(header::CONTENT_TYPE, "application/grpc")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn otlp_logs_json_ingest_returns_partial_success() {
        let app = app_router(test_state("otlp-json-api", false));
        let tid = "cafebabe0011223344556677889900aa";
        let payload = json!({
            "resourceLogs": [{
                "resource": {
                    "attributes": [{"key": "service.name", "value": {"stringValue": "api-otlp"}}]
                },
                "scopeLogs": [{
                    "logRecords": [{
                        "timeUnixNano": "1715689200000000000",
                        "severityNumber": 9,
                        "body": {"stringValue": "otlp json ingest probe"},
                        "traceId": tid
                    }]
                }]
            }]
        });
        let (status, body) = post_json(app.clone(), "/v1/logs", payload, None).await;
        assert_eq!(status, StatusCode::OK, "{body:?}");
        assert_eq!(
            body["partialSuccess"]["rejectedLogRecords"],
            JsonValue::Number(0.into())
        );
        let events = get_json(app, "/api/events?limit=20").await;
        let arr = events["events"].as_array().expect("events");
        assert!(arr.iter().any(|e| {
            e.get("message").and_then(|m| m.as_str()) == Some("otlp json ingest probe")
        }));
    }

    #[tokio::test]
    async fn metrics_route_exposes_prometheus_text_payload() {
        let app = app_router(seeded_test_state("metrics", false));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/metrics")
                    .body(Body::empty())
                    .expect("build metrics request"),
            )
            .await
            .expect("metrics response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("text/plain; version=0.0.4"))
        );
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read metrics body");
        let text = String::from_utf8(body.to_vec()).expect("metrics text");
        assert!(text.contains("inferra_events_total"));
        assert!(text.contains("inferra_active_incidents"));
        assert!(text.contains("inferra_observability_export_batches_success_total"));
        assert!(text.contains("inferra_observability_export_retries_total"));
        assert!(text.contains("inferra_raw_queue_depth"));
    }

    #[tokio::test]
    async fn investigate_missing_incident_returns_not_found() {
        let app = app_router(seeded_test_state("missing-incident", false));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/investigate/incident/inc-missing")
                    .body(Body::empty())
                    .expect("build missing incident request"),
            )
            .await
            .expect("missing incident response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn incident_feedback_route_persists_feedback_and_refreshes_reasoning() {
        let state = seeded_test_state("incident-feedback", false);
        let app = app_router(state.clone());
        let (status, payload) = post_json(
            app.clone(),
            "/api/incidents/inc-1/feedback",
            json!({
                "feedback_type": "confirmed",
                "correct_hypothesis_id": "hyp-1",
                "operator_notes": "matched operator judgment"
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            payload.get("stored").and_then(JsonValue::as_bool),
            Some(true)
        );
        assert!(payload
            .get("feedback")
            .and_then(JsonValue::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));

        let detail = get_json(app, "/api/incidents/inc-1").await;
        assert!(detail
            .get("feedback")
            .and_then(JsonValue::as_array)
            .map(|items| !items.is_empty())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn adaptive_learning_route_returns_learned_artifacts() {
        let state = seeded_test_state("adaptive-learning-get", false);
        write_adaptive_learning_fixture(state.paths.as_ref());
        let app = app_router(state);
        let payload = get_json(app, "/api/learning/adaptive").await;
        assert_eq!(
            payload
                .get("counts")
                .and_then(|value| value.get("detectors"))
                .and_then(JsonValue::as_u64),
            Some(1)
        );
        assert_eq!(
            payload["detectors"][0]
                .get("status")
                .and_then(JsonValue::as_str),
            Some("active")
        );
    }

    #[tokio::test]
    async fn adaptive_learning_action_route_can_disable_detector() {
        let state = seeded_test_state("adaptive-learning-disable", false);
        write_adaptive_learning_fixture(state.paths.as_ref());
        let app = app_router(state.clone());
        let (status, payload) = post_json(
            app.clone(),
            "/api/learning/adaptive/detector/det-1",
            json!({
                "action": "disable",
                "reason": "operator retired noisy learned detector"
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            payload["learning"]["detectors"][0]
                .get("manually_disabled")
                .and_then(JsonValue::as_bool),
            Some(true)
        );
        let refreshed = get_json(app, "/api/learning/adaptive").await;
        assert_eq!(
            refreshed["detectors"][0]
                .get("status")
                .and_then(JsonValue::as_str),
            Some("manually_disabled")
        );
    }

    #[tokio::test]
    async fn adaptive_learning_audit_route_returns_governance_actions() {
        let state = seeded_test_state("adaptive-learning-audit", false);
        write_adaptive_learning_fixture(state.paths.as_ref());
        let app = app_router(state.clone());
        let (status, _) = post_json(
            app.clone(),
            "/api/learning/adaptive/detector/det-1",
            json!({
                "action": "disable",
                "reason": "retired for audit test"
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let payload = get_json(app, "/api/learning/adaptive/audit").await;
        assert_eq!(payload.get("count").and_then(JsonValue::as_u64), Some(1));
        assert_eq!(
            payload["entries"][0]
                .get("action")
                .and_then(JsonValue::as_str),
            Some("disable")
        );
    }

    #[tokio::test]
    async fn adaptive_learning_audit_route_supports_query_filters() {
        let state = seeded_test_state("adaptive-learning-audit-filters", false);
        write_adaptive_learning_fixture(state.paths.as_ref());
        let app = app_router(state.clone());
        let (status, _) = post_json(
            app.clone(),
            "/api/learning/adaptive/detector/det-1",
            json!({
                "action": "disable",
                "reason": "filtered audit entry"
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let payload = get_json(
            app,
            "/api/learning/adaptive/audit?action=disable&artifact_kind=detector&offset=0&limit=10",
        )
        .await;
        assert_eq!(payload.get("count").and_then(JsonValue::as_u64), Some(1));
        assert_eq!(
            payload["query"].get("action").and_then(JsonValue::as_str),
            Some("disable")
        );
    }

    #[tokio::test]
    async fn incident_detail_includes_learning_provenance_summary() {
        let app = app_router(seeded_test_state("incident-provenance", false));
        let detail = get_json(app, "/api/incidents/inc-1").await;
        assert_eq!(
            detail["learning_provenance"]
                .get("influenced_hypotheses")
                .and_then(JsonValue::as_u64),
            Some(1)
        );
        assert_eq!(
            detail["learning_provenance"]
                .get("estimated_total_impact")
                .and_then(JsonValue::as_f64),
            Some(2.5)
        );
        assert_eq!(
            detail["hypotheses"][0]
                .get("provenance")
                .and_then(|value| value.get("has_learned_influence"))
                .and_then(JsonValue::as_bool),
            Some(true)
        );
        assert_eq!(
            detail["hypotheses"][0]
                .get("provenance")
                .and_then(|value| value.get("artifacts"))
                .and_then(JsonValue::as_array)
                .and_then(|items| items.first())
                .and_then(|value| value.get("impact_value"))
                .and_then(JsonValue::as_f64),
            Some(2.5)
        );
    }

    #[tokio::test]
    async fn adaptive_learning_review_route_groups_incident_influence_and_attention() {
        let state = seeded_test_state("adaptive-learning-review", false);
        write_adaptive_learning_fixture(state.paths.as_ref());
        write_adaptive_learning_history_fixture(state.paths.as_ref());
        let app = app_router(state.clone());
        let payload = get_json(app.clone(), "/api/learning/adaptive/review").await;
        assert_eq!(
            payload["active_incident_influence"][0]["incident_id"].as_str(),
            Some("inc-1")
        );
        assert!(payload["history_summary"]["count"]
            .as_u64()
            .map(|count| count > 0)
            .unwrap_or(false));
        assert!(payload["comparison_rows"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false));
        assert!(payload["trend_drilldowns"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false));
        assert!(payload["analytics"]["kind_breakdown"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false));
        let (status, _) = post_json(
            app,
            "/api/learning/adaptive/detector/det-1",
            json!({
                "action": "disable",
                "reason": "review workflow retirement"
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let refreshed = get_json(app_router(state), "/api/learning/adaptive/review").await;
        assert!(refreshed["artifacts_requiring_attention"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false));
    }

    #[tokio::test]
    async fn adaptive_learning_review_action_route_records_decision_and_updates_queue() {
        let state = seeded_test_state("adaptive-learning-review-action", false);
        write_adaptive_learning_fixture(state.paths.as_ref());
        let app = app_router(state.clone());
        let initial = get_json(app.clone(), "/api/learning/adaptive/review").await;
        assert_eq!(
            initial["review_queue"].as_array().map(|items| items.len()),
            Some(1)
        );

        let (status, payload) = post_json(
            app.clone(),
            "/api/learning/adaptive/detector/det-1/review",
            json!({
                "decision": "reject",
                "reason": "operator rejected noisy detector"
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            payload["review"]["review_counts"]
                .get("rejected")
                .and_then(JsonValue::as_u64),
            Some(1)
        );
        assert_eq!(
            payload["review"]["recent_review_activity"][0]
                .get("action")
                .and_then(JsonValue::as_str),
            Some("review:reject")
        );

        let refreshed = get_json(app_router(state), "/api/learning/adaptive").await;
        assert_eq!(
            refreshed["detectors"][0]
                .get("review_status")
                .and_then(JsonValue::as_str),
            Some("rejected")
        );
        assert_eq!(
            refreshed["detectors"][0]
                .get("manually_disabled")
                .and_then(JsonValue::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn adaptive_learning_bulk_review_route_updates_multiple_artifacts() {
        let state = seeded_test_state("adaptive-learning-bulk-review", false);
        write_adaptive_learning_bulk_fixture(state.paths.as_ref());
        let app = app_router(state.clone());
        let (status, payload) = post_json(
            app.clone(),
            "/api/learning/adaptive/bulk/review",
            json!({
                "decision": "watch",
                "reason": "bulk watch during triage",
                "artifacts": [
                    { "artifact_kind": "detector", "artifact_id": "det-1" },
                    { "artifact_kind": "template", "artifact_id": "tpl-1" }
                ]
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            payload.get("updated_count").and_then(JsonValue::as_u64),
            Some(2)
        );
        assert_eq!(
            payload["review"]["review_counts"]
                .get("watch")
                .and_then(JsonValue::as_u64),
            Some(2)
        );

        let refreshed = get_json(app_router(state), "/api/learning/adaptive").await;
        assert_eq!(
            refreshed["detectors"][0]
                .get("review_status")
                .and_then(JsonValue::as_str),
            Some("watch")
        );
        assert_eq!(
            refreshed["templates"][0]
                .get("review_status")
                .and_then(JsonValue::as_str),
            Some("watch")
        );
    }

    #[tokio::test]
    async fn adaptive_learning_bulk_state_route_updates_multiple_artifacts() {
        let state = seeded_test_state("adaptive-learning-bulk-state", false);
        write_adaptive_learning_bulk_fixture(state.paths.as_ref());
        let app = app_router(state.clone());
        let (status, payload) = post_json(
            app.clone(),
            "/api/learning/adaptive/bulk/state",
            json!({
                "action": "disable",
                "reason": "bulk runtime retirement",
                "artifacts": [
                    { "artifact_kind": "detector", "artifact_id": "det-1" },
                    { "artifact_kind": "template", "artifact_id": "tpl-1" }
                ]
            }),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            payload.get("updated_count").and_then(JsonValue::as_u64),
            Some(2)
        );
        assert!(payload["review"]["artifacts_requiring_attention"]
            .as_array()
            .map(|items| !items.is_empty())
            .unwrap_or(false));

        let refreshed = get_json(app_router(state), "/api/learning/adaptive").await;
        assert_eq!(
            refreshed["detectors"][0]
                .get("manually_disabled")
                .and_then(JsonValue::as_bool),
            Some(true)
        );
        assert_eq!(
            refreshed["templates"][0]
                .get("manually_disabled")
                .and_then(JsonValue::as_bool),
            Some(true)
        );
    }

    #[tokio::test]
    async fn adaptive_learning_saved_view_routes_persist_and_touch_views() {
        let state = seeded_test_state("adaptive-learning-saved-views", false);
        write_adaptive_learning_bulk_fixture(state.paths.as_ref());
        write_adaptive_learning_history_fixture(state.paths.as_ref());
        let app = app_router(state.clone());

        let (save_status, save_payload) = post_json(
            app.clone(),
            "/api/learning/adaptive/views",
            json!({
                "name": "Database queue",
                "description": "review db artifacts first",
                "search_text": "database",
                "assigned_reviewer": "alice",
                "artifacts": [
                    { "artifact_kind": "detector", "artifact_id": "det-1" },
                    { "artifact_kind": "template", "artifact_id": "tpl-1" }
                ]
            }),
            None,
        )
        .await;
        assert_eq!(save_status, StatusCode::OK);
        let view_id = save_payload
            .get("view_id")
            .and_then(JsonValue::as_str)
            .expect("view id");
        assert_eq!(
            save_payload["review"]["saved_views"][0]
                .get("assigned_reviewer")
                .and_then(JsonValue::as_str),
            Some("alice")
        );

        let (use_status, use_payload) = post_json(
            app.clone(),
            &format!("/api/learning/adaptive/views/{view_id}/use"),
            json!({}),
            None,
        )
        .await;
        assert_eq!(use_status, StatusCode::OK);
        assert!(use_payload["review"]["saved_views"][0]
            .get("last_used_at")
            .and_then(JsonValue::as_str)
            .is_some());

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/learning/adaptive/views/{view_id}"))
                    .body(Body::empty())
                    .expect("build delete request"),
            )
            .await
            .expect("delete saved view response");
        assert_eq!(delete_response.status(), StatusCode::OK);
        let delete_body = axum::body::to_bytes(delete_response.into_body(), usize::MAX)
            .await
            .expect("read delete body");
        let delete_payload: JsonValue =
            serde_json::from_slice(&delete_body).expect("parse delete json");
        assert_eq!(
            delete_payload["review"]["saved_views"]
                .as_array()
                .map(|items| items.len()),
            Some(0)
        );
    }

    #[tokio::test]
    async fn adaptive_learning_history_route_returns_longitudinal_entries() {
        let state = seeded_test_state("adaptive-learning-history", false);
        write_adaptive_learning_history_fixture(state.paths.as_ref());
        let app = app_router(state);
        let payload = get_json(app, "/api/learning/adaptive/history").await;
        assert_eq!(payload.get("count").and_then(JsonValue::as_u64), Some(2));
        assert_eq!(
            payload["entries"][1]
                .get("edge_delta")
                .and_then(JsonValue::as_f64),
            Some(0.08)
        );
    }

    #[tokio::test]
    async fn adaptive_learning_history_route_supports_filters_and_legacy_import() {
        let state = seeded_test_state("adaptive-learning-history-filters", false);
        write_legacy_adaptive_learning_history_fixture(state.paths.as_ref());
        let app = app_router(state);
        let payload = get_json(
            app,
            "/api/learning/adaptive/history?artifact_kind=detector&artifact_id=det-legacy&incident_id=inc-1&limit=5",
        )
        .await;
        assert_eq!(payload.get("count").and_then(JsonValue::as_u64), Some(1));
        assert_eq!(
            payload["entries"][0]
                .get("artifact_id")
                .and_then(JsonValue::as_str),
            Some("det-legacy")
        );
        assert_eq!(
            payload["query"]
                .get("artifact_kind")
                .and_then(JsonValue::as_str),
            Some("detector")
        );
    }

    #[tokio::test]
    async fn collectors_start_and_stop_toggle_runtime_state() {
        let app = app_router(test_state("collectors", false));

        let started = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/collectors/start")
                    .body(Body::empty())
                    .expect("build start request"),
            )
            .await
            .expect("start response");
        assert_eq!(started.status(), StatusCode::OK);

        let running = get_json(app.clone(), "/api/collectors").await;
        let rows = running["collectors"].as_array().expect("collectors array");
        assert!(rows.iter().all(|row| row.get("is_running").is_some()));
        assert!(rows
            .iter()
            .any(|row| row.get("is_running") == Some(&JsonValue::Bool(true))));

        let stopped = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/collectors/stop")
                    .body(Body::empty())
                    .expect("build stop request"),
            )
            .await
            .expect("stop response");
        assert_eq!(stopped.status(), StatusCode::OK);

        let stopped_collectors = get_json(app, "/api/collectors").await;
        let statuses = stopped_collectors["collectors"]
            .as_array()
            .expect("collectors array")
            .iter()
            .all(|row| row.get("is_running") == Some(&JsonValue::Bool(false)));
        assert!(statuses);
    }

    #[test]
    fn grounding_removes_unknown_evidence_ids() {
        let bundle = json!({
            "events": [{"event_id": "evt-1"}],
            "services": [{"service_id": "api"}],
            "incident": {"incident_id": "inc-1"},
        });
        let mut output = json!({
            "evidence": [
                {"type": "event", "id": "evt-1", "summary": "ok"},
                {"type": "event", "id": "evt-fake", "summary": "bad"},
            ],
            "citations": ["evt-1", "evt-nope", "api"],
        });
        let g = apply_output_grounding(&mut output, &bundle);
        assert_eq!(output["evidence"].as_array().unwrap().len(), 1);
        let removed = g["removed_evidence_ids"].as_array().unwrap();
        assert!(removed.iter().any(|v| v == "event:evt-fake"));
        let rc = g["removed_citation_ids"].as_array().unwrap();
        assert!(rc.iter().any(|v| v == "evt-nope"));
    }
}
