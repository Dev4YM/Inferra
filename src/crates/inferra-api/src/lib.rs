//! Axum HTTP server: native `/api/*` plus static UI.

use anyhow::{bail, Context, Result};
use async_stream::stream;
use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{sse::Event, sse::KeepAlive, sse::Sse, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures_util::Stream;
use inferra_collectors::{configured_collectors, CollectorRuntime};
use inferra_config::{
    apply_config_put, config_to_json, experience_from_config, load_merged_config, resolve_data_dir,
    server_listen, Paths,
};
use inferra_contracts::{
    AiDoctorResponse, AiStatusResponse, ApiVersionResponse, CollectorRow, CollectorsResponse,
    ConfigResponse, IncidentDetailResponse, IncidentRow, OverviewResponse, ServiceDetailResponse,
    WorkspaceMapResponse,
};
use inferra_core::{
    ai_status_from_config, build_overview, build_workspace_map, collect_host_resources_snapshot,
    collect_runtime_monitor_window, try_collect_gpu_summary,
};
use inferra_storage::{
    initialize_databases, EventsStore, IncidentsStore, StoredAiTrace, StoredExplanation,
};
use serde_json::{json, Value as JsonValue};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use toml::Value as TomlValue;

const INVESTIGATION_MAX_AI_ATTEMPTS: usize = 3;
const INVESTIGATION_SYSTEM_PROMPT: &str = "You are Inferra's read-only investigation assistant.\n\
You receive a redacted runtime evidence bundle. You must:\n\
- explain what is happening using only the supplied facts\n\
- prioritize the next inspection step the operator should take\n\
- cite supporting incident_id, service_id, or event_id values when possible\n\
- never claim you executed or modified anything\n\
- never propose remediation that would mutate the observed system\n\
- include explicit uncertainty when evidence is thin\n\
- respect host_resources and runtime_monitor samples as authoritative for machine state during this investigation window\n\
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
      \"command\": \"string\",\n\
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
    pub ui_dist: PathBuf,
}

