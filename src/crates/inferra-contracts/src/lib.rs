//! JSON shapes aligned with `src/web/frontend/src/api.ts` for stable UI contracts.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SeverityValue {
    Level(i64),
    Label(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigResponse {
    pub config: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiVersionResponse {
    pub name: String,
    pub version: String,
    pub api: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExperiencePayload {
    pub mode: String,
    pub ai_role: String,
    pub suggest_safe_actions: bool,
    pub execute_actions: bool,
    pub show_raw_evidence_by_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickAnalysis {
    pub headline: String,
    pub risk_level: String,
    pub containers_running: usize,
    pub process_sample_size: usize,
    pub code_projects_found: usize,
    pub mode: String,
    pub ai_role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentRow {
    pub incident_id: String,
    pub state: String,
    pub severity: i64,
    pub primary_service: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected_services: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRow {
    pub service_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_incidents: Option<Vec<IncidentRow>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardHealth {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_incidents: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_depth: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector_errors: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded_reasons: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_writes_ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir_bytes_free: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<DashboardHealth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incidents: Option<Vec<IncidentRow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<Vec<ServiceRow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedup: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_rate: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity_counts: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeContainer {
    pub name: String,
    pub image: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeProcess {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f64,
    pub memory_mb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub containers: Option<Vec<RuntimeContainer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processes: Option<Vec<RuntimeProcess>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceProject {
    pub path: String,
    pub kind: String,
    pub marker: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceRuntimeApp {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub name: String,
    pub runtime: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_hints: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manager: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_path: Option<String>,
    pub confidence: f64,
    pub source: String,
    pub signals: Vec<WorkspaceMappingSignal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSupportItem {
    pub id: String,
    pub label: String,
    pub support_type: String,
    pub detects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_hints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WorkspaceSupportItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSupportLayer {
    pub layer: String,
    pub title: String,
    pub description: String,
    pub items: Vec<WorkspaceSupportItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverviewResponse {
    pub quick_analysis: QuickAnalysis,
    pub dashboard: DashboardPayload,
    pub runtime: RuntimeContext,
    pub workspace_projects: Vec<WorkspaceProject>,
    pub experience: ExperiencePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorRow {
    pub collector_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_running: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events_emitted: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub events_per_second: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped_events: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lag_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectorsResponse {
    pub collectors: Vec<CollectorRow>,
    pub queue_depth: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiStatusResponse {
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_remote: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry_model: Option<Value>,
    /// Effective model used for `/api/ai/status` probe when `ai.model_status` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_model: Option<String>,
    /// Effective model used for investigation/chat when `ai.model_investigate` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub investigate_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMappingSignal {
    pub name: String,
    pub confidence: f64,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMapping {
    pub service_id: String,
    pub project_path: String,
    pub confidence: f64,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub signals: Vec<WorkspaceMappingSignal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMapResponse {
    pub enabled: bool,
    #[serde(default)]
    pub support_layers: Vec<WorkspaceSupportLayer>,
    pub projects: Vec<WorkspaceProject>,
    #[serde(default)]
    pub runtime_apps: Vec<WorkspaceRuntimeApp>,
    pub service_mappings: Vec<WorkspaceMapping>,
    pub unmapped_services: Vec<String>,
    pub config_mappings: Vec<WorkspaceMapping>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSourceRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<SeverityValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<EventSourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypothesisRow {
    pub hypothesis_id: String,
    pub cause_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_checks: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentDetailResponse {
    pub incident: IncidentRow,
    pub events: Vec<EventRow>,
    pub hypotheses: Vec<HypothesisRow>,
    pub clusters: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_trace: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state_log: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feedback: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learning_provenance: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDetailResponse {
    pub service: ServiceRow,
    pub events: Vec<EventRow>,
    pub incidents: Vec<IncidentRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiDoctorResponse {
    pub ok: bool,
    pub enabled: bool,
    pub provider: String,
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub investigate_model: Option<String>,
    pub allow_remote: bool,
    pub token_env_set: bool,
    pub redact_raw_logs: bool,
    pub available: bool,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance: Option<Vec<String>>,
}