pub fn app_router(state: AppState) -> Router {
    Router::new()
        .route("/api/version", get(api_version))
        .route("/api/health", get(api_health))
        .route("/api/config", get(api_get_config).put(api_put_config))
        .route("/api/overview", get(api_overview))
        .route("/api/metrics", get(api_metrics))
        .route("/api/events", get(api_events))
        .route("/api/events/{event_id}", get(api_event_detail))
        .route("/api/anomaly/{service_id}/status", get(api_anomaly_status))
        .route("/api/logs", get(api_logs))
        .route("/api/incidents", get(api_incidents))
        .route("/api/incidents/{incident_id}", get(api_incident_detail))
        .route(
            "/api/incidents/{incident_id}/events",
            get(api_incident_events),
        )
        .route(
            "/api/incidents/{incident_id}/hypotheses",
            get(api_incident_hypotheses),
        )
        .route(
            "/api/incidents/{incident_id}/clusters",
            get(api_incident_clusters),
        )
        .route("/api/services", get(api_services))
        .route("/api/services/{service_id}", get(api_service_detail))
        .route("/api/services/{service_id}/events", get(api_service_events))
        .route("/api/ai/status", get(api_ai_status))
        .route("/api/ai/doctor", get(api_ai_doctor))
        .route("/api/ai/ask", post(api_ai_ask))
        .route("/api/ai/investigate-stream", post(api_ai_investigate_stream))
        .route("/api/ai/context", get(api_ai_context_get).put(api_ai_context_put))
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
        .route("/api/ingest", post(api_ingest))
        .route("/api/workspace/projects", get(api_workspace_projects))
        .route("/api/workspace/map", get(api_workspace_map))
        .route("/api/workspace/services", get(api_workspace_services))
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
    let state = AppState {
        paths: Arc::new(paths),
        config: Arc::new(RwLock::new(merged)),
        collectors: CollectorRuntime::default(),
        ui_dist,
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
    let app = app_router(state);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("inferra Rust runtime listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await?;
    Ok(())
}

async fn api_version() -> Json<ApiVersionResponse> {
    Json(ApiVersionResponse {
        name: "inferra".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        api: "1".into(),
    })
}

async fn api_health(State(state): State<AppState>) -> Json<JsonValue> {
    let cfg = state.config.read().await;
    let paths = state.paths.as_ref();
    let storage_ok = paths.data_dir.exists();
    Json(json!({
        "status": if storage_ok { "ok" } else { "degraded" },
        "runtime": "rust",
        "storage_writes_ok": storage_ok,
        "config_path": paths.config_path,
        "data_dir": paths.data_dir,
        "ai_enabled": cfg.get("ai").and_then(|a| a.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false),
    }))
}

async fn api_get_config(State(state): State<AppState>) -> Json<ConfigResponse> {
    let cfg = state.config.read().await;
    Json(ConfigResponse {
        config: config_to_json(&cfg),
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
    inferra_config::write_config(&state.paths.config_path, &new_cfg)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    *state.config.write().await = new_cfg.clone();
    Ok(Json(ConfigResponse {
        config: config_to_json(&new_cfg),
        applied: Some(true),
    }))
}

async fn api_overview(
    State(state): State<AppState>,
) -> Result<Json<OverviewResponse>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let mut overview = build_overview(&cfg, state.paths.as_ref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let probe = probe_ai_provider(&cfg, AiProbePurpose::Status).await;
    if let Some(health) = overview.dashboard.health.as_mut() {
        health.ai_enabled = Some(probe.enabled);
        health.ai_available = Some(probe.available);
        health.ai_reason = probe.reason.clone();
        health.queue_depth = Some(state.collectors.queue_depth());
        health.collector_errors = Some(state.collectors.total_errors().await);
    }
    Ok(Json(overview))
}

async fn api_metrics(State(state): State<AppState>) -> Response {
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
    let payload = format!(
        "# HELP inferra_events_total Approximate stored normalized events.\n# TYPE inferra_events_total counter\ninferra_events_total {event_count}\n# HELP inferra_active_incidents Active incidents (open, investigating, explained).\n# TYPE inferra_active_incidents gauge\ninferra_active_incidents {active_incidents}\n# HELP inferra_raw_queue_depth In-flight ingestion operations.\n# TYPE inferra_raw_queue_depth gauge\ninferra_raw_queue_depth {queue_depth}\n"
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
    let store = EventsStore::open(&state.paths.events_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let logs = if let Some(store) = store {
        store
            .query_logs(
                limit,
                params.get("service").map(String::as_str),
                severity,
                params.get("search").map(String::as_str),
                params.get("source_type").map(String::as_str),
            )
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        vec![]
    };
    Ok(Json(json!({ "logs": logs, "limit": limit })))
}

async fn api_incidents(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let store = IncidentsStore::open(&state.paths.incidents_db)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let incidents = if let Some(store) = store {
        store
            .active_incidents(100)
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
    Ok(Json(IncidentDetailResponse {
        incident,
        events: event_rows,
        hypotheses,
        clusters,
        explanation,
        latest_trace,
        state_log,
        feedback,
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

async fn api_services(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let overview = build_overview(&cfg, state.paths.as_ref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "services": overview.dashboard.services.unwrap_or_default(),
    })))
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
        model: probe.resolved_model.clone().unwrap_or_else(|| probe.model.clone()),
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

async fn api_investigate_now(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    let mode = current_mode(&cfg, params.get("mode").map(String::as_str));
    let focus = "overview".to_string();
    let monitor = resolve_monitor_seconds(&cfg, params.get("monitor_seconds"), None);
    let bundle = investigation_bundle_enriched(
        state.paths.as_ref(),
        &cfg,
        &focus,
        "",
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
        None,
        false,
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
    let bundle = investigation_bundle_enriched(
        state.paths.as_ref(),
        &cfg,
        &focus,
        "",
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
        None,
        false,
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
    let bundle = investigation_bundle_enriched(
        state.paths.as_ref(),
        &cfg,
        &focus,
        "",
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
        None,
        false,
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
    let bundle = investigation_bundle_enriched(
        state.paths.as_ref(),
        &cfg,
        &focus,
        "",
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
        None,
        true,
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
) -> Result<Json<JsonValue>, (StatusCode, String)> {
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
    if let Some(question) = question {
        response["question"] = JsonValue::String(question);
    }
    if report {
        response["report"] = JsonValue::Bool(true);
    }
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
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "scope query parameter is required".into()))?;
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
                        yield Ok(Event::default().event("delta").data(p.to_string()));
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
                    "raw_logs_sent": false,
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
        let _ = persist_investigation_artifacts(paths.as_ref(), &bundle, &response);
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
    Json(CollectorsResponse {
        collectors: configured_collectors(&cfg)
            .into_iter()
            .map(|c| CollectorRow {
                collector_id: c.collector_id.clone(),
                status: runtime_by_id
                    .get(&c.collector_id)
                    .map(|row| row.status.clone())
                    .or(Some(c.status)),
                source_type: Some(c.source_type),
                is_running: runtime_by_id.get(&c.collector_id).map(|row| row.is_running),
                events_emitted: runtime_by_id
                    .get(&c.collector_id)
                    .map(|row| row.events_emitted),
                events_per_second: runtime_by_id
                    .get(&c.collector_id)
                    .map(|row| row.events_per_second),
                last_event_at: runtime_by_id
                    .get(&c.collector_id)
                    .and_then(|row| row.last_event_at.clone()),
                error_count: runtime_by_id
                    .get(&c.collector_id)
                    .map(|row| row.error_count),
                dropped_events: runtime_by_id
                    .get(&c.collector_id)
                    .map(|row| row.dropped_events),
                last_error: runtime_by_id
                    .get(&c.collector_id)
                    .and_then(|row| row.last_error.clone()),
                lag_seconds: runtime_by_id
                    .get(&c.collector_id)
                    .and_then(|row| row.lag_seconds),
            })
            .collect(),
        queue_depth: state.collectors.queue_depth(),
    })
}

async fn api_collectors_start(
    State(state): State<AppState>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let current = state.config.read().await.clone();
    if let Ok(next) = apply_config_put(current, &json!({ "collectors": { "auto_start": true } })) {
        let _ = inferra_config::write_config(&state.paths.config_path, &next);
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
        let _ = inferra_config::write_config(&state.paths.config_path, &next);
        *state.config.write().await = next;
    }
    state
        .collectors
        .stop()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({ "stopped": true, "desired_state": "stopped" })))
}

async fn api_ingest(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<JsonValue>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let accepted = ingest_payload(&state, &headers, payload).await?;
    Ok(Json(accepted))
}

async fn api_workspace_map(
    State(state): State<AppState>,
) -> Result<Json<WorkspaceMapResponse>, (StatusCode, String)> {
    let cfg = state.config.read().await.clone();
    build_workspace_map(&cfg, state.paths.as_ref())
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
    let workspace = build_workspace_map(&cfg, state.paths.as_ref())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "service_mappings": workspace.service_mappings,
        "unmapped_services": workspace.unmapped_services,
    })))
}

async fn api_workspace_inspect(
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<JsonValue>, (StatusCode, String)> {
    let Some(path) = params.get("path").cloned() else {
        return Err((StatusCode::BAD_REQUEST, "path is required".to_string()));
    };
    if path.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "path is required".to_string()));
    }
    Ok(Json(inspect_workspace_project(std::path::Path::new(&path))))
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
    let mappings = ensure_toml_array_path(&mut config, &["workspace", "service_mappings"]);
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
    Json(json!({ "edges": topology_edges(&cfg) }))
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
    let edges = ensure_toml_array_path(&mut config, &["topology", "edges"]);
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
        "raw_logs_sent": false,
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
                        if let Some(w) = hypothesis_rank_alignment_warning(&output, &redacted_bundle) {
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
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serialized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn artifact_id(prefix: &str, incident_id: &str, seed: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    format!(
        "{prefix}-{incident_id}-{}-{:016x}",
        unix_seconds(),
        hasher.finish()
    )
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
    let Some(store) = (match IncidentsStore::open(&paths.incidents_db) {
        Ok(s) => s,
        Err(_) => None,
    }) else {
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
    top.sort_by(|a, b| b.1.cmp(&a.1));
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
    rows.sort_by(|a, b| b.0.cmp(&a.0));
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

fn collect_allowed_ids_from_bundle(bundle: &JsonValue) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
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
    if let Some(arr) = bundle.get("similar_incidents").and_then(JsonValue::as_array) {
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
    if let Some(arr) = output.get_mut("citations").and_then(JsonValue::as_array_mut) {
        arr.retain(|c| {
            let id = c.as_str().unwrap_or("");
            let ok = allowed_ev.contains(id) || allowed_svc.contains(id) || allowed_inc.contains(id);
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
}

fn resolve_focus_from_scope(paths: &Paths, scope: &str) -> Result<String> {
    let trimmed = scope.trim();
    if trimmed.eq_ignore_ascii_case("latest") {
        return Ok(latest_incident_focus(paths)?.unwrap_or_else(|| "latest:none".to_string()));
    }
    if trimmed.starts_with("incident:") || trimmed.starts_with("service:") {
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
    let redact_raw_logs = config
        .get("ai")
        .and_then(|value| value.get("redact_raw_logs"))
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    if redact_raw_logs {
        if let Some(events) = redacted.get_mut("events").and_then(JsonValue::as_array_mut) {
            for event in events.iter_mut() {
                *event = summarize_event_for_ai(event);
            }
        }
    }
    redacted
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
                        "command": item.get("command").and_then(JsonValue::as_str).unwrap_or_default(),
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
            "command": format!("inferra incidents show {incident_id}"),
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
            "command": format!("inferra services events {service_id} --limit 25"),
            "requires_user_action": true,
        }));
    }
    if next_steps.is_empty() {
        next_steps.push(json!({
            "title": "List recent events",
            "reason": "No active incident; sample events to understand current activity.",
            "safety": "read_only",
            "command": "inferra events list --limit 25",
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
        inferra_contracts::SeverityValue::Label(label) => label.parse::<i64>().ok().or_else(|| {
            match label.trim().to_ascii_lowercase().as_str() {
                "trace" | "debug" => Some(0),
                "info" | "informational" => Some(1),
                "warn" | "warning" => Some(2),
                "error" => Some(3),
                "critical" | "fatal" | "panic" => Some(4),
                _ => None,
            }
        }),
    }
}

fn discover_projects_with_limits(
    root: &std::path::Path,
    max_depth: usize,
    max_results: usize,
) -> Vec<JsonValue> {
    let markers: &[(&str, &str)] = &[
        ("package.json", "node"),
        ("Cargo.toml", "rust"),
        ("go.mod", "go"),
        ("pyproject.toml", "python"),
        (".git", "git"),
    ];
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .max_depth(max_depth)
        .into_iter()
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
            if path.join(marker).exists() {
                let rendered = path.to_string_lossy().to_string();
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

fn inspect_workspace_project(path: &std::path::Path) -> JsonValue {
    let exists = path.exists();
    let is_dir = path.is_dir();
    let mut entries = Vec::new();
    let mut markers = Vec::new();
    for marker in [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "go.mod",
        ".git",
    ] {
        if path.join(marker).exists() {
            markers.push(marker.to_string());
        }
    }
    if exists && is_dir {
        if let Ok(read_dir) = std::fs::read_dir(path) {
            for entry in read_dir.flatten().take(50) {
                let child_path = entry.path();
                entries.push(json!({
                    "name": entry.file_name().to_string_lossy().to_string(),
                    "kind": if child_path.is_dir() { "dir" } else { "file" },
                }));
            }
        }
    }
    json!({
        "path": path.to_string_lossy().to_string(),
        "exists": exists,
        "is_dir": is_dir,
        "markers": markers,
        "entries": entries,
    })
}

fn ensure_toml_array_path<'a>(root: &'a mut TomlValue, path: &[&str]) -> &'a mut Vec<TomlValue> {
    let mut current = root;
    for segment in path.iter().take(path.len().saturating_sub(1)) {
        let table = match current {
            TomlValue::Table(table) => table,
            _ => {
                *current = TomlValue::Table(toml::map::Map::new());
                match current {
                    TomlValue::Table(table) => table,
                    _ => unreachable!(),
                }
            }
        };
        current = table
            .entry((*segment).to_string())
            .or_insert_with(|| TomlValue::Table(toml::map::Map::new()));
    }
    let last = path.last().expect("non-empty path");
    let table = match current {
        TomlValue::Table(table) => table,
        _ => {
            *current = TomlValue::Table(toml::map::Map::new());
            match current {
                TomlValue::Table(table) => table,
                _ => unreachable!(),
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
        TomlValue::Array(array) => array,
        _ => unreachable!(),
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
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::util::ServiceExt;

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
            ui_dist,
        }
    }

    fn seeded_test_state(name: &str, ai_enabled: bool) -> AppState {
        let state = test_state(name, ai_enabled);
        seed_test_databases(state.paths.as_ref());
        state
    }

    fn seed_test_databases(paths: &Paths) {
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
                    tags TEXT
                );",
            )
            .expect("create events schema");
        events
            .execute(
                "INSERT INTO events (event_id, timestamp, severity, service_id, message, source_type, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    "evt-1",
                    "2026-05-07T09:00:00Z",
                    3,
                    "api",
                    "timeout calling postgres",
                    "app",
                    "[\"database\"]"
                ],
            )
            .expect("insert event 1");
        events
            .execute(
                "INSERT INTO events (event_id, timestamp, severity, service_id, message, source_type, tags)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    "evt-2",
                    "2026-05-07T09:05:00Z",
                    4,
                    "api",
                    "connection refused from postgres",
                    "app",
                    "[\"database\",\"critical\"]"
                ],
            )
            .expect("insert event 2");

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
                    "2026-05-07T09:00:00Z",
                    "2026-05-07T09:05:00Z",
                    2
                ],
            )
            .expect("insert incident");
        incidents
            .execute(
                "INSERT INTO incident_events (incident_id, event_id, added_at) VALUES (?1, ?2, ?3)",
                rusqlite::params!["inc-1", "evt-1", "2026-05-07T09:00:00Z"],
            )
            .expect("insert incident event 1");
        incidents
            .execute(
                "INSERT INTO incident_events (incident_id, event_id, added_at) VALUES (?1, ?2, ?3)",
                rusqlite::params!["inc-1", "evt-2", "2026-05-07T09:05:00Z"],
            )
            .expect("insert incident event 2");
        incidents
            .execute(
                "INSERT INTO hypotheses (hypothesis_id, incident_id, cause_type, description, total_score, confidence_label, suggested_checks, rank)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    "hyp-1",
                    "inc-1",
                    "database",
                    "Primary datastore is timing out",
                    0.92,
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

        let incidents = get_json(app.clone(), "/api/incidents").await;
        assert!(incidents
            .get("incidents")
            .and_then(|v| v.as_array())
            .is_some());

        let services = get_json(app.clone(), "/api/services").await;
        assert!(services
            .get("services")
            .and_then(|v| v.as_array())
            .is_some());

        let logs = get_json(app.clone(), "/api/logs?limit=5").await;
        assert!(logs.get("logs").and_then(|v| v.as_array()).is_some());

        let collectors = get_json(app.clone(), "/api/collectors").await;
        assert!(collectors
            .get("collectors")
            .and_then(|v| v.as_array())
            .is_some());
        assert!(collectors.get("queue_depth").is_some());

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

        let incident_events = get_json(app.clone(), "/api/incidents/inc-1/events").await;
        assert_eq!(
            incident_events["events"]
                .as_array()
                .map(|items| items.len()),
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
        let payload = get_json(app.clone(), "/api/investigate/incident/inc-1?monitor_seconds=0").await;
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
