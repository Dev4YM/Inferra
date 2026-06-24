//! Native overview assembly for the Rust runtime.

mod resource_snapshot;

pub use resource_snapshot::{
    collect_host_resources_snapshot, collect_runtime_monitor_window, try_collect_gpu_summary,
};

use anyhow::Result;
use inferra_config::{experience_from_config, Paths};
use inferra_contracts::{
    AiStatusResponse, DashboardHealth, DashboardPayload, EventRow, IncidentRow, OverviewResponse,
    QuickAnalysis, RuntimeContainer, RuntimeContext, RuntimeProcess, ServiceRow, SeverityValue,
    WorkspaceAppCapability, WorkspaceAppEndpoint, WorkspaceAppLocation, WorkspaceAppResources,
    WorkspaceAppState, WorkspaceAppStructureItem, WorkspaceLogSource, WorkspaceMapResponse,
    WorkspaceMapping, WorkspaceMappingSignal, WorkspaceProject, WorkspaceRuntimeApp,
    WorkspaceSupportItem, WorkspaceSupportLayer,
};
use inferra_storage::{
    AdaptiveLearningAuditQuery, AdaptiveLearningHistoryQuery, EventsStore, GovernanceSummary,
    IncidentRecord, IncidentsStore, ServiceStats, StoredAdaptiveLearningAuditEntry,
    StoredAdaptiveLearningHistoryEntry, StoredAdaptiveReviewView,
    StoredAdaptiveReviewViewSelection, StoredHypothesis, StoredInferenceGraphSnapshot,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use sysinfo::{Disks, System};
use toml::Value as TomlValue;
use walkdir::WalkDir;

const SEVERITY_INFO: i64 = 1;
const SEVERITY_WARN: i64 = 2;
const SEVERITY_ERROR: i64 = 3;

#[derive(Debug, Clone, Default)]
pub struct OverviewRuntimeSignals {
    pub ai_available: Option<bool>,
    pub ai_reason: Option<String>,
    pub queue_depth: Option<i64>,
    pub collector_errors: Option<i64>,
}

#[derive(Debug, Clone)]
struct Candidate {
    cause_type: String,
    description: String,
    prior: f64,
    score: f64,
    suggested_checks: Vec<String>,
    supporting_events: Vec<String>,
    affected_services: Vec<String>,
    contradicting_events: Vec<String>,
    dependency_proximity: f64,
    is_valid: bool,
    invalidation_reasons: Vec<String>,
    root_cause_event_id: Option<String>,
    root_cause_timestamp: Option<String>,
    scoring: CandidateScoring,
    provenance_refs: Vec<LearningArtifactRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LearningArtifactRef {
    kind: String,
    artifact_id: String,
    label: String,
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    impact_metric: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    impact_value: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AdaptiveLearningAuditEntry {
    audit_id: String,
    artifact_kind: String,
    artifact_id: String,
    action: String,
    reason: Option<String>,
    previous_status: String,
    new_status: String,
    review_status_before: Option<String>,
    review_status_after: Option<String>,
    runtime_effect: Option<String>,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AdaptiveLearningHistoryEntry {
    entry_id: String,
    artifact_kind: String,
    artifact_id: String,
    artifact_label: String,
    incident_id: String,
    cause_type: String,
    hypothesis_id: String,
    observed_at: String,
    score: Option<f64>,
    rank: Option<i64>,
    estimated_impact: f64,
    impact_metric: Option<String>,
    score_delta: Option<f64>,
    rank_delta: Option<i64>,
    edge_delta: Option<f64>,
}

#[derive(Debug, Clone, Default)]
struct CandidateScoring {
    temporal_alignment: f64,
    correlation_strength: f64,
    frequency_weight: f64,
    dependency_proximity: f64,
    evidence_coverage: f64,
    anomaly_severity: f64,
    graph_impact: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalibrationBucket {
    score_lower: f64,
    score_upper: f64,
    total_predictions: u64,
    correct_predictions: u64,
    accuracy: f64,
    sample_confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CalibrationModel {
    schema_version: u64,
    last_updated: Option<String>,
    total_feedback_count: u64,
    overall_accuracy: f64,
    staleness_status: String,
    buckets: Vec<CalibrationBucket>,
    processed_feedback_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WeightAdjustmentAudit {
    feedback_id: String,
    incident_id: String,
    applied_at: String,
    adjustments: std::collections::BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LearnedScoringWeights {
    schema_version: u64,
    last_updated: Option<String>,
    default_weights: std::collections::BTreeMap<String, f64>,
    effective_weights: std::collections::BTreeMap<String, f64>,
    processed_feedback_ids: Vec<String>,
    audit: Vec<WeightAdjustmentAudit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AdaptiveLearningModel {
    schema_version: u64,
    last_updated: Option<String>,
    processed_feedback_ids: Vec<String>,
    learned_detectors: Vec<LearnedDetector>,
    learned_templates: Vec<LearnedTemplate>,
    #[serde(default)]
    learned_compositions: Vec<LearnedComposition>,
    #[serde(default)]
    learned_edge_profiles: Vec<LearnedEdgeProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LearnedDetector {
    detector_id: String,
    requirement_name: String,
    cause_type: String,
    positive_terms: Vec<String>,
    tags: Vec<String>,
    source_types: Vec<String>,
    min_severity: Option<i64>,
    confirmations: u64,
    false_positives: u64,
    created_from_feedback_id: String,
    updated_at: String,
    #[serde(default)]
    manually_disabled: bool,
    #[serde(default)]
    status_reason: Option<String>,
    #[serde(default = "default_review_status")]
    review_status: String,
    #[serde(default)]
    review_reason: Option<String>,
    #[serde(default)]
    last_reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LearnedTemplate {
    template_id: String,
    template_name: String,
    cause_type: String,
    cause_subtype: Option<String>,
    title_template: String,
    confidence: f64,
    requires: Vec<String>,
    requires_same_service: bool,
    requires_temporal_order: bool,
    confirmations: u64,
    false_positives: u64,
    created_from_feedback_id: String,
    updated_at: String,
    #[serde(default)]
    manually_disabled: bool,
    #[serde(default)]
    status_reason: Option<String>,
    #[serde(default = "default_review_status")]
    review_status: String,
    #[serde(default)]
    review_reason: Option<String>,
    #[serde(default)]
    last_reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LearnedComposition {
    composition_id: String,
    composition_name: String,
    cause_type: String,
    cause_subtype: Option<String>,
    title_template: String,
    confidence: f64,
    requires: Vec<String>,
    requires_same_service: bool,
    requires_temporal_order: bool,
    preferred_edge_types: Vec<String>,
    confirmations: u64,
    false_positives: u64,
    created_from_feedback_id: String,
    updated_at: String,
    #[serde(default)]
    manually_disabled: bool,
    #[serde(default)]
    status_reason: Option<String>,
    #[serde(default = "default_review_status")]
    review_status: String,
    #[serde(default)]
    review_reason: Option<String>,
    #[serde(default)]
    last_reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LearnedEdgeProfile {
    profile_id: String,
    edge_type: String,
    source_service: Option<String>,
    target_service: Option<String>,
    cause_type: Option<String>,
    confirmations: u64,
    false_positives: u64,
    average_plausibility: f64,
    average_latency_ms: f64,
    created_from_feedback_id: String,
    updated_at: String,
    #[serde(default)]
    manually_disabled: bool,
    #[serde(default)]
    status_reason: Option<String>,
    #[serde(default = "default_review_status")]
    review_status: String,
    #[serde(default)]
    review_reason: Option<String>,
    #[serde(default)]
    last_reviewed_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct LearningArtifacts {
    calibration: CalibrationModel,
    weights: LearnedScoringWeights,
    adaptive: AdaptiveLearningModel,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct InferenceGraph {
    nodes: Vec<InferenceNode>,
    edges: Vec<InferenceEdge>,
    root_candidates: Vec<String>,
    leaf_nodes: Vec<String>,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InferenceNode {
    event_id: String,
    service_id: String,
    timestamp: String,
    severity: i64,
    summary: String,
    node_type: String,
    in_degree: usize,
    out_degree: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InferenceEdge {
    source_event_id: String,
    target_event_id: String,
    edge_type: String,
    plausibility: f64,
    latency_ms: f64,
    evidence: String,
    requires: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    learned_adjustments: Vec<InferenceEdgeAdjustment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InferenceEdgeAdjustment {
    artifact_id: String,
    artifact_label: String,
    baseline_plausibility: f64,
    adjusted_plausibility: f64,
    delta: f64,
}

#[derive(Debug, Clone)]
struct GraphEvent {
    event_id: String,
    service_id: String,
    timestamp: time::OffsetDateTime,
    timestamp_raw: String,
    severity: i64,
    summary: String,
    source_type: String,
    tags: Vec<String>,
}

/// Build `/api/overview` from local SQLite + lightweight host snapshot.
pub fn build_overview(config: &TomlValue, paths: &Paths) -> Result<OverviewResponse> {
    build_overview_with_runtime_signals(config, paths, None)
}

/// Build `/api/overview`, optionally enriching health with live runtime signals.
pub fn build_overview_with_runtime_signals(
    config: &TomlValue,
    paths: &Paths,
    runtime_signals: Option<&OverviewRuntimeSignals>,
) -> Result<OverviewResponse> {
    run_incident_lifecycle_maintenance(paths, config)?;
    let experience = experience_from_config(config);
    let ai_enabled = config
        .get("ai")
        .and_then(|a| a.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let incidents_db = IncidentsStore::open(&paths.incidents_db)?;
    let events_db = EventsStore::open(&paths.events_db)?;

    let incidents: Vec<IncidentRow> = if let Some(ref db) = incidents_db {
        db.active_incidents(10)?
    } else {
        vec![]
    };
    let incidents = enrich_incident_rows_with_latest_traces(
        incidents,
        incidents_db.as_ref(),
        events_db.as_ref(),
    )?;

    let active_n = if let Some(ref db) = incidents_db {
        db.active_incident_count()?
    } else {
        0
    };

    let service_stats: Vec<ServiceStats> = if let Some(ref db) = events_db {
        db.service_aggregates(30)?
    } else {
        vec![]
    };
    let recent_events = if let Some(ref db) = events_db {
        db.latest_events(200)?
    } else {
        vec![]
    };
    let governance = if let Some(ref db) = events_db {
        db.governance_summary()?
    } else {
        GovernanceSummary::default()
    };
    let severity_counts = severity_counts_payload(&recent_events);
    let event_rate = event_rate_payload(&recent_events);
    let dedup = dedup_payload(config, recent_events.len(), &governance);
    let noise = noise_payload(config, &event_rate, &governance);

    let services = enrich_service_rows(&service_stats, &incidents, events_db.as_ref())?;
    let containers = discover_runtime_containers();
    let containers_running = containers.as_ref().map(|items| items.len()).unwrap_or(0);
    let runtime_signals = runtime_signals.cloned().unwrap_or_default();

    let storage_ok = paths.data_dir.exists();
    let mut degraded_reasons: Vec<String> = Vec::new();
    if !storage_ok {
        degraded_reasons.push("data directory does not exist yet".into());
    }
    if runtime_signals.collector_errors.unwrap_or_default() > 0 {
        degraded_reasons.push(format!(
            "{} collector error(s) reported by the active runtime",
            runtime_signals.collector_errors.unwrap_or_default()
        ));
    }
    if ai_enabled && runtime_signals.ai_available == Some(false) {
        degraded_reasons.push(
            runtime_signals
                .ai_reason
                .clone()
                .unwrap_or_else(|| "AI provider probe reported unavailable".into()),
        );
    }

    let free_bytes = disk_free_near(&paths.data_dir);

    let mut sys = System::new_all();
    sys.refresh_all();
    let logical_processors = system_logical_processors(&sys);
    let hostname = System::host_name();
    let mut processes: Vec<RuntimeProcess> = sys
        .processes()
        .values()
        .map(|p| {
            let mem = p.memory() as f64 / (1024.0 * 1024.0);
            let raw_cpu = f64::from(p.cpu_usage());
            RuntimeProcess {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_percent: normalize_process_cpu_to_host_percent(raw_cpu, logical_processors),
                cpu_raw_percent: Some(round_f64(raw_cpu, 2)),
                cpu_percent_scope: Some("host_total".into()),
                cpu_logical_processors: Some(logical_processors),
                memory_mb: mem,
            }
        })
        .collect();
    processes.sort_by(|left, right| {
        right
            .cpu_percent
            .partial_cmp(&left.cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                right
                    .memory_mb
                    .partial_cmp(&left.memory_mb)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    processes.truncate(60);

    let runtime = RuntimeContext {
        hostname,
        containers,
        processes: Some(processes),
    };

    let scan_root = paths.config_path.parent().unwrap_or(Path::new("."));
    let projects = discover_projects(config, scan_root);

    let incidents_n = active_n;
    let degraded = !storage_ok || !degraded_reasons.is_empty();
    let mut summary_parts: Vec<String> = Vec::new();
    if incidents_n > 0 {
        summary_parts.push(format!("{incidents_n} active incident(s)."));
    } else {
        summary_parts.push("No active incidents.".into());
    }
    if degraded && !storage_ok {
        summary_parts.push("Data directory missing; run setup or adjust storage.data_dir.".into());
    }
    if let Some(top) = incidents.first() {
        summary_parts.push(format!(
            "Top incident: {} (severity {}).",
            top.primary_service, top.severity
        ));
    }
    if ai_enabled {
        if runtime_signals.ai_available == Some(true) {
            summary_parts.push("AI enabled and provider probe succeeded.".into());
        } else if let Some(reason) = runtime_signals.ai_reason.as_deref() {
            summary_parts.push(format!("AI enabled but unavailable: {reason}."));
        } else {
            summary_parts
                .push("AI enabled; availability is resolved by the active runtime.".into());
        }
    }

    let risk = if degraded || incidents_n > 0 {
        "high"
    } else {
        "low"
    };

    let quick = QuickAnalysis {
        headline: summary_parts.join(" "),
        risk_level: risk.into(),
        containers_running,
        process_sample_size: runtime.processes.as_ref().map(|p| p.len()).unwrap_or(0),
        code_projects_found: projects.len(),
        mode: experience.mode.clone(),
        ai_role: experience.ai_role.clone(),
    };

    let health = DashboardHealth {
        status: Some((if degraded { "degraded" } else { "ok" }).into()),
        active_incidents: Some(incidents_n),
        queue_depth: runtime_signals.queue_depth,
        collector_errors: runtime_signals.collector_errors,
        degraded: Some(degraded),
        degraded_reasons: Some(if degraded_reasons.is_empty() && !storage_ok {
            vec!["storage path missing".into()]
        } else {
            degraded_reasons
        }),
        storage_writes_ok: Some(storage_ok),
        data_dir_bytes_free: free_bytes,
        ai_enabled: Some(ai_enabled),
        ai_available: runtime_signals.ai_available,
        ai_reason: runtime_signals.ai_reason,
    };

    let dashboard = DashboardPayload {
        health: Some(health),
        incidents: Some(incidents.clone()),
        services: Some(services),
        dedup: Some(dedup),
        noise: Some(noise),
        event_rate: Some(event_rate),
        severity_counts: Some(severity_counts),
    };

    Ok(OverviewResponse {
        quick_analysis: quick,
        dashboard,
        runtime,
        workspace_projects: projects,
        experience,
    })
}

fn is_infrastructure_service_id(service_id: &str) -> bool {
    matches!(
        service_id.trim().to_ascii_lowercase().as_str(),
        "host" | "localhost"
    )
}

pub fn build_workspace_map(config: &TomlValue, paths: &Paths) -> Result<WorkspaceMapResponse> {
    let enabled = workspace_enabled(config);
    let scan_root = paths.config_path.parent().unwrap_or(Path::new("."));
    let mut projects = if enabled {
        discover_projects(config, scan_root)
    } else {
        Vec::new()
    };
    let runtime_apps = if enabled {
        discover_workspace_runtime_apps(&projects)
    } else {
        Vec::new()
    };
    for app in &runtime_apps {
        if let Some(project_path) = app.project_path.as_deref() {
            if projects.iter().any(|project| project.path == project_path) {
                continue;
            }
            if let Some(project) = workspace_project_from_path(Path::new(project_path)) {
                projects.push(project);
            }
        }
    }
    let service_stats = if let Some(db) = EventsStore::open(&paths.events_db)? {
        db.service_aggregates(200)?
    } else {
        vec![]
    };
    let mut service_ids: Vec<String> = service_stats.iter().map(|s| s.service_id.clone()).collect();
    for app in &runtime_apps {
        if !service_ids.iter().any(|id| id == &app.name) {
            service_ids.push(app.name.clone());
        }
    }
    let config_mappings = config_workspace_mappings(config);
    let mut service_mappings = config_mappings.clone();
    let explicit_keys: std::collections::HashSet<(String, String)> = config_mappings
        .iter()
        .map(|m| (m.service_id.clone(), m.project_path.clone()))
        .collect();
    for app in &runtime_apps {
        if let Some(mapping) = mapping_from_runtime_app(app) {
            upsert_workspace_mapping(&mut service_mappings, mapping, &explicit_keys);
        }
    }
    for service_id in &service_ids {
        if service_mappings
            .iter()
            .any(|mapping| mapping.service_id == *service_id)
        {
            continue;
        }
        if let Some(mapping) = mapping_from_service_tokens(service_id, &projects) {
            upsert_workspace_mapping(&mut service_mappings, mapping, &explicit_keys);
        }
    }
    let mapped_services: std::collections::HashSet<String> = service_mappings
        .iter()
        .map(|m| m.service_id.clone())
        .collect();
    let unmapped_services = service_ids
        .into_iter()
        .filter(|s| enabled && !mapped_services.contains(s) && !is_infrastructure_service_id(s))
        .collect();
    Ok(WorkspaceMapResponse {
        enabled,
        support_layers: workspace_support_layers(),
        projects,
        runtime_apps,
        service_mappings,
        unmapped_services,
        config_mappings,
    })
}

pub fn workspace_app_live_resources(
    pid: Option<u32>,
    name: Option<&str>,
) -> Option<WorkspaceAppResources> {
    let mut sys = System::new_all();
    sys.refresh_all();
    let logical_processors = system_logical_processors(&sys);
    let name = name.map(|value| value.to_ascii_lowercase());
    let process = sys.processes().values().find(|process| {
        pid.map(|pid| process.pid().as_u32() == pid)
            .unwrap_or(false)
            || name
                .as_ref()
                .map(|needle| {
                    process
                        .name()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .contains(needle)
                })
                .unwrap_or(false)
    })?;
    let raw_cpu = f64::from(process.cpu_usage());
    Some(WorkspaceAppResources {
        cpu_percent: Some(normalize_process_cpu_to_host_percent(
            raw_cpu,
            logical_processors,
        )),
        cpu_raw_percent: Some(round_f64(raw_cpu, 2)),
        cpu_percent_scope: Some("host_total".into()),
        cpu_logical_processors: Some(logical_processors),
        memory_mb: Some(round_f64(process.memory() as f64 / (1024.0 * 1024.0), 2)),
        virtual_memory_mb: Some(round_f64(
            process.virtual_memory() as f64 / (1024.0 * 1024.0),
            2,
        )),
        uptime_seconds: Some(process.run_time()),
        process_status: Some(format!("{:?}", process.status())),
    })
}

pub fn ai_status_from_config(config: &TomlValue) -> AiStatusResponse {
    let enabled = config
        .get("ai")
        .and_then(|a| a.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let base_url = config
        .get("ai")
        .and_then(|a| a.get("base_url"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let model = config
        .get("ai")
        .and_then(|a| a.get("model"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let allow_remote = config
        .get("ai")
        .and_then(|a| a.get("allow_remote"))
        .and_then(|v| v.as_bool());

    AiStatusResponse {
        enabled,
        provider: config
            .get("ai")
            .and_then(|a| a.get("provider"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| Some("ollama".into())),
        resolved_model: model.clone(),
        model,
        base_url,
        available: None,
        installed: None,
        reason: if enabled {
            Some("AI health is resolved by the runtime API.".into())
        } else {
            None
        },
        error: None,
        allow_remote,
        registry_model: None,
        status_model: config
            .get("ai")
            .and_then(|a| a.get("model_status"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
        investigate_model: config
            .get("ai")
            .and_then(|a| a.get("model_investigate"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
    }
}

pub fn reconcile_new_events(
    events_db: &Path,
    incidents_db: &Path,
    config: &TomlValue,
    event_ids: &[String],
) -> Result<Vec<String>> {
    if event_ids.is_empty() {
        return Ok(vec![]);
    }
    let Some(events) = EventsStore::open(events_db)? else {
        return Ok(vec![]);
    };
    let Some(mut incidents) = IncidentsStore::open(incidents_db)? else {
        return Ok(vec![]);
    };
    let learning = sync_learning_artifacts(config, events_db, incidents_db, &mut incidents)?;
    run_incident_lifecycle_maintenance_with_store(config, events_db, incidents_db, &mut incidents)?;
    let inserted = events.get_events(event_ids)?;
    if inserted.is_empty() {
        return Ok(vec![]);
    }
    let active = incidents.active_incidents(500)?;
    let mut active_count = active.len();
    let merge_window_seconds = config
        .get("incident_lifecycle")
        .and_then(|value| value.get("merge_time_threshold_seconds"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(300);
    let stale_timeout_seconds = config
        .get("incident_lifecycle")
        .and_then(|value| value.get("stale_timeout_seconds"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(900);
    let cluster_min_events = config
        .get("correlation")
        .and_then(|value| value.get("cluster_min_events"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(2)
        .max(2) as usize;
    let analysis_window_seconds = config
        .get("correlation")
        .and_then(|value| value.get("analysis_window_seconds"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(60);
    let max_events_per_incident = config
        .get("incident_lifecycle")
        .and_then(|value| value.get("limits"))
        .and_then(|value| value.get("max_events_per_incident"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(10_000)
        .clamp(10, 100_000) as usize;
    let max_active_incidents = config
        .get("incident_lifecycle")
        .and_then(|value| value.get("limits"))
        .and_then(|value| value.get("max_active_incidents"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(200)
        .clamp(1, 10_000) as usize;
    let enable_auto_split = config
        .get("incident_lifecycle")
        .and_then(|value| value.get("enable_auto_split"))
        .and_then(TomlValue::as_bool)
        .unwrap_or(true);
    let mut services = inserted
        .iter()
        .filter_map(|event| event.service_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if services.is_empty() {
        services.insert("runtime".to_string());
    }
    let mut touched = Vec::new();
    let mut processed_domains = std::collections::BTreeSet::new();
    for service_id in services {
        let domain_services = related_services(config, &service_id);
        let domain_key = domain_services.join("|");
        if !processed_domains.insert(domain_key) {
            continue;
        }
        let newest_inserted_at = inserted
            .iter()
            .filter(|event| {
                event
                    .service_id
                    .as_deref()
                    .map(|candidate| domain_services.iter().any(|service| service == candidate))
                    .unwrap_or(false)
            })
            .filter_map(|event| event.timestamp.clone())
            .max();
        let recent = recent_domain_events(
            &events,
            &domain_services,
            newest_inserted_at.as_deref(),
            stale_timeout_seconds
                .max(merge_window_seconds * 2)
                .max(analysis_window_seconds),
            max_events_per_incident.clamp(60, 250),
        )?;
        if recent.is_empty() {
            continue;
        }
        let incident_events =
            trim_incident_events(&recent, max_events_per_incident, enable_auto_split);
        let domain_metrics = analyze_domain_events(config, &incident_events);
        let max_severity = incident_events
            .iter()
            .filter_map(event_severity)
            .max()
            .unwrap_or(SEVERITY_INFO);
        let warn_or_higher = incident_events
            .iter()
            .filter(|event| event_severity(event).unwrap_or(SEVERITY_INFO) >= SEVERITY_WARN)
            .count();
        let affected_services = services_in_events(&incident_events);
        let source_types = distinct_source_types(&incident_events);
        let should_open = max_severity >= SEVERITY_ERROR
            || warn_or_higher >= cluster_min_events
            || (warn_or_higher >= 1 && affected_services.len() > 1)
            || (warn_or_higher >= 1
                && source_types.len() > 1
                && incident_events.len() >= cluster_min_events)
            || domain_metrics.anomaly_status == "anomalous";
        let existing = active
            .iter()
            .find(|incident| incident_matches_domain(incident, &domain_services));
        if should_open {
            if existing.is_none() && active_count >= max_active_incidents {
                continue;
            }
            let primary_service = dominant_service(&service_id, &incident_events);
            let incident_id = existing
                .map(|incident| incident.incident_id.clone())
                .unwrap_or_else(|| format!("inc-{}-{}", slug(&primary_service), unix_seconds()));
            let updated_at = incident_events
                .iter()
                .filter_map(|event| event.timestamp.clone())
                .max()
                .unwrap_or_else(now_iso);
            let created_at = existing
                .and_then(|incident| incident.created_at.clone())
                .unwrap_or_else(|| {
                    incident_events
                        .iter()
                        .filter_map(|event| event.timestamp.clone())
                        .min()
                        .unwrap_or_else(now_iso)
                });
            let first_seen = incident_events
                .iter()
                .filter_map(|event| event.timestamp.clone())
                .min()
                .unwrap_or_else(|| created_at.clone());
            let incident_event_ids = incident_events
                .iter()
                .filter_map(|event| event.event_id.clone())
                .collect::<Vec<_>>();
            let cluster_payloads = build_clusters(config, &primary_service, &incident_events);
            let cluster_ids = cluster_payloads
                .iter()
                .filter_map(|cluster| {
                    cluster
                        .get("cluster_id")
                        .and_then(serde_json::Value::as_str)
                })
                .map(str::to_string)
                .collect::<Vec<_>>();
            let incident = IncidentRecord {
                incident_id: incident_id.clone(),
                state: "open".into(),
                severity: max_severity,
                primary_service: primary_service.clone(),
                affected_services: affected_services.clone(),
                created_at: created_at.clone(),
                updated_at: updated_at.clone(),
                time_range_start: first_seen,
                time_range_end: updated_at.clone(),
                event_count: incident_event_ids.len() as i64,
                cluster_ids: cluster_ids.clone(),
                runtime_context: Some(serde_json::json!({
                    "service_id": primary_service,
                    "domain_services": domain_services,
                    "recent_events": incident_event_ids.len(),
                    "warn_or_higher": warn_or_higher,
                    "max_severity": max_severity,
                    "source_types": source_types,
                    "service_count": affected_services.len(),
                    "cluster_count": cluster_ids.len(),
                    "top_messages": top_messages(&incident_events),
                    "anomaly_score": domain_metrics.anomaly_score,
                    "anomaly_status": domain_metrics.anomaly_status,
                    "temporal_alignment": domain_metrics.temporal_alignment,
                    "auto_split_applied": incident_events.len() != recent.len(),
                    "max_events_per_incident": max_events_per_incident,
                })),
                resolution_info: None,
            };
            incidents.upsert_incident(&incident, &incident_event_ids)?;
            if existing.is_none() {
                active_count += 1;
                incidents.record_state_log(
                    &incident_id,
                    "new",
                    "open",
                    "native incident opened from incoming evidence",
                    Some(&updated_at),
                )?;
            }
            for cluster in cluster_payloads {
                if let Some(cluster_id) = cluster
                    .get("cluster_id")
                    .and_then(serde_json::Value::as_str)
                {
                    incidents.upsert_cluster(&incident_id, cluster_id, &cluster)?;
                }
            }
            let inference_graph =
                build_inference_graph_with_learning(config, &incident_events, &learning);
            incidents.upsert_inference_graph_snapshot(&StoredInferenceGraphSnapshot {
                incident_id: incident_id.clone(),
                graph_data: serde_json::to_value(&inference_graph)
                    .unwrap_or_else(|_| serde_json::json!({})),
                created_at: updated_at.clone(),
                event_count: incident_event_ids.len() as i64,
            })?;
            let previous_hypotheses = incidents
                .hypothesis_records(&incident_id)
                .unwrap_or_default();
            let hypotheses = build_hypotheses(
                config,
                &incident_id,
                &incident_events,
                &updated_at,
                &inference_graph,
                &learning,
            );
            incidents.replace_hypotheses(&incident_id, &hypotheses)?;
            record_adaptive_learning_history(
                &mut incidents,
                &incident_id,
                &updated_at,
                &previous_hypotheses,
                &hypotheses,
            )?;
            touched.push(incident_id);
        } else if let Some(existing) = existing {
            let resolved_at = now_iso();
            incidents.resolve_incident(
                &existing.incident_id,
                &serde_json::json!({
                    "resolved_by": "native_runtime",
                    "reason": "recent signal no longer exceeds warn threshold",
                    "service_id": service_id,
                    "affected_services": affected_services,
                }),
                &resolved_at,
            )?;
            active_count = active_count.saturating_sub(1);
            touched.push(existing.incident_id.clone());
        }
    }
    Ok(touched)
}

pub fn refresh_incident_reasoning(
    config: &TomlValue,
    paths: &Paths,
    incident_id: &str,
) -> Result<bool> {
    let Some(events) = EventsStore::open(&paths.events_db)? else {
        return Ok(false);
    };
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(false);
    };
    let learning = sync_learning_artifacts(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    refresh_single_incident_reasoning(
        config,
        &paths.incidents_db,
        incident_id,
        &events,
        &mut incidents,
        &learning,
    )
}

pub fn adaptive_learning_summary(config: &TomlValue, paths: &Paths) -> Result<serde_json::Value> {
    let adaptive_storage = adaptive_learning_storage_ref(&paths.incidents_db);
    let audit_storage = adaptive_learning_audit_storage_ref(&paths.incidents_db);
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(adaptive_learning_summary_payload(
            &AdaptiveLearningModel::default(),
            &adaptive_storage,
            &audit_storage,
            &[],
        ));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let audit = read_adaptive_learning_audit(&incidents, 25)?;
    let learning = sync_learning_artifacts(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    Ok(adaptive_learning_summary_payload(
        &learning.adaptive,
        &adaptive_storage,
        &audit_storage,
        &audit,
    ))
}

pub fn adaptive_learning_audit_log(
    config: &TomlValue,
    paths: &Paths,
    query: &AdaptiveLearningAuditQuery,
) -> Result<serde_json::Value> {
    let storage = adaptive_learning_audit_storage_ref(&paths.incidents_db);
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(serde_json::json!({
            "path": storage,
            "entries": [],
            "count": 0,
            "query": {
                "artifact_kind": query.artifact_kind.clone(),
                "artifact_id": query.artifact_id.clone(),
                "action": query.action.clone(),
                "review_status_after": query.review_status_after.clone(),
                "limit": query.limit,
                "offset": query.offset,
            },
        }));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let entries = incidents
        .list_adaptive_learning_audit(query)?
        .into_iter()
        .map(|entry| AdaptiveLearningAuditEntry {
            audit_id: entry.audit_id,
            artifact_kind: entry.artifact_kind,
            artifact_id: entry.artifact_id,
            action: entry.action,
            reason: entry.reason,
            previous_status: entry.previous_status,
            new_status: entry.new_status,
            review_status_before: entry.review_status_before,
            review_status_after: entry.review_status_after,
            runtime_effect: entry.runtime_effect,
            created_at: entry.created_at,
        })
        .collect::<Vec<_>>();
    let mut entries = entries;
    entries.reverse();
    Ok(serde_json::json!({
        "path": storage,
        "entries": entries.iter().map(adaptive_learning_audit_entry_json).collect::<Vec<_>>(),
        "count": entries.len(),
        "query": {
            "artifact_kind": query.artifact_kind.clone(),
            "artifact_id": query.artifact_id.clone(),
            "action": query.action.clone(),
            "review_status_after": query.review_status_after.clone(),
            "limit": query.limit,
            "offset": query.offset,
        },
    }))
}

pub fn adaptive_learning_review_summary(
    config: &TomlValue,
    paths: &Paths,
) -> Result<serde_json::Value> {
    let adaptive_storage = adaptive_learning_storage_ref(&paths.incidents_db);
    let audit_storage = adaptive_learning_audit_storage_ref(&paths.incidents_db);
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(serde_json::json!({
            "summary": adaptive_learning_summary_payload(
                &AdaptiveLearningModel::default(),
                &adaptive_storage,
                &audit_storage,
                &[],
            ),
            "active_incident_influence": [],
            "artifacts_requiring_attention": [],
            "review_counts": {},
            "review_queue": [],
            "recent_review_activity": [],
            "history_summary": adaptive_learning_history_summary(config, paths, 500)?,
            "comparison_rows": [],
            "saved_views": [],
            "trend_drilldowns": [],
            "analytics": {
                "kind_breakdown": [],
                "top_confirmed": [],
                "top_noisy": [],
                "top_impact": [],
                "recently_changed": [],
            },
        }));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let audit = read_adaptive_learning_audit(&incidents, 50)?;
    let learning = sync_learning_artifacts(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    let history_summary = adaptive_learning_history_summary(config, paths, 500)?;
    let trend_drilldowns = adaptive_learning_trend_drilldowns(&incidents, 2000, 8)?;
    let active_incident_influence = incidents
        .active_incidents(100)?
        .into_iter()
        .map(|incident| {
            let hypotheses = incidents
                .hypothesis_records(&incident.incident_id)
                .unwrap_or_default();
            let influenced = summarize_incident_learning_influence(&incident, &hypotheses);
            serde_json::json!({
                "incident_id": incident.incident_id,
                "state": incident.state,
                "primary_service": incident.primary_service,
                "severity": incident.severity,
                "learning": influenced,
            })
        })
        .filter(|item| {
            item.get("learning")
                .and_then(|value| value.get("influenced_hypotheses"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                > 0
        })
        .collect::<Vec<_>>();
    let comparison_rows = adaptive_review_comparison_rows(
        &learning.adaptive,
        &history_summary,
        &active_incident_influence,
    );
    let saved_views = adaptive_review_saved_views(&incidents, &comparison_rows);
    Ok(serde_json::json!({
        "summary": adaptive_learning_summary_payload(
            &learning.adaptive,
            &adaptive_storage,
            &audit_storage,
            &audit,
        ),
        "active_incident_influence": active_incident_influence,
        "artifacts_requiring_attention": artifacts_requiring_attention(&learning.adaptive),
        "review_counts": adaptive_review_counts(&learning.adaptive),
        "review_queue": adaptive_review_queue(&learning.adaptive),
        "recent_review_activity": recent_review_activity(&audit, 25),
        "history_summary": history_summary,
        "comparison_rows": comparison_rows,
        "saved_views": saved_views,
        "trend_drilldowns": trend_drilldowns,
        "analytics": adaptive_review_analytics(&comparison_rows),
    }))
}

pub fn adaptive_learning_history(
    config: &TomlValue,
    paths: &Paths,
    query: &AdaptiveLearningHistoryQuery,
) -> Result<serde_json::Value> {
    let storage = adaptive_learning_history_storage_ref(&paths.incidents_db);
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(serde_json::json!({
            "path": storage,
            "entries": [],
            "count": 0,
            "query": {
                "artifact_kind": query.artifact_kind.clone(),
                "artifact_id": query.artifact_id.clone(),
                "incident_id": query.incident_id.clone(),
                "cause_type": query.cause_type.clone(),
                "limit": query.limit,
                "offset": query.offset,
            },
        }));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let entries = incidents
        .list_adaptive_learning_history(query)?
        .into_iter()
        .map(|entry| AdaptiveLearningHistoryEntry {
            entry_id: entry.entry_id,
            artifact_kind: entry.artifact_kind,
            artifact_id: entry.artifact_id,
            artifact_label: entry.artifact_label,
            incident_id: entry.incident_id,
            cause_type: entry.cause_type,
            hypothesis_id: entry.hypothesis_id,
            observed_at: entry.observed_at,
            score: entry.score,
            rank: entry.rank,
            estimated_impact: entry.estimated_impact,
            impact_metric: entry.impact_metric,
            score_delta: entry.score_delta,
            rank_delta: entry.rank_delta,
            edge_delta: entry.edge_delta,
        })
        .collect::<Vec<_>>();
    let mut entries = entries;
    entries.reverse();
    Ok(serde_json::json!({
        "path": storage,
        "entries": entries.iter().map(adaptive_learning_history_entry_json).collect::<Vec<_>>(),
        "count": entries.len(),
        "query": {
            "artifact_kind": query.artifact_kind.clone(),
            "artifact_id": query.artifact_id.clone(),
            "incident_id": query.incident_id.clone(),
            "cause_type": query.cause_type.clone(),
            "limit": query.limit,
            "offset": query.offset,
        },
    }))
}

pub fn adaptive_learning_history_summary(
    config: &TomlValue,
    paths: &Paths,
    limit: usize,
) -> Result<serde_json::Value> {
    let storage = adaptive_learning_history_storage_ref(&paths.incidents_db);
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(serde_json::json!({
            "path": storage,
            "artifacts": [],
            "count": 0,
        }));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let entries = read_adaptive_learning_history(&incidents, limit)?;
    let mut by_artifact =
        std::collections::BTreeMap::<(String, String), Vec<AdaptiveLearningHistoryEntry>>::new();
    for entry in entries {
        by_artifact
            .entry((entry.artifact_kind.clone(), entry.artifact_id.clone()))
            .or_default()
            .push(entry);
    }
    let mut artifacts = by_artifact
        .into_iter()
        .map(|((kind, artifact_id), entries)| {
            let latest = entries.last().cloned();
            let cumulative_score_delta = entries
                .iter()
                .filter_map(|entry| entry.score_delta)
                .sum::<f64>();
            let cumulative_edge_delta = entries
                .iter()
                .filter_map(|entry| entry.edge_delta)
                .sum::<f64>();
            let best_rank = entries.iter().filter_map(|entry| entry.rank).min();
            serde_json::json!({
                "artifact_kind": kind,
                "artifact_id": artifact_id,
                "artifact_label": latest.as_ref().map(|entry| entry.artifact_label.clone()),
                "observations": entries.len(),
                "latest_observed_at": latest.as_ref().map(|entry| entry.observed_at.clone()),
                "latest_score": latest.as_ref().and_then(|entry| entry.score),
                "latest_rank": latest.as_ref().and_then(|entry| entry.rank),
                "best_rank": best_rank,
                "cumulative_score_delta": cumulative_score_delta,
                "cumulative_edge_delta": cumulative_edge_delta,
                "latest_estimated_impact": latest.as_ref().map(|entry| entry.estimated_impact).unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    artifacts.sort_by(|left, right| {
        let left_value = left
            .get("cumulative_score_delta")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default()
            .abs()
            + left
                .get("cumulative_edge_delta")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default()
                .abs();
        let right_value = right
            .get("cumulative_score_delta")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default()
            .abs()
            + right
                .get("cumulative_edge_delta")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default()
                .abs();
        right_value
            .partial_cmp(&left_value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let count = artifacts.len();
    Ok(serde_json::json!({
        "path": storage,
        "artifacts": artifacts,
        "count": count,
    }))
}

#[derive(Debug, Clone)]
pub struct AdaptiveArtifactSelection {
    pub artifact_kind: String,
    pub artifact_id: String,
}

#[derive(Debug, Clone)]
pub struct AdaptiveSavedReviewViewDraft {
    pub view_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub search_text: Option<String>,
    pub assigned_reviewer: Option<String>,
    pub artifact_selections: Vec<AdaptiveArtifactSelection>,
}

#[derive(Debug, Clone)]
struct AdaptiveArtifactMutationRecord {
    artifact_kind: String,
    artifact_id: String,
    label: String,
    previous_status: String,
    new_status: String,
    review_status_before: Option<String>,
    review_status_after: Option<String>,
    runtime_effect: Option<String>,
    updated_at: String,
    refresh_reasoning: bool,
}

fn adaptive_artifact_mutation_record_json(
    record: &AdaptiveArtifactMutationRecord,
) -> serde_json::Value {
    serde_json::json!({
        "artifact_kind": record.artifact_kind,
        "artifact_id": record.artifact_id,
        "label": record.label,
        "previous_status": record.previous_status,
        "new_status": record.new_status,
        "review_status_before": record.review_status_before,
        "review_status_after": record.review_status_after,
        "runtime_effect": record.runtime_effect,
        "updated_at": record.updated_at,
    })
}

pub fn adaptive_learning_set_artifact_state(
    config: &TomlValue,
    paths: &Paths,
    artifact_kind: &str,
    artifact_id: &str,
    action: &str,
    reason: Option<&str>,
) -> Result<serde_json::Value> {
    let adaptive_storage = adaptive_learning_storage_ref(&paths.incidents_db);
    let audit_storage = adaptive_learning_audit_storage_ref(&paths.incidents_db);
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Err(anyhow::anyhow!("incident store not found"));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let learning = sync_learning_artifacts(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    let mut adaptive = learning.adaptive;
    let normalized_kind = artifact_kind.trim().to_ascii_lowercase();
    let normalized_action = action.trim().to_ascii_lowercase();
    let disable = match normalized_action.as_str() {
        "disable" | "retire" => true,
        "enable" | "restore" => false,
        other => {
            return Err(anyhow::anyhow!(
                "unsupported action '{other}', expected disable/retire or enable/restore"
            ))
        }
    };
    let status_reason = reason
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let updated_at = now_iso();
    let Some(mutation) = apply_runtime_action_to_selected_artifact(
        &mut adaptive,
        &normalized_kind,
        artifact_id,
        disable,
        &status_reason,
        &updated_at,
    )?
    else {
        return Err(anyhow::anyhow!(
            "adaptive learning artifact '{artifact_id}' not found in kind '{artifact_kind}'"
        ));
    };
    adaptive.last_updated = Some(updated_at);
    incidents.replace_adaptive_learning_model(&stored_adaptive_learning_model(&adaptive))?;
    let audit_entry = AdaptiveLearningAuditEntry {
        audit_id: format!(
            "audit-{}-{}",
            artifact_id,
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ),
        artifact_kind: normalized_kind,
        artifact_id: artifact_id.to_string(),
        action: normalized_action,
        reason: status_reason,
        previous_status: mutation.previous_status,
        new_status: mutation.new_status,
        review_status_before: None,
        review_status_after: None,
        runtime_effect: mutation.runtime_effect,
        created_at: adaptive.last_updated.clone().unwrap_or_else(now_iso),
    };
    append_adaptive_learning_audit(&incidents, &audit_entry)?;
    refresh_active_incident_reasoning(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    let audit = read_adaptive_learning_audit(&incidents, 25)?;
    Ok(adaptive_learning_summary_payload(
        &adaptive,
        &adaptive_storage,
        &audit_storage,
        &audit,
    ))
}

pub fn adaptive_learning_review_artifact(
    config: &TomlValue,
    paths: &Paths,
    artifact_kind: &str,
    artifact_id: &str,
    decision: &str,
    reason: Option<&str>,
) -> Result<serde_json::Value> {
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Err(anyhow::anyhow!("incident store not found"));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let learning = sync_learning_artifacts(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    let mut adaptive = learning.adaptive;
    let normalized_kind = artifact_kind.trim().to_ascii_lowercase();
    let normalized_decision = decision.trim().to_ascii_lowercase();
    let review_reason = reason
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let reviewed_at = now_iso();
    let Some(mutation) = apply_review_decision_to_selected_artifact(
        &mut adaptive,
        &normalized_kind,
        artifact_id,
        &normalized_decision,
        &review_reason,
        &reviewed_at,
    )?
    else {
        return Err(anyhow::anyhow!(
            "adaptive learning artifact '{artifact_id}' not found in kind '{artifact_kind}'"
        ));
    };
    adaptive.last_updated = Some(reviewed_at.clone());
    incidents.replace_adaptive_learning_model(&stored_adaptive_learning_model(&adaptive))?;
    let audit_entry = AdaptiveLearningAuditEntry {
        audit_id: format!(
            "audit-review-{}-{}",
            artifact_id,
            time::OffsetDateTime::now_utc().unix_timestamp_nanos()
        ),
        artifact_kind: normalized_kind,
        artifact_id: artifact_id.to_string(),
        action: format!("review:{normalized_decision}"),
        reason: review_reason,
        previous_status: mutation.previous_status,
        new_status: mutation.new_status,
        review_status_before: mutation.review_status_before,
        review_status_after: mutation.review_status_after,
        runtime_effect: mutation.runtime_effect,
        created_at: reviewed_at,
    };
    append_adaptive_learning_audit(&incidents, &audit_entry)?;
    if mutation.refresh_reasoning {
        refresh_active_incident_reasoning(
            config,
            &paths.events_db,
            &paths.incidents_db,
            &mut incidents,
        )?;
    }
    adaptive_learning_review_summary(config, paths)
}

pub fn adaptive_learning_bulk_review_artifacts(
    config: &TomlValue,
    paths: &Paths,
    artifacts: &[AdaptiveArtifactSelection],
    decision: &str,
    reason: Option<&str>,
) -> Result<serde_json::Value> {
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Err(anyhow::anyhow!("incident store not found"));
    };
    if artifacts.is_empty() {
        return Err(anyhow::anyhow!(
            "no adaptive learning artifacts were selected"
        ));
    }
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let learning = sync_learning_artifacts(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    let mut adaptive = learning.adaptive;
    let normalized_decision = decision.trim().to_ascii_lowercase();
    let review_reason = reason
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let reviewed_at = now_iso();
    let mut seen = std::collections::BTreeSet::<(String, String)>::new();
    let mut updates = Vec::new();
    let mut refresh_reasoning = false;
    for selection in artifacts {
        let normalized_kind =
            normalize_adaptive_artifact_kind(&selection.artifact_kind)?.to_string();
        let dedup_key = (normalized_kind.clone(), selection.artifact_id.clone());
        if !seen.insert(dedup_key) {
            continue;
        }
        let Some(update) = apply_review_decision_to_selected_artifact(
            &mut adaptive,
            &normalized_kind,
            &selection.artifact_id,
            &normalized_decision,
            &review_reason,
            &reviewed_at,
        )?
        else {
            return Err(anyhow::anyhow!(
                "adaptive learning artifact '{}' not found in kind '{}'",
                selection.artifact_id,
                selection.artifact_kind
            ));
        };
        refresh_reasoning |= update.refresh_reasoning;
        updates.push(update);
    }
    if updates.is_empty() {
        return Err(anyhow::anyhow!(
            "no adaptive learning artifacts were selected"
        ));
    }
    adaptive.last_updated = Some(reviewed_at.clone());
    incidents.replace_adaptive_learning_model(&stored_adaptive_learning_model(&adaptive))?;
    for (index, update) in updates.iter().enumerate() {
        append_adaptive_learning_audit(
            &incidents,
            &AdaptiveLearningAuditEntry {
                audit_id: format!(
                    "audit-review-bulk-{}-{}-{}",
                    update.artifact_id,
                    time::OffsetDateTime::now_utc().unix_timestamp_nanos(),
                    index
                ),
                artifact_kind: update.artifact_kind.clone(),
                artifact_id: update.artifact_id.clone(),
                action: format!("review:{normalized_decision}"),
                reason: review_reason.clone(),
                previous_status: update.previous_status.clone(),
                new_status: update.new_status.clone(),
                review_status_before: update.review_status_before.clone(),
                review_status_after: update.review_status_after.clone(),
                runtime_effect: update.runtime_effect.clone(),
                created_at: reviewed_at.clone(),
            },
        )?;
    }
    if refresh_reasoning {
        refresh_active_incident_reasoning(
            config,
            &paths.events_db,
            &paths.incidents_db,
            &mut incidents,
        )?;
    }
    Ok(serde_json::json!({
        "updated": true,
        "updated_count": updates.len(),
        "decision": normalized_decision,
        "artifacts": updates.iter().map(adaptive_artifact_mutation_record_json).collect::<Vec<_>>(),
        "review": adaptive_learning_review_summary(config, paths)?,
    }))
}

pub fn adaptive_learning_bulk_set_artifact_state(
    config: &TomlValue,
    paths: &Paths,
    artifacts: &[AdaptiveArtifactSelection],
    action: &str,
    reason: Option<&str>,
) -> Result<serde_json::Value> {
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Err(anyhow::anyhow!("incident store not found"));
    };
    if artifacts.is_empty() {
        return Err(anyhow::anyhow!(
            "no adaptive learning artifacts were selected"
        ));
    }
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let learning = sync_learning_artifacts(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    let mut adaptive = learning.adaptive;
    let normalized_action = action.trim().to_ascii_lowercase();
    let disable = match normalized_action.as_str() {
        "disable" | "retire" => true,
        "enable" | "restore" => false,
        other => {
            return Err(anyhow::anyhow!(
                "unsupported action '{other}', expected disable/retire or enable/restore"
            ))
        }
    };
    let status_reason = reason
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let updated_at = now_iso();
    let mut seen = std::collections::BTreeSet::<(String, String)>::new();
    let mut updates = Vec::new();
    for selection in artifacts {
        let normalized_kind =
            normalize_adaptive_artifact_kind(&selection.artifact_kind)?.to_string();
        let dedup_key = (normalized_kind.clone(), selection.artifact_id.clone());
        if !seen.insert(dedup_key) {
            continue;
        }
        let Some(update) = apply_runtime_action_to_selected_artifact(
            &mut adaptive,
            &normalized_kind,
            &selection.artifact_id,
            disable,
            &status_reason,
            &updated_at,
        )?
        else {
            return Err(anyhow::anyhow!(
                "adaptive learning artifact '{}' not found in kind '{}'",
                selection.artifact_id,
                selection.artifact_kind
            ));
        };
        updates.push(update);
    }
    if updates.is_empty() {
        return Err(anyhow::anyhow!(
            "no adaptive learning artifacts were selected"
        ));
    }
    adaptive.last_updated = Some(updated_at.clone());
    incidents.replace_adaptive_learning_model(&stored_adaptive_learning_model(&adaptive))?;
    for (index, update) in updates.iter().enumerate() {
        append_adaptive_learning_audit(
            &incidents,
            &AdaptiveLearningAuditEntry {
                audit_id: format!(
                    "audit-bulk-{}-{}-{}",
                    update.artifact_id,
                    time::OffsetDateTime::now_utc().unix_timestamp_nanos(),
                    index
                ),
                artifact_kind: update.artifact_kind.clone(),
                artifact_id: update.artifact_id.clone(),
                action: normalized_action.clone(),
                reason: status_reason.clone(),
                previous_status: update.previous_status.clone(),
                new_status: update.new_status.clone(),
                review_status_before: None,
                review_status_after: None,
                runtime_effect: update.runtime_effect.clone(),
                created_at: updated_at.clone(),
            },
        )?;
    }
    refresh_active_incident_reasoning(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    Ok(serde_json::json!({
        "updated": true,
        "updated_count": updates.len(),
        "action": normalized_action,
        "artifacts": updates.iter().map(adaptive_artifact_mutation_record_json).collect::<Vec<_>>(),
        "review": adaptive_learning_review_summary(config, paths)?,
    }))
}

pub fn adaptive_learning_save_review_view(
    config: &TomlValue,
    paths: &Paths,
    draft: &AdaptiveSavedReviewViewDraft,
) -> Result<serde_json::Value> {
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Err(anyhow::anyhow!("incident store not found"));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let name = draft.name.trim();
    if name.is_empty() {
        return Err(anyhow::anyhow!("saved review view name must not be empty"));
    }
    let existing_views = incidents.list_adaptive_review_views()?;
    let now = now_iso();
    let existing = draft
        .view_id
        .as_ref()
        .and_then(|view_id| existing_views.iter().find(|view| &view.view_id == view_id));
    let view_id = draft
        .view_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "review-view-{}",
                time::OffsetDateTime::now_utc().unix_timestamp_nanos()
            )
        });
    let mut seen = std::collections::BTreeSet::<(String, String)>::new();
    let artifact_selections = draft
        .artifact_selections
        .iter()
        .filter_map(|selection| {
            let kind = normalize_adaptive_artifact_kind(&selection.artifact_kind)
                .ok()?
                .to_string();
            let artifact_id = selection.artifact_id.trim();
            if artifact_id.is_empty() {
                return None;
            }
            let key = (kind.clone(), artifact_id.to_string());
            if !seen.insert(key.clone()) {
                return None;
            }
            Some(StoredAdaptiveReviewViewSelection {
                artifact_kind: key.0,
                artifact_id: key.1,
            })
        })
        .collect::<Vec<_>>();
    incidents.upsert_adaptive_review_view(&StoredAdaptiveReviewView {
        view_id: view_id.clone(),
        name: name.to_string(),
        description: draft
            .description
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        search_text: draft
            .search_text
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        assigned_reviewer: draft
            .assigned_reviewer
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        artifact_selections,
        created_at: existing
            .map(|view| view.created_at.clone())
            .unwrap_or_else(|| now.clone()),
        updated_at: now.clone(),
        last_used_at: existing.and_then(|view| view.last_used_at.clone()),
    })?;
    Ok(serde_json::json!({
        "updated": true,
        "view_id": view_id,
        "review": adaptive_learning_review_summary(config, paths)?,
    }))
}

pub fn adaptive_learning_delete_review_view(
    config: &TomlValue,
    paths: &Paths,
    view_id: &str,
) -> Result<serde_json::Value> {
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Err(anyhow::anyhow!("incident store not found"));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    incidents.delete_adaptive_review_view(view_id)?;
    Ok(serde_json::json!({
        "deleted": true,
        "view_id": view_id,
        "review": adaptive_learning_review_summary(config, paths)?,
    }))
}

pub fn adaptive_learning_touch_review_view(
    config: &TomlValue,
    paths: &Paths,
    view_id: &str,
) -> Result<serde_json::Value> {
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Err(anyhow::anyhow!("incident store not found"));
    };
    ensure_adaptive_learning_storage_imported(config, &paths.incidents_db, &mut incidents)?;
    let used_at = now_iso();
    incidents.touch_adaptive_review_view(view_id, &used_at)?;
    Ok(serde_json::json!({
        "updated": true,
        "view_id": view_id,
        "used_at": used_at,
        "review": adaptive_learning_review_summary(config, paths)?,
    }))
}

fn build_hypotheses(
    config: &TomlValue,
    incident_id: &str,
    events: &[EventRow],
    updated_at: &str,
    inference_graph: &InferenceGraph,
    learning: &LearningArtifacts,
) -> Vec<StoredHypothesis> {
    let all_services = services_in_events(events);
    let source_types = distinct_source_types(events);
    let max_severity_value = max_severity(events);
    let domain_metrics = analyze_domain_events(config, events);
    let min_supporting_events = config
        .get("hypothesis_engine")
        .and_then(|value| value.get("min_supporting_events"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(1)
        .max(1) as usize;
    let min_generation_confidence = config
        .get("hypothesis_engine")
        .and_then(|value| value.get("min_generation_confidence"))
        .and_then(TomlValue::as_float)
        .or_else(|| {
            config
                .get("hypothesis_engine")
                .and_then(|value| value.get("min_generation_confidence"))
                .and_then(TomlValue::as_integer)
                .map(|value| value as f64)
        })
        .unwrap_or(0.1)
        .clamp(0.0, 1.0);
    let dedup_overlap_threshold = config
        .get("hypothesis_engine")
        .and_then(|value| value.get("dedup_overlap_threshold"))
        .and_then(TomlValue::as_float)
        .or_else(|| {
            config
                .get("hypothesis_engine")
                .and_then(|value| value.get("dedup_overlap_threshold"))
                .and_then(TomlValue::as_integer)
                .map(|value| value as f64)
        })
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let dependency_support = events
        .iter()
        .filter(|event| {
            let text = event_signal_text(event);
            has_any_term(
                &text,
                &[
                    "timeout",
                    "connection refused",
                    "dependency",
                    "database",
                    "postgres",
                    "redis",
                    "dns",
                    "upstream",
                    "unavailable",
                ],
            )
        })
        .filter_map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    let resource_support = events
        .iter()
        .filter(|event| {
            let text = event_signal_text(event);
            has_any_term(
                &text,
                &[
                    "cpu",
                    "memory",
                    "disk",
                    "oom",
                    "resource pressure",
                    "saturation",
                    "throttle",
                ],
            ) || matches!(
                event
                    .source_ref
                    .as_ref()
                    .and_then(|source| source.source_type.as_deref()),
                Some("host_metrics" | "process_snapshot")
            )
        })
        .filter_map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    let instability_support = events
        .iter()
        .filter(|event| {
            let text = event_signal_text(event);
            has_any_term(
                &text,
                &[
                    "restart", "crash", "stopped", "failed", "panic", "evicted", "degraded",
                ],
            )
        })
        .filter_map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    let orchestration_support = events
        .iter()
        .filter(|event| {
            let text = event_signal_text(event);
            has_any_term(
                &text,
                &[
                    "kubernetes",
                    "docker",
                    "pod",
                    "container",
                    "deployment",
                    "daemonset",
                ],
            )
        })
        .filter_map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    let mut candidates = Vec::<Candidate>::new();
    candidates.extend(graph_root_candidates(
        events,
        &all_services,
        &source_types,
        &domain_metrics,
        inference_graph,
    ));
    candidates.extend(custom_rule_candidates(
        config,
        events,
        &all_services,
        &source_types,
        &domain_metrics,
        inference_graph,
        learning,
    ));
    if !dependency_support.is_empty() {
        candidates.push(Candidate {
            cause_type: "dependency_failure".into(),
            description: if all_services.len() > 1 {
                format!(
                    "Evidence spans {} related services and points to an upstream dependency timing out, refusing connections, or becoming unavailable.",
                    all_services.len()
                )
            } else {
                "Evidence points to an upstream dependency timing out, refusing connections, or becoming unavailable.".into()
            },
            prior: 0.56,
            score: 0.0,
            suggested_checks: vec![
                "Check dependency connectivity and credentials".into(),
                "Inspect upstream health and latency around the incident window".into(),
            ],
            supporting_events: dependency_support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
            dependency_proximity: dependency_proximity_score("dependency_failure", events),
            is_valid: true,
            invalidation_reasons: Vec::new(),
            root_cause_event_id: graph_root_for_cause(inference_graph, "dependency_failure"),
            root_cause_timestamp: None,
            scoring: CandidateScoring::default(),
            provenance_refs: Vec::new(),
        });
    }
    if !resource_support.is_empty() {
        candidates.push(Candidate {
            cause_type: "resource_pressure".into(),
            description: "Host or process telemetry indicates resource pressure contributing to the incident.".into(),
            prior: 0.52,
            score: 0.0,
            suggested_checks: vec![
                "Inspect CPU, memory, and disk saturation near the first warning".into(),
                "Review process-level spikes or OOM conditions".into(),
            ],
            supporting_events: resource_support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
            dependency_proximity: dependency_proximity_score("resource_pressure", events),
            is_valid: true,
            invalidation_reasons: Vec::new(),
            root_cause_event_id: graph_root_for_cause(inference_graph, "resource_pressure"),
            root_cause_timestamp: None,
            scoring: CandidateScoring::default(),
            provenance_refs: Vec::new(),
        });
    }
    if !instability_support.is_empty() {
        candidates.push(Candidate {
            cause_type: "service_instability".into(),
            description: "Lifecycle signals show the service or a nearby runtime repeatedly failing, restarting, or stopping.".into(),
            prior: 0.49,
            score: 0.0,
            suggested_checks: vec![
                "Inspect restart loops, crash output, and supervisor state".into(),
                "Correlate deploy or restart timing with dependency and resource signals".into(),
            ],
            supporting_events: instability_support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
            dependency_proximity: dependency_proximity_score("service_instability", events),
            is_valid: true,
            invalidation_reasons: Vec::new(),
            root_cause_event_id: graph_root_for_cause(inference_graph, "service_instability"),
            root_cause_timestamp: None,
            scoring: CandidateScoring::default(),
            provenance_refs: Vec::new(),
        });
    }
    if !orchestration_support.is_empty() {
        candidates.push(Candidate {
            cause_type: "orchestration_change".into(),
            description: "Container or orchestration activity changed during the incident window and may have triggered downstream impact.".into(),
            prior: 0.46,
            score: 0.0,
            suggested_checks: vec![
                "Review recent pod/container scheduling, restart, and rollout events".into(),
                "Compare runtime changes with the first user-visible failure".into(),
            ],
            supporting_events: orchestration_support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
            dependency_proximity: dependency_proximity_score("orchestration_change", events),
            is_valid: true,
            invalidation_reasons: Vec::new(),
            root_cause_event_id: graph_root_for_cause(inference_graph, "orchestration_change"),
            root_cause_timestamp: None,
            scoring: CandidateScoring::default(),
            provenance_refs: Vec::new(),
        });
    }
    if all_services.len() > 1 && source_types.len() > 1 {
        let support = events
            .iter()
            .filter_map(|event| event.event_id.clone())
            .collect::<Vec<_>>();
        candidates.push(Candidate {
            cause_type: "shared_fate".into(),
            description: "Multiple related services degraded together, which suggests a shared dependency or infrastructure fault domain.".into(),
            prior: 0.44,
            score: 0.0,
            suggested_checks: vec![
                "Trace common dependencies across the affected services".into(),
                "Review topology edges and shared infrastructure during the incident window".into(),
            ],
            supporting_events: support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
            dependency_proximity: dependency_proximity_score("shared_fate", events),
            is_valid: true,
            invalidation_reasons: Vec::new(),
            root_cause_event_id: graph_root_for_cause(inference_graph, "shared_fate"),
            root_cause_timestamp: None,
            scoring: CandidateScoring::default(),
            provenance_refs: Vec::new(),
        });
    }
    if candidates.is_empty() {
        candidates.push(Candidate {
            cause_type: "unknown".into(),
            description: "Elevated signal was detected but did not match a stronger native hypothesis template.".into(),
            prior: 0.48,
            score: 0.0,
            suggested_checks: vec![
                "Inspect the grouped event timeline".into(),
                "Add topology and service mappings for stronger correlation".into(),
            ],
            supporting_events: events.iter().filter_map(|event| event.event_id.clone()).collect(),
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
            dependency_proximity: 0.2,
            is_valid: true,
            invalidation_reasons: Vec::new(),
            root_cause_event_id: inference_graph.root_candidates.first().cloned(),
            root_cause_timestamp: inference_graph
                .root_candidates
                .first()
                .and_then(|event_id| graph_node(inference_graph, event_id))
                .map(|node| node.timestamp.clone()),
            scoring: CandidateScoring::default(),
            provenance_refs: Vec::new(),
        });
    }
    for candidate in &mut candidates {
        candidate.supporting_events.sort();
        candidate.supporting_events.dedup();
        candidate.root_cause_timestamp = candidate
            .root_cause_event_id
            .as_ref()
            .and_then(|event_id| graph_node(inference_graph, event_id))
            .map(|node| node.timestamp.clone());
        candidate.contradicting_events =
            contradiction_events_for_candidate(config, events, &candidate.cause_type);
        let contradiction_ratio = candidate.contradicting_events.len() as f64
            / candidate.supporting_events.len().max(1) as f64;
        candidate.scoring =
            candidate_scoring_components(events, candidate, &domain_metrics, inference_graph);
        candidate.score = candidate_score(
            config,
            max_severity_value,
            candidate.prior,
            &candidate.scoring,
            contradiction_ratio,
            &learning.weights,
        );
        candidate.is_valid = true;
        if candidate.supporting_events.len() < min_supporting_events {
            candidate.is_valid = false;
            candidate.invalidation_reasons.push(format!(
                "supporting evidence below hypothesis_engine.min_supporting_events ({min_supporting_events})"
            ));
        }
        if candidate.score < min_generation_confidence {
            candidate.is_valid = false;
            candidate.invalidation_reasons.push(format!(
                "score {:.2} below hypothesis_engine.min_generation_confidence {:.2}",
                candidate.score, min_generation_confidence
            ));
        }
        if contradiction_ratio >= contradiction_fail_threshold(config) {
            candidate.is_valid = false;
            candidate
                .invalidation_reasons
                .push("contradiction ratio exceeded fail threshold".into());
        } else if contradiction_ratio >= contradiction_warn_threshold(config) {
            candidate
                .invalidation_reasons
                .push("contradiction ratio exceeded warning threshold".into());
        }
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| candidate_tiebreak_ordering(config, left, right))
    });
    let mut deduped = Vec::<Candidate>::new();
    for candidate in candidates {
        let duplicate = deduped.iter().any(|existing| {
            existing.cause_type == candidate.cause_type
                && candidate_overlap_ratio(existing, &candidate) >= dedup_overlap_threshold
        });
        if !duplicate {
            deduped.push(candidate);
        }
    }
    let max_hypotheses = config
        .get("hypothesis_engine")
        .and_then(|value| value.get("max_hypotheses_per_incident"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(4)
        .max(1) as usize;
    deduped
        .into_iter()
        .take(max_hypotheses)
        .enumerate()
        .map(|(index, candidate)| {
            let provenance = candidate_learning_provenance(&candidate, events, inference_graph, learning);
            StoredHypothesis {
            hypothesis_id: format!("{incident_id}-hyp-{}", index + 1),
            rank: Some((index + 1) as i64),
            cause_type: candidate.cause_type,
            description: candidate.description,
            total_score: Some(candidate.score),
            score_breakdown: serde_json::json!({
                "severity": max_severity_value,
                "supporting_events": candidate.supporting_events.len(),
                "related_services": candidate.affected_services.len(),
                "source_types": source_types.clone(),
                "heuristic_score": candidate.prior,
                "temporal_alignment": candidate.scoring.temporal_alignment,
                "correlation_strength": candidate.scoring.correlation_strength,
                "frequency_weight": candidate.scoring.frequency_weight,
                "dependency_proximity": candidate.scoring.dependency_proximity,
                "evidence_coverage": candidate.scoring.evidence_coverage,
                "anomaly_severity": candidate.scoring.anomaly_severity,
                "graph_impact": candidate.scoring.graph_impact,
                "root_cause_event_id": candidate.root_cause_event_id,
                "root_cause_timestamp": candidate.root_cause_timestamp,
                "calibration_bucket_accuracy": calibration_bucket_accuracy(&learning.calibration, candidate.score),
                "calibration_status": learning.calibration.staleness_status,
                "provenance": provenance,
            }),
            supporting_events: candidate.supporting_events,
            contradicting_events: candidate.contradicting_events,
            affected_services: candidate.affected_services,
            suggested_checks: candidate.suggested_checks,
            confidence_label: Some(confidence_label(config, candidate.score, &learning.calibration)),
            is_valid: candidate.is_valid,
            invalidation_reasons: candidate.invalidation_reasons,
            created_at: updated_at.to_string(),
            updated_at: updated_at.to_string(),
        }})
        .collect()
}

fn refresh_single_incident_reasoning(
    config: &TomlValue,
    _incidents_db: &Path,
    incident_id: &str,
    events: &EventsStore,
    incidents: &mut IncidentsStore,
    learning: &LearningArtifacts,
) -> Result<bool> {
    let event_ids = incidents.incident_event_ids(incident_id)?;
    if event_ids.is_empty() {
        return Ok(false);
    }
    let incident_events = events.get_events(&event_ids)?;
    if incident_events.is_empty() {
        return Ok(false);
    }
    let updated_at = incident_events
        .iter()
        .filter_map(|event| event.timestamp.clone())
        .max()
        .unwrap_or_else(now_iso);
    let inference_graph = build_inference_graph_with_learning(config, &incident_events, learning);
    incidents.upsert_inference_graph_snapshot(&StoredInferenceGraphSnapshot {
        incident_id: incident_id.to_string(),
        graph_data: serde_json::to_value(&inference_graph)
            .unwrap_or_else(|_| serde_json::json!({})),
        created_at: updated_at.clone(),
        event_count: incident_events.len() as i64,
    })?;
    let previous_hypotheses = incidents
        .hypothesis_records(incident_id)
        .unwrap_or_default();
    let hypotheses = build_hypotheses(
        config,
        incident_id,
        &incident_events,
        &updated_at,
        &inference_graph,
        learning,
    );
    incidents.replace_hypotheses(incident_id, &hypotheses)?;
    record_adaptive_learning_history(
        incidents,
        incident_id,
        &updated_at,
        &previous_hypotheses,
        &hypotheses,
    )?;
    Ok(true)
}

fn recent_domain_events(
    events: &EventsStore,
    services: &[String],
    reference_timestamp: Option<&str>,
    lookback_seconds: i64,
    per_service_limit: usize,
) -> Result<Vec<EventRow>> {
    let mut merged = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for service in services {
        for event in events.events_for_service(service, per_service_limit)? {
            let event_id = event.event_id.clone().unwrap_or_default();
            if !event_id.is_empty() && !seen.insert(event_id) {
                continue;
            }
            merged.push(event);
        }
    }
    if let Some(reference) = reference_timestamp.and_then(parse_rfc3339) {
        merged.retain(|event| {
            event
                .timestamp
                .as_deref()
                .and_then(parse_rfc3339)
                .map(|timestamp| {
                    (reference - timestamp).whole_seconds().abs() <= lookback_seconds.max(60)
                })
                .unwrap_or(true)
        });
    }
    merged.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    Ok(merged)
}

fn incident_matches_domain(incident: &IncidentRow, domain_services: &[String]) -> bool {
    let mut incident_services = std::collections::BTreeSet::new();
    if !incident.primary_service.is_empty() {
        incident_services.insert(incident.primary_service.clone());
    }
    for service in incident.affected_services.iter().flatten() {
        incident_services.insert(service.clone());
    }
    domain_services
        .iter()
        .any(|service| incident_services.contains(service))
}

fn services_in_events(events: &[EventRow]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event.service_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn dominant_service(anchor_service: &str, events: &[EventRow]) -> String {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for event in events {
        if let Some(service_id) = event.service_id.as_ref() {
            let weight = if event_severity(event).unwrap_or(SEVERITY_INFO) >= SEVERITY_WARN {
                2
            } else {
                1
            };
            *counts.entry(service_id.clone()).or_default() += weight;
        }
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)))
        .map(|item| item.0)
        .unwrap_or_else(|| anchor_service.to_string())
}

fn build_clusters(
    config: &TomlValue,
    primary_service: &str,
    events: &[EventRow],
) -> Vec<serde_json::Value> {
    let mut grouped = std::collections::BTreeMap::<String, Vec<EventRow>>::new();
    for event in events {
        let key = event
            .source_ref
            .as_ref()
            .and_then(|source| source.source_type.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "runtime".into());
        grouped.entry(key).or_default().push(event.clone());
    }
    if grouped.is_empty() {
        grouped.insert("runtime".into(), events.to_vec());
    }
    let max_clusters = config
        .get("incident_lifecycle")
        .and_then(|value| value.get("limits"))
        .and_then(|value| value.get("max_clusters_per_incident"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(4)
        .max(1) as usize;
    grouped
        .into_iter()
        .take(max_clusters)
        .map(|(source_type, grouped_events)| {
            let cluster_id = format!("cluster-{}-{}", slug(primary_service), slug(&source_type));
            serde_json::json!({
                "cluster_id": cluster_id,
                "primary_service": primary_service,
                "source_type": source_type,
                "affected_services": services_in_events(&grouped_events),
                "event_ids": grouped_events.iter().filter_map(|event| event.event_id.clone()).collect::<Vec<_>>(),
                "event_count": grouped_events.len(),
                "top_messages": top_messages(&grouped_events),
                "source_types": distinct_source_types(&grouped_events),
                "max_severity": max_severity(&grouped_events),
            })
        })
        .collect()
}

fn has_any_term(text: &str, terms: &[&str]) -> bool {
    terms.iter().any(|term| text.contains(term))
}

fn candidate_score(
    config: &TomlValue,
    severity: i64,
    prior: f64,
    scoring: &CandidateScoring,
    contradiction_ratio: f64,
    learned_weights: &LearnedScoringWeights,
) -> f64 {
    let severity_score = match severity {
        value if value >= 4 => 1.0,
        value if value >= SEVERITY_ERROR => 0.85,
        value if value >= SEVERITY_WARN => 0.6,
        _ => 0.35,
    };
    let weights = effective_scoring_weights(config, learned_weights);
    let weighted = weights.temporal_alignment * scoring.temporal_alignment
        + weights.correlation_strength * scoring.correlation_strength
        + weights.frequency_weight * scoring.frequency_weight
        + weights.dependency_proximity * scoring.dependency_proximity
        + weights.evidence_coverage * scoring.evidence_coverage
        + weights.anomaly_severity * scoring.anomaly_severity;
    let normalized = weighted / weights.total.max(0.0001);
    let contradiction_penalty = contradiction_penalty(config, contradiction_ratio);
    ((prior * 0.25) + (severity_score * 0.1) + (normalized * 0.45) + (scoring.graph_impact * 0.2)
        - contradiction_penalty)
        .clamp(0.0, 0.98)
}

fn confidence_label(config: &TomlValue, score: f64, calibration: &CalibrationModel) -> String {
    let bucket_accuracy = calibration_bucket_accuracy(calibration, score);
    if calibration_enabled(config) && calibration_bucket_is_sufficient(calibration, score) {
        let base = if bucket_accuracy >= 0.7 {
            "high"
        } else if bucket_accuracy >= 0.4 {
            "medium"
        } else {
            "low"
        };
        if calibration.staleness_status == "stale" {
            format!("{base} (calibration outdated)")
        } else {
            base.into()
        }
    } else {
        default_confidence_label(config, score)
    }
}

fn default_confidence_label(config: &TomlValue, score: f64) -> String {
    let (high_threshold, _medium_threshold) = calibration_thresholds(config);
    if score >= high_threshold {
        "medium".into()
    } else {
        "low".into()
    }
}

fn related_services(config: &TomlValue, primary_service: &str) -> Vec<String> {
    let mut services = std::collections::BTreeSet::from([primary_service.to_string()]);
    if let Some(edges) = config
        .get("topology")
        .and_then(|value| value.get("edges"))
        .and_then(TomlValue::as_array)
    {
        for edge in edges.iter().filter_map(TomlValue::as_table) {
            let source = edge
                .get("source")
                .and_then(TomlValue::as_str)
                .unwrap_or_default();
            let target = edge
                .get("target")
                .and_then(TomlValue::as_str)
                .unwrap_or_default();
            if source == primary_service && !target.is_empty() {
                services.insert(target.to_string());
            }
            if target == primary_service && !source.is_empty() {
                services.insert(source.to_string());
            }
        }
    }
    services.into_iter().collect()
}

fn event_severity(event: &EventRow) -> Option<i64> {
    event.severity.as_ref().and_then(|value| match value {
        SeverityValue::Level(level) => Some(*level),
        SeverityValue::Label(label) => severity_label_value(label),
    })
}

fn severity_label_value(raw: &str) -> Option<i64> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "trace" | "debug" => Some(0),
        "info" | "informational" => Some(SEVERITY_INFO),
        "warn" | "warning" => Some(SEVERITY_WARN),
        "error" => Some(SEVERITY_ERROR),
        "critical" | "fatal" | "panic" => Some(4),
        value => value.parse::<i64>().ok(),
    }
}

fn max_severity(events: &[EventRow]) -> i64 {
    events
        .iter()
        .filter_map(event_severity)
        .max()
        .unwrap_or(SEVERITY_INFO)
}

fn distinct_source_types(events: &[EventRow]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event.source_ref.as_ref())
        .filter_map(|source| source.source_type.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn top_messages(events: &[EventRow]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event.message.clone().or_else(|| event.summary.clone()))
        .take(5)
        .collect()
}

#[derive(Debug, Clone)]
struct DomainMetrics {
    event_count: usize,
    warn_count: usize,
    error_count: usize,
    anomaly_score: f64,
    anomaly_status: String,
    temporal_alignment: f64,
}

#[derive(Debug, Clone, Copy)]
struct ScoringWeights {
    temporal_alignment: f64,
    correlation_strength: f64,
    frequency_weight: f64,
    dependency_proximity: f64,
    evidence_coverage: f64,
    anomaly_severity: f64,
    total: f64,
}

impl Default for CalibrationModel {
    fn default() -> Self {
        Self {
            schema_version: 1,
            last_updated: None,
            total_feedback_count: 0,
            overall_accuracy: 0.0,
            staleness_status: "insufficient_data".into(),
            buckets: default_calibration_buckets(5),
            processed_feedback_ids: Vec::new(),
        }
    }
}

impl Default for LearnedScoringWeights {
    fn default() -> Self {
        let defaults = scoring_weight_defaults();
        Self {
            schema_version: 1,
            last_updated: None,
            default_weights: defaults.clone(),
            effective_weights: defaults,
            processed_feedback_ids: Vec::new(),
            audit: Vec::new(),
        }
    }
}

impl Default for AdaptiveLearningModel {
    fn default() -> Self {
        Self {
            schema_version: 1,
            last_updated: None,
            processed_feedback_ids: Vec::new(),
            learned_detectors: Vec::new(),
            learned_templates: Vec::new(),
            learned_compositions: Vec::new(),
            learned_edge_profiles: Vec::new(),
        }
    }
}

fn adaptive_learning_from_stored(
    stored: inferra_storage::StoredAdaptiveLearningModel,
) -> AdaptiveLearningModel {
    AdaptiveLearningModel {
        schema_version: stored.schema_version.max(1) as u64,
        last_updated: stored.last_updated,
        processed_feedback_ids: stored.processed_feedback_ids,
        learned_detectors: stored
            .learned_detectors
            .into_iter()
            .map(|item| LearnedDetector {
                detector_id: item.detector_id,
                requirement_name: item.requirement_name,
                cause_type: item.cause_type,
                positive_terms: item.positive_terms,
                tags: item.tags,
                source_types: item.source_types,
                min_severity: item.min_severity,
                confirmations: item.confirmations.max(0) as u64,
                false_positives: item.false_positives.max(0) as u64,
                created_from_feedback_id: item.created_from_feedback_id,
                updated_at: item.updated_at,
                manually_disabled: item.manually_disabled,
                status_reason: item.status_reason,
                review_status: item.review_status,
                review_reason: item.review_reason,
                last_reviewed_at: item.last_reviewed_at,
            })
            .collect(),
        learned_templates: stored
            .learned_templates
            .into_iter()
            .map(|item| LearnedTemplate {
                template_id: item.template_id,
                template_name: item.template_name,
                cause_type: item.cause_type,
                cause_subtype: item.cause_subtype,
                title_template: item.title_template,
                confidence: item.confidence,
                requires: item.requires,
                requires_same_service: item.requires_same_service,
                requires_temporal_order: item.requires_temporal_order,
                confirmations: item.confirmations.max(0) as u64,
                false_positives: item.false_positives.max(0) as u64,
                created_from_feedback_id: item.created_from_feedback_id,
                updated_at: item.updated_at,
                manually_disabled: item.manually_disabled,
                status_reason: item.status_reason,
                review_status: item.review_status,
                review_reason: item.review_reason,
                last_reviewed_at: item.last_reviewed_at,
            })
            .collect(),
        learned_compositions: stored
            .learned_compositions
            .into_iter()
            .map(|item| LearnedComposition {
                composition_id: item.composition_id,
                composition_name: item.composition_name,
                cause_type: item.cause_type,
                cause_subtype: item.cause_subtype,
                title_template: item.title_template,
                confidence: item.confidence,
                requires: item.requires,
                requires_same_service: item.requires_same_service,
                requires_temporal_order: item.requires_temporal_order,
                preferred_edge_types: item.preferred_edge_types,
                confirmations: item.confirmations.max(0) as u64,
                false_positives: item.false_positives.max(0) as u64,
                created_from_feedback_id: item.created_from_feedback_id,
                updated_at: item.updated_at,
                manually_disabled: item.manually_disabled,
                status_reason: item.status_reason,
                review_status: item.review_status,
                review_reason: item.review_reason,
                last_reviewed_at: item.last_reviewed_at,
            })
            .collect(),
        learned_edge_profiles: stored
            .learned_edge_profiles
            .into_iter()
            .map(|item| LearnedEdgeProfile {
                profile_id: item.profile_id,
                edge_type: item.edge_type,
                source_service: item.source_service,
                target_service: item.target_service,
                cause_type: item.cause_type,
                confirmations: item.confirmations.max(0) as u64,
                false_positives: item.false_positives.max(0) as u64,
                average_plausibility: item.average_plausibility,
                average_latency_ms: item.average_latency_ms,
                created_from_feedback_id: item.created_from_feedback_id,
                updated_at: item.updated_at,
                manually_disabled: item.manually_disabled,
                status_reason: item.status_reason,
                review_status: item.review_status,
                review_reason: item.review_reason,
                last_reviewed_at: item.last_reviewed_at,
            })
            .collect(),
    }
}

fn stored_adaptive_learning_model(
    adaptive: &AdaptiveLearningModel,
) -> inferra_storage::StoredAdaptiveLearningModel {
    inferra_storage::StoredAdaptiveLearningModel {
        schema_version: adaptive.schema_version.max(1) as i64,
        last_updated: adaptive.last_updated.clone(),
        processed_feedback_ids: adaptive.processed_feedback_ids.clone(),
        learned_detectors: adaptive
            .learned_detectors
            .iter()
            .map(|item| inferra_storage::StoredLearnedDetector {
                detector_id: item.detector_id.clone(),
                requirement_name: item.requirement_name.clone(),
                cause_type: item.cause_type.clone(),
                positive_terms: item.positive_terms.clone(),
                tags: item.tags.clone(),
                source_types: item.source_types.clone(),
                min_severity: item.min_severity,
                confirmations: item.confirmations as i64,
                false_positives: item.false_positives as i64,
                created_from_feedback_id: item.created_from_feedback_id.clone(),
                updated_at: item.updated_at.clone(),
                manually_disabled: item.manually_disabled,
                status_reason: item.status_reason.clone(),
                review_status: item.review_status.clone(),
                review_reason: item.review_reason.clone(),
                last_reviewed_at: item.last_reviewed_at.clone(),
            })
            .collect(),
        learned_templates: adaptive
            .learned_templates
            .iter()
            .map(|item| inferra_storage::StoredLearnedTemplate {
                template_id: item.template_id.clone(),
                template_name: item.template_name.clone(),
                cause_type: item.cause_type.clone(),
                cause_subtype: item.cause_subtype.clone(),
                title_template: item.title_template.clone(),
                confidence: item.confidence,
                requires: item.requires.clone(),
                requires_same_service: item.requires_same_service,
                requires_temporal_order: item.requires_temporal_order,
                confirmations: item.confirmations as i64,
                false_positives: item.false_positives as i64,
                created_from_feedback_id: item.created_from_feedback_id.clone(),
                updated_at: item.updated_at.clone(),
                manually_disabled: item.manually_disabled,
                status_reason: item.status_reason.clone(),
                review_status: item.review_status.clone(),
                review_reason: item.review_reason.clone(),
                last_reviewed_at: item.last_reviewed_at.clone(),
            })
            .collect(),
        learned_compositions: adaptive
            .learned_compositions
            .iter()
            .map(|item| inferra_storage::StoredLearnedComposition {
                composition_id: item.composition_id.clone(),
                composition_name: item.composition_name.clone(),
                cause_type: item.cause_type.clone(),
                cause_subtype: item.cause_subtype.clone(),
                title_template: item.title_template.clone(),
                confidence: item.confidence,
                requires: item.requires.clone(),
                requires_same_service: item.requires_same_service,
                requires_temporal_order: item.requires_temporal_order,
                preferred_edge_types: item.preferred_edge_types.clone(),
                confirmations: item.confirmations as i64,
                false_positives: item.false_positives as i64,
                created_from_feedback_id: item.created_from_feedback_id.clone(),
                updated_at: item.updated_at.clone(),
                manually_disabled: item.manually_disabled,
                status_reason: item.status_reason.clone(),
                review_status: item.review_status.clone(),
                review_reason: item.review_reason.clone(),
                last_reviewed_at: item.last_reviewed_at.clone(),
            })
            .collect(),
        learned_edge_profiles: adaptive
            .learned_edge_profiles
            .iter()
            .map(|item| inferra_storage::StoredLearnedEdgeProfile {
                profile_id: item.profile_id.clone(),
                edge_type: item.edge_type.clone(),
                source_service: item.source_service.clone(),
                target_service: item.target_service.clone(),
                cause_type: item.cause_type.clone(),
                confirmations: item.confirmations as i64,
                false_positives: item.false_positives as i64,
                average_plausibility: item.average_plausibility,
                average_latency_ms: item.average_latency_ms,
                created_from_feedback_id: item.created_from_feedback_id.clone(),
                updated_at: item.updated_at.clone(),
                manually_disabled: item.manually_disabled,
                status_reason: item.status_reason.clone(),
                review_status: item.review_status.clone(),
                review_reason: item.review_reason.clone(),
                last_reviewed_at: item.last_reviewed_at.clone(),
            })
            .collect(),
    }
}

fn analyze_domain_events(config: &TomlValue, events: &[EventRow]) -> DomainMetrics {
    let event_count = events.len().max(1);
    let warn_count = events
        .iter()
        .filter(|event| event_severity(event).unwrap_or(SEVERITY_INFO) >= SEVERITY_WARN)
        .count();
    let error_count = events
        .iter()
        .filter(|event| event_severity(event).unwrap_or(SEVERITY_INFO) >= SEVERITY_ERROR)
        .count();
    let restart_count = events
        .iter()
        .filter(|event| {
            has_any_term(
                &event_signal_text(event),
                &["restart", "crash", "panic", "oom", "stopped", "failed"],
            )
        })
        .count();
    let unique_messages = events
        .iter()
        .map(event_signal_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let unique_ratio = (unique_messages as f64 / event_count as f64).clamp(0.0, 1.0);
    let error_rate = error_count as f64 / event_count as f64;
    let warn_rate = warn_count as f64 / event_count as f64;
    let volume_score = (event_count as f64 / 12.0).clamp(0.0, 1.0);
    let restart_score = (restart_count as f64 / event_count as f64 * 2.0).clamp(0.0, 1.0);
    let weights = anomaly_weights(config);
    let anomaly_score = (weights.error_rate * error_rate
        + weights.event_volume * volume_score
        + weights.new_fingerprint_rate * unique_ratio
        + weights.restart_count * restart_score
        + weights.warn_rate * warn_rate)
        .clamp(0.0, 1.0);
    let temporal_alignment = temporal_alignment_score(config, events);
    let min_samples = config
        .get("anomaly_detection")
        .and_then(|value| value.get("min_samples_for_confidence"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(4)
        .max(1) as usize;
    let anomaly_threshold = anomaly_trigger_threshold(config);
    let anomaly_status = if event_count < min_samples {
        "insufficient_data"
    } else if anomaly_score >= anomaly_threshold {
        "anomalous"
    } else if anomaly_score >= (anomaly_threshold * 0.7).clamp(0.2, 0.8) {
        "elevated"
    } else {
        "normal"
    };
    DomainMetrics {
        event_count,
        warn_count,
        error_count,
        anomaly_score,
        anomaly_status: anomaly_status.into(),
        temporal_alignment,
    }
}

fn temporal_alignment_score(config: &TomlValue, events: &[EventRow]) -> f64 {
    let mut timestamps = events
        .iter()
        .filter_map(|event| event.timestamp.as_deref())
        .filter_map(parse_rfc3339)
        .collect::<Vec<_>>();
    timestamps.sort();
    let window_seconds = config
        .get("correlation")
        .and_then(|value| value.get("analysis_window_seconds"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(60)
        .max(1) as f64;
    let span_seconds = match (timestamps.first(), timestamps.last()) {
        (Some(first), Some(last)) => (*last - *first).whole_seconds().abs() as f64,
        _ => 0.0,
    };
    (1.0 - (span_seconds / window_seconds).clamp(0.0, 1.0)).clamp(0.1, 1.0)
}

fn trim_incident_events(
    events: &[EventRow],
    max_events_per_incident: usize,
    enable_auto_split: bool,
) -> Vec<EventRow> {
    if events.len() <= max_events_per_incident {
        return events.to_vec();
    }
    if !enable_auto_split {
        return events[events.len().saturating_sub(max_events_per_incident)..].to_vec();
    }
    let mut grouped = std::collections::BTreeMap::<String, Vec<EventRow>>::new();
    for event in events {
        let key = event
            .source_ref
            .as_ref()
            .and_then(|source| source.source_type.clone())
            .unwrap_or_else(|| "runtime".into());
        grouped.entry(key).or_default().push(event.clone());
    }
    let mut ranked = grouped.into_values().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        max_severity(right)
            .cmp(&max_severity(left))
            .then_with(|| right.len().cmp(&left.len()))
            .then_with(|| last_event_timestamp(right).cmp(&last_event_timestamp(left)))
    });
    let mut trimmed = Vec::new();
    for mut group in ranked {
        if trimmed.len() >= max_events_per_incident {
            break;
        }
        let remaining = max_events_per_incident - trimmed.len();
        if group.len() > remaining {
            let start = group.len().saturating_sub(remaining);
            trimmed.extend(group.drain(start..));
        } else {
            trimmed.append(&mut group);
        }
    }
    trimmed.sort_by(|left, right| left.timestamp.cmp(&right.timestamp));
    trimmed
}

fn last_event_timestamp(events: &[EventRow]) -> Option<String> {
    events
        .iter()
        .filter_map(|event| event.timestamp.clone())
        .max()
}

fn run_incident_lifecycle_maintenance(paths: &Paths, config: &TomlValue) -> Result<()> {
    let Some(mut incidents) = IncidentsStore::open(&paths.incidents_db)? else {
        return Ok(());
    };
    let _ = sync_learning_artifacts(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )?;
    run_incident_lifecycle_maintenance_with_store(
        config,
        &paths.events_db,
        &paths.incidents_db,
        &mut incidents,
    )
}

fn run_incident_lifecycle_maintenance_with_store(
    config: &TomlValue,
    events_db: &Path,
    incidents_db: &Path,
    incidents: &mut IncidentsStore,
) -> Result<()> {
    let stale_timeout_seconds = config
        .get("incident_lifecycle")
        .and_then(|value| value.get("stale_timeout_seconds"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(900)
        .max(60);
    let archive_after_days = config
        .get("incident_lifecycle")
        .and_then(|value| value.get("archive_after_days"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(7)
        .max(1);
    let stale_cutoff = iso_time_before_seconds(stale_timeout_seconds);
    for incident_id in incidents.stale_incident_ids_before(&stale_cutoff, 200)? {
        incidents.transition_state(
            &incident_id,
            "stale",
            &format!("No new events for {stale_timeout_seconds}s"),
            &now_iso(),
        )?;
    }
    let archive_cutoff = iso_time_before_seconds(archive_after_days * 24 * 60 * 60);
    let archived_at = now_iso();
    let archive_path = archive_db_path(incidents_db, &archived_at);
    for incident_id in incidents.archive_candidate_ids_before(&archive_cutoff, 200)? {
        let _ = incidents.archive_incident_to_path(&incident_id, &archive_path, &archived_at)?;
    }
    refresh_active_incident_reasoning(config, events_db, incidents_db, incidents)?;
    Ok(())
}

fn sync_learning_artifacts(
    config: &TomlValue,
    events_db: &Path,
    incidents_db: &Path,
    incidents: &mut IncidentsStore,
) -> Result<LearningArtifacts> {
    let calibration_path = calibration_persistence_path(config, incidents_db);
    let weights_path = scoring_weights_persistence_path(config, incidents_db);
    let events = EventsStore::open(events_db)?;
    let mut calibration =
        load_json_file::<CalibrationModel>(&calibration_path)?.unwrap_or_else(|| {
            CalibrationModel {
                buckets: default_calibration_buckets(calibration_bucket_count(config)),
                ..Default::default()
            }
        });
    if calibration.buckets.is_empty() {
        calibration.buckets = default_calibration_buckets(calibration_bucket_count(config));
    }
    let mut weights =
        load_json_file::<LearnedScoringWeights>(&weights_path)?.unwrap_or_else(|| {
            let defaults = scoring_weight_defaults();
            LearnedScoringWeights {
                default_weights: defaults.clone(),
                effective_weights: defaults,
                ..LearnedScoringWeights::default()
            }
        });
    if weights.default_weights.is_empty() {
        let defaults = scoring_weight_defaults();
        weights.default_weights = defaults.clone();
        if weights.effective_weights.is_empty() {
            weights.effective_weights = defaults;
        }
    }
    import_legacy_adaptive_learning_registry(config, incidents_db, incidents)?;
    let mut adaptive = incidents
        .adaptive_learning_model()?
        .map(adaptive_learning_from_stored)
        .unwrap_or_default();
    for feedback in incidents.all_feedback()? {
        let hypotheses = incidents
            .hypothesis_records(&feedback.incident_id)
            .unwrap_or_default();
        if !calibration
            .processed_feedback_ids
            .iter()
            .any(|item| item == &feedback.feedback_id)
        {
            apply_feedback_to_calibration(config, &mut calibration, &feedback, &hypotheses);
        }
        if !weights
            .processed_feedback_ids
            .iter()
            .any(|item| item == &feedback.feedback_id)
        {
            apply_feedback_to_weights(config, &mut weights, &feedback, &hypotheses);
        }
        if !adaptive
            .processed_feedback_ids
            .iter()
            .any(|item| item == &feedback.feedback_id)
        {
            apply_feedback_to_adaptive_learning(
                &mut adaptive,
                &feedback,
                &hypotheses,
                incidents,
                events.as_ref(),
            )?;
        }
    }
    calibration.staleness_status = calibration_staleness_status(config, &calibration);
    write_json_file(&calibration_path, &calibration)?;
    write_json_file(&weights_path, &weights)?;
    incidents.replace_adaptive_learning_model(&stored_adaptive_learning_model(&adaptive))?;
    Ok(LearningArtifacts {
        calibration,
        weights,
        adaptive,
    })
}

fn refresh_active_incident_reasoning(
    config: &TomlValue,
    events_db: &Path,
    incidents_db: &Path,
    incidents: &mut IncidentsStore,
) -> Result<()> {
    let Some(events) = EventsStore::open(events_db)? else {
        return Ok(());
    };
    let learning = sync_learning_artifacts(config, events_db, incidents_db, incidents)?;
    let active_ids = incidents
        .active_incidents(200)?
        .into_iter()
        .map(|incident| incident.incident_id)
        .collect::<Vec<_>>();
    for incident_id in active_ids {
        let _ = refresh_single_incident_reasoning(
            config,
            incidents_db,
            &incident_id,
            &events,
            incidents,
            &learning,
        )?;
    }
    Ok(())
}

fn calibration_persistence_path(config: &TomlValue, incidents_db: &Path) -> PathBuf {
    let raw = config
        .get("calibration")
        .and_then(|value| value.get("persistence_file"))
        .and_then(TomlValue::as_str)
        .unwrap_or("./data/calibration.json");
    resolve_runtime_sidecar_path(raw, incidents_db)
}

fn scoring_weights_persistence_path(config: &TomlValue, incidents_db: &Path) -> PathBuf {
    let calibration_path = calibration_persistence_path(config, incidents_db);
    calibration_path.with_file_name("scoring_weights.json")
}

fn adaptive_learning_persistence_path(config: &TomlValue, incidents_db: &Path) -> PathBuf {
    let calibration_path = calibration_persistence_path(config, incidents_db);
    calibration_path.with_file_name("adaptive_learning.json")
}

fn adaptive_learning_storage_ref(incidents_db: &Path) -> String {
    format!("{}#adaptive_learning_registry", incidents_db.display())
}

fn adaptive_learning_audit_path(config: &TomlValue, incidents_db: &Path) -> PathBuf {
    let adaptive_path = adaptive_learning_persistence_path(config, incidents_db);
    adaptive_path.with_file_name("adaptive_learning_audit.jsonl")
}

fn adaptive_learning_history_path(config: &TomlValue, incidents_db: &Path) -> PathBuf {
    let adaptive_path = adaptive_learning_persistence_path(config, incidents_db);
    adaptive_path.with_file_name("adaptive_learning_history.jsonl")
}

fn adaptive_learning_audit_storage_ref(incidents_db: &Path) -> String {
    format!("{}#adaptive_learning_audit", incidents_db.display())
}

fn adaptive_learning_history_storage_ref(incidents_db: &Path) -> String {
    format!("{}#adaptive_learning_history", incidents_db.display())
}

fn adaptive_learning_summary_payload(
    adaptive: &AdaptiveLearningModel,
    adaptive_storage: &str,
    audit_storage: &str,
    recent_audit: &[AdaptiveLearningAuditEntry],
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": adaptive.schema_version,
        "last_updated": adaptive.last_updated,
        "path": adaptive_storage,
        "audit_path": audit_storage,
        "processed_feedback_count": adaptive.processed_feedback_ids.len(),
        "counts": {
            "detectors": adaptive.learned_detectors.len(),
            "templates": adaptive.learned_templates.len(),
            "compositions": adaptive.learned_compositions.len(),
            "edge_profiles": adaptive.learned_edge_profiles.len(),
            "active_detectors": adaptive.learned_detectors.iter().filter(|item| detector_is_active(item)).count(),
            "active_templates": adaptive.learned_templates.iter().filter(|item| template_is_active(item)).count(),
            "active_compositions": adaptive.learned_compositions.iter().filter(|item| composition_is_active(item)).count(),
            "active_edge_profiles": adaptive.learned_edge_profiles.iter().filter(|item| edge_profile_is_active(item)).count(),
            "manually_disabled": adaptive.learned_detectors.iter().filter(|item| item.manually_disabled).count()
                + adaptive.learned_templates.iter().filter(|item| item.manually_disabled).count()
                + adaptive.learned_compositions.iter().filter(|item| item.manually_disabled).count()
                + adaptive.learned_edge_profiles.iter().filter(|item| item.manually_disabled).count(),
        },
        "detectors": adaptive.learned_detectors.iter().map(adaptive_detector_json).collect::<Vec<_>>(),
        "templates": adaptive.learned_templates.iter().map(adaptive_template_json).collect::<Vec<_>>(),
        "compositions": adaptive.learned_compositions.iter().map(adaptive_composition_json).collect::<Vec<_>>(),
        "edge_profiles": adaptive.learned_edge_profiles.iter().map(adaptive_edge_profile_json).collect::<Vec<_>>(),
        "recent_audit": recent_audit.iter().map(adaptive_learning_audit_entry_json).collect::<Vec<_>>(),
    })
}

fn append_adaptive_learning_audit(
    incidents: &IncidentsStore,
    entry: &AdaptiveLearningAuditEntry,
) -> Result<()> {
    incidents.add_adaptive_learning_audit_entry(&StoredAdaptiveLearningAuditEntry {
        audit_id: entry.audit_id.clone(),
        artifact_kind: entry.artifact_kind.clone(),
        artifact_id: entry.artifact_id.clone(),
        action: entry.action.clone(),
        reason: entry.reason.clone(),
        previous_status: entry.previous_status.clone(),
        new_status: entry.new_status.clone(),
        review_status_before: entry.review_status_before.clone(),
        review_status_after: entry.review_status_after.clone(),
        runtime_effect: entry.runtime_effect.clone(),
        created_at: entry.created_at.clone(),
    })
}

fn read_adaptive_learning_audit(
    incidents: &IncidentsStore,
    limit: usize,
) -> Result<Vec<AdaptiveLearningAuditEntry>> {
    let mut entries = incidents
        .list_adaptive_learning_audit(&AdaptiveLearningAuditQuery {
            limit,
            ..AdaptiveLearningAuditQuery::default()
        })?
        .into_iter()
        .map(|entry| AdaptiveLearningAuditEntry {
            audit_id: entry.audit_id,
            artifact_kind: entry.artifact_kind,
            artifact_id: entry.artifact_id,
            action: entry.action,
            reason: entry.reason,
            previous_status: entry.previous_status,
            new_status: entry.new_status,
            review_status_before: entry.review_status_before,
            review_status_after: entry.review_status_after,
            runtime_effect: entry.runtime_effect,
            created_at: entry.created_at,
        })
        .collect::<Vec<_>>();
    entries.reverse();
    Ok(entries)
}

fn adaptive_learning_audit_entry_json(entry: &AdaptiveLearningAuditEntry) -> serde_json::Value {
    serde_json::json!({
        "audit_id": entry.audit_id,
        "artifact_kind": entry.artifact_kind,
        "artifact_id": entry.artifact_id,
        "action": entry.action,
        "reason": entry.reason,
        "previous_status": entry.previous_status,
        "new_status": entry.new_status,
        "review_status_before": entry.review_status_before,
        "review_status_after": entry.review_status_after,
        "runtime_effect": entry.runtime_effect,
        "created_at": entry.created_at,
    })
}

fn append_adaptive_learning_history(
    incidents: &mut IncidentsStore,
    entries: &[AdaptiveLearningHistoryEntry],
) -> Result<()> {
    incidents.add_adaptive_learning_history_entries(
        &entries
            .iter()
            .map(|entry| StoredAdaptiveLearningHistoryEntry {
                entry_id: entry.entry_id.clone(),
                artifact_kind: entry.artifact_kind.clone(),
                artifact_id: entry.artifact_id.clone(),
                artifact_label: entry.artifact_label.clone(),
                incident_id: entry.incident_id.clone(),
                cause_type: entry.cause_type.clone(),
                hypothesis_id: entry.hypothesis_id.clone(),
                observed_at: entry.observed_at.clone(),
                score: entry.score,
                rank: entry.rank,
                estimated_impact: entry.estimated_impact,
                impact_metric: entry.impact_metric.clone(),
                score_delta: entry.score_delta,
                rank_delta: entry.rank_delta,
                edge_delta: entry.edge_delta,
            })
            .collect::<Vec<_>>(),
    )
}

fn read_adaptive_learning_history(
    incidents: &IncidentsStore,
    limit: usize,
) -> Result<Vec<AdaptiveLearningHistoryEntry>> {
    let mut entries = incidents
        .list_adaptive_learning_history(&AdaptiveLearningHistoryQuery {
            limit,
            ..AdaptiveLearningHistoryQuery::default()
        })?
        .into_iter()
        .map(|entry| AdaptiveLearningHistoryEntry {
            entry_id: entry.entry_id,
            artifact_kind: entry.artifact_kind,
            artifact_id: entry.artifact_id,
            artifact_label: entry.artifact_label,
            incident_id: entry.incident_id,
            cause_type: entry.cause_type,
            hypothesis_id: entry.hypothesis_id,
            observed_at: entry.observed_at,
            score: entry.score,
            rank: entry.rank,
            estimated_impact: entry.estimated_impact,
            impact_metric: entry.impact_metric,
            score_delta: entry.score_delta,
            rank_delta: entry.rank_delta,
            edge_delta: entry.edge_delta,
        })
        .collect::<Vec<_>>();
    entries.reverse();
    Ok(entries)
}

fn adaptive_learning_history_entry_json(entry: &AdaptiveLearningHistoryEntry) -> serde_json::Value {
    serde_json::json!({
        "entry_id": entry.entry_id,
        "artifact_kind": entry.artifact_kind,
        "artifact_id": entry.artifact_id,
        "artifact_label": entry.artifact_label,
        "incident_id": entry.incident_id,
        "cause_type": entry.cause_type,
        "hypothesis_id": entry.hypothesis_id,
        "observed_at": entry.observed_at,
        "score": entry.score,
        "rank": entry.rank,
        "estimated_impact": entry.estimated_impact,
        "impact_metric": entry.impact_metric,
        "score_delta": entry.score_delta,
        "rank_delta": entry.rank_delta,
        "edge_delta": entry.edge_delta,
    })
}

fn ensure_adaptive_learning_storage_imported(
    config: &TomlValue,
    incidents_db: &Path,
    incidents: &mut IncidentsStore,
) -> Result<()> {
    import_legacy_adaptive_learning_registry(config, incidents_db, incidents)?;
    import_legacy_adaptive_learning_audit(config, incidents_db, incidents)?;
    import_legacy_adaptive_learning_history(config, incidents_db, incidents)?;
    Ok(())
}

fn import_legacy_adaptive_learning_registry(
    config: &TomlValue,
    incidents_db: &Path,
    incidents: &mut IncidentsStore,
) -> Result<()> {
    if incidents.adaptive_learning_model()?.is_some() {
        return Ok(());
    }
    let legacy_path = adaptive_learning_persistence_path(config, incidents_db);
    if !legacy_path.exists() {
        return Ok(());
    }
    let Some(model) = load_json_file::<AdaptiveLearningModel>(&legacy_path)? else {
        return Ok(());
    };
    incidents.replace_adaptive_learning_model(&stored_adaptive_learning_model(&model))?;
    Ok(())
}

fn import_legacy_adaptive_learning_audit(
    config: &TomlValue,
    incidents_db: &Path,
    incidents: &mut IncidentsStore,
) -> Result<()> {
    let legacy_path = adaptive_learning_audit_path(config, incidents_db);
    if !legacy_path.exists() {
        return Ok(());
    }
    if !incidents
        .list_adaptive_learning_audit(&AdaptiveLearningAuditQuery {
            limit: 1,
            ..AdaptiveLearningAuditQuery::default()
        })?
        .is_empty()
    {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&legacy_path)
        .map_err(|error| anyhow::anyhow!("read {}: {error}", legacy_path.display()))?;
    for entry in raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<AdaptiveLearningAuditEntry>(line).ok())
    {
        append_adaptive_learning_audit(incidents, &entry)?;
    }
    Ok(())
}

fn import_legacy_adaptive_learning_history(
    config: &TomlValue,
    incidents_db: &Path,
    incidents: &mut IncidentsStore,
) -> Result<()> {
    let legacy_path = adaptive_learning_history_path(config, incidents_db);
    if !legacy_path.exists() {
        return Ok(());
    }
    if !incidents
        .list_adaptive_learning_history(&AdaptiveLearningHistoryQuery {
            limit: 1,
            ..AdaptiveLearningHistoryQuery::default()
        })?
        .is_empty()
    {
        return Ok(());
    }
    let raw = std::fs::read_to_string(&legacy_path)
        .map_err(|error| anyhow::anyhow!("read {}: {error}", legacy_path.display()))?;
    let entries = raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<AdaptiveLearningHistoryEntry>(line).ok())
        .collect::<Vec<_>>();
    append_adaptive_learning_history(incidents, &entries)
}

fn record_adaptive_learning_history(
    incidents: &mut IncidentsStore,
    incident_id: &str,
    observed_at: &str,
    previous_hypotheses: &[serde_json::Value],
    new_hypotheses: &[StoredHypothesis],
) -> Result<()> {
    let entries = build_adaptive_learning_history_entries(
        incident_id,
        observed_at,
        previous_hypotheses,
        new_hypotheses,
    );
    append_adaptive_learning_history(incidents, &entries)
}

fn build_adaptive_learning_history_entries(
    incident_id: &str,
    observed_at: &str,
    previous_hypotheses: &[serde_json::Value],
    new_hypotheses: &[StoredHypothesis],
) -> Vec<AdaptiveLearningHistoryEntry> {
    let mut entries = Vec::new();
    for hypothesis in new_hypotheses {
        let Some(provenance) = hypothesis.score_breakdown.get("provenance") else {
            continue;
        };
        let Some(artifacts) = provenance
            .get("artifacts")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for artifact in artifacts {
            let artifact_kind = artifact
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let artifact_id = artifact
                .get("artifact_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let artifact_label = artifact
                .get("label")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(artifact_id);
            let impact_metric = artifact
                .get("impact_metric")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let estimated_impact = artifact
                .get("impact_value")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default();
            let previous = previous_artifact_observation(
                previous_hypotheses,
                artifact_kind,
                artifact_id,
                &hypothesis.cause_type,
            );
            entries.push(AdaptiveLearningHistoryEntry {
                entry_id: format!(
                    "hist-{}-{}-{}",
                    artifact_id,
                    hypothesis.hypothesis_id,
                    time::OffsetDateTime::now_utc().unix_timestamp_nanos()
                ),
                artifact_kind: artifact_kind.to_string(),
                artifact_id: artifact_id.to_string(),
                artifact_label: artifact_label.to_string(),
                incident_id: incident_id.to_string(),
                cause_type: hypothesis.cause_type.clone(),
                hypothesis_id: hypothesis.hypothesis_id.clone(),
                observed_at: observed_at.to_string(),
                score: hypothesis.total_score,
                rank: hypothesis.rank,
                estimated_impact,
                impact_metric: impact_metric.clone(),
                score_delta: previous
                    .and_then(|(score, _)| hypothesis.total_score.map(|current| current - score)),
                rank_delta: previous
                    .and_then(|(_, rank)| hypothesis.rank.map(|current| rank - current)),
                edge_delta: (impact_metric.as_deref() == Some("plausibility_delta"))
                    .then_some(estimated_impact),
            });
        }
    }
    entries
}

fn previous_artifact_observation(
    previous_hypotheses: &[serde_json::Value],
    artifact_kind: &str,
    artifact_id: &str,
    cause_type: &str,
) -> Option<(f64, i64)> {
    previous_hypotheses
        .iter()
        .filter(|hypothesis| {
            hypothesis
                .get("cause_type")
                .and_then(serde_json::Value::as_str)
                == Some(cause_type)
        })
        .find_map(|hypothesis| {
            let matched = hypothesis
                .get("score_breakdown")
                .and_then(|value| value.get("provenance"))
                .and_then(|value| value.get("artifacts"))
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items.iter().any(|artifact| {
                        artifact.get("kind").and_then(serde_json::Value::as_str)
                            == Some(artifact_kind)
                            && artifact
                                .get("artifact_id")
                                .and_then(serde_json::Value::as_str)
                                == Some(artifact_id)
                    })
                })
                .unwrap_or(false);
            if !matched {
                return None;
            }
            Some((
                hypothesis
                    .get("total_score")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or_default(),
                hypothesis
                    .get("rank")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default(),
            ))
        })
}

fn summarize_incident_learning_influence(
    incident: &IncidentRow,
    hypotheses: &[serde_json::Value],
) -> serde_json::Value {
    let mut influenced_hypotheses = 0u64;
    let mut total_estimated_impact = 0.0;
    let mut artifacts = std::collections::BTreeMap::<String, serde_json::Value>::new();
    for hypothesis in hypotheses {
        let Some(provenance) = hypothesis
            .get("score_breakdown")
            .and_then(|value| value.get("provenance"))
        else {
            continue;
        };
        if provenance
            .get("has_learned_influence")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            influenced_hypotheses += 1;
        }
        total_estimated_impact += provenance
            .get("estimated_total_impact")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default();
        for artifact in provenance
            .get("artifacts")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let key = format!(
                "{}:{}",
                artifact
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown"),
                artifact
                    .get("artifact_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
            );
            let impact = artifact
                .get("impact_value")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default();
            let entry = artifacts.entry(key).or_insert_with(|| {
                serde_json::json!({
                    "kind": artifact.get("kind").cloned().unwrap_or(serde_json::Value::Null),
                    "artifact_id": artifact.get("artifact_id").cloned().unwrap_or(serde_json::Value::Null),
                    "label": artifact.get("label").cloned().unwrap_or(serde_json::Value::Null),
                    "impact_metric": artifact.get("impact_metric").cloned().unwrap_or(serde_json::Value::Null),
                    "cumulative_impact": 0.0,
                })
            });
            let current = entry
                .get("cumulative_impact")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default();
            entry["cumulative_impact"] = serde_json::json!(current + impact);
        }
    }
    let mut artifact_list = artifacts.into_values().collect::<Vec<_>>();
    artifact_list.sort_by(|left, right| {
        right["cumulative_impact"]
            .as_f64()
            .unwrap_or_default()
            .partial_cmp(&left["cumulative_impact"].as_f64().unwrap_or_default())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    serde_json::json!({
        "incident_id": incident.incident_id,
        "influenced_hypotheses": influenced_hypotheses,
        "estimated_total_impact": total_estimated_impact,
        "artifacts": artifact_list,
    })
}

fn artifacts_requiring_attention(adaptive: &AdaptiveLearningModel) -> Vec<serde_json::Value> {
    let mut items = Vec::new();
    for detector in adaptive.learned_detectors.iter().filter(|item| {
        item.manually_disabled || item.false_positives > 0 || !detector_is_active(item)
    }) {
        let mut value = adaptive_detector_json(detector);
        value["kind"] = serde_json::json!("detector");
        items.push(value);
    }
    for template in adaptive.learned_templates.iter().filter(|item| {
        item.manually_disabled || item.false_positives > 0 || !template_is_active(item)
    }) {
        let mut value = adaptive_template_json(template);
        value["kind"] = serde_json::json!("template");
        items.push(value);
    }
    for composition in adaptive.learned_compositions.iter().filter(|item| {
        item.manually_disabled || item.false_positives > 0 || !composition_is_active(item)
    }) {
        let mut value = adaptive_composition_json(composition);
        value["kind"] = serde_json::json!("composition");
        items.push(value);
    }
    for profile in adaptive.learned_edge_profiles.iter().filter(|item| {
        item.manually_disabled || item.false_positives > 0 || !edge_profile_is_active(item)
    }) {
        let mut value = adaptive_edge_profile_json(profile);
        value["kind"] = serde_json::json!("edge_profile");
        items.push(value);
    }
    items
}

fn adaptive_detector_json(detector: &LearnedDetector) -> serde_json::Value {
    serde_json::json!({
        "detector_id": detector.detector_id,
        "requirement_name": detector.requirement_name,
        "cause_type": detector.cause_type,
        "positive_terms": detector.positive_terms,
        "tags": detector.tags,
        "source_types": detector.source_types,
        "min_severity": detector.min_severity,
        "confirmations": detector.confirmations,
        "false_positives": detector.false_positives,
        "manually_disabled": detector.manually_disabled,
        "status_reason": detector.status_reason,
        "status": adaptive_artifact_status(detector.manually_disabled, detector.confirmations, detector.false_positives),
        "review_status": detector.review_status,
        "review_reason": detector.review_reason,
        "last_reviewed_at": detector.last_reviewed_at,
        "created_from_feedback_id": detector.created_from_feedback_id,
        "updated_at": detector.updated_at,
    })
}

fn adaptive_template_json(template: &LearnedTemplate) -> serde_json::Value {
    serde_json::json!({
        "template_id": template.template_id,
        "template_name": template.template_name,
        "cause_type": template.cause_type,
        "cause_subtype": template.cause_subtype,
        "title_template": template.title_template,
        "confidence": template.confidence,
        "requires": template.requires,
        "requires_same_service": template.requires_same_service,
        "requires_temporal_order": template.requires_temporal_order,
        "confirmations": template.confirmations,
        "false_positives": template.false_positives,
        "manually_disabled": template.manually_disabled,
        "status_reason": template.status_reason,
        "status": adaptive_artifact_status(template.manually_disabled, template.confirmations, template.false_positives),
        "review_status": template.review_status,
        "review_reason": template.review_reason,
        "last_reviewed_at": template.last_reviewed_at,
        "created_from_feedback_id": template.created_from_feedback_id,
        "updated_at": template.updated_at,
    })
}

fn adaptive_composition_json(composition: &LearnedComposition) -> serde_json::Value {
    serde_json::json!({
        "composition_id": composition.composition_id,
        "composition_name": composition.composition_name,
        "cause_type": composition.cause_type,
        "cause_subtype": composition.cause_subtype,
        "title_template": composition.title_template,
        "confidence": composition.confidence,
        "requires": composition.requires,
        "requires_same_service": composition.requires_same_service,
        "requires_temporal_order": composition.requires_temporal_order,
        "preferred_edge_types": composition.preferred_edge_types,
        "confirmations": composition.confirmations,
        "false_positives": composition.false_positives,
        "manually_disabled": composition.manually_disabled,
        "status_reason": composition.status_reason,
        "status": adaptive_artifact_status(composition.manually_disabled, composition.confirmations, composition.false_positives),
        "review_status": composition.review_status,
        "review_reason": composition.review_reason,
        "last_reviewed_at": composition.last_reviewed_at,
        "created_from_feedback_id": composition.created_from_feedback_id,
        "updated_at": composition.updated_at,
    })
}

fn adaptive_edge_profile_json(profile: &LearnedEdgeProfile) -> serde_json::Value {
    serde_json::json!({
        "profile_id": profile.profile_id,
        "edge_type": profile.edge_type,
        "source_service": profile.source_service,
        "target_service": profile.target_service,
        "cause_type": profile.cause_type,
        "confirmations": profile.confirmations,
        "false_positives": profile.false_positives,
        "average_plausibility": profile.average_plausibility,
        "average_latency_ms": profile.average_latency_ms,
        "manually_disabled": profile.manually_disabled,
        "status_reason": profile.status_reason,
        "status": adaptive_artifact_status(profile.manually_disabled, profile.confirmations, profile.false_positives),
        "review_status": profile.review_status,
        "review_reason": profile.review_reason,
        "last_reviewed_at": profile.last_reviewed_at,
        "created_from_feedback_id": profile.created_from_feedback_id,
        "updated_at": profile.updated_at,
    })
}

fn adaptive_artifact_status(
    manually_disabled: bool,
    confirmations: u64,
    false_positives: u64,
) -> &'static str {
    if manually_disabled {
        "manually_disabled"
    } else if confirmations > false_positives {
        "active"
    } else {
        "suppressed"
    }
}

fn normalize_adaptive_artifact_kind(value: &str) -> Result<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "detector" | "detectors" => Ok("detector"),
        "template" | "templates" => Ok("template"),
        "composition" | "compositions" => Ok("composition"),
        "edge_profile" | "edge-profile" | "edge_profiles" | "edge-profiles" => Ok("edge_profile"),
        other => Err(anyhow::anyhow!(
            "unsupported artifact kind '{other}', expected detector, template, composition, or edge_profile"
        )),
    }
}

fn default_review_status() -> String {
    "unreviewed".into()
}

trait ReviewableAdaptiveArtifact {
    fn manually_disabled(&self) -> bool;
    fn manually_disabled_mut(&mut self) -> &mut bool;
    fn status_reason_mut(&mut self) -> &mut Option<String>;
    fn confirmations(&self) -> u64;
    fn false_positives(&self) -> u64;
    fn review_status(&self) -> &str;
    fn review_status_mut(&mut self) -> &mut String;
    fn review_reason_mut(&mut self) -> &mut Option<String>;
    fn last_reviewed_at_mut(&mut self) -> &mut Option<String>;
    fn updated_at_mut(&mut self) -> &mut String;
}

impl ReviewableAdaptiveArtifact for LearnedDetector {
    fn manually_disabled(&self) -> bool {
        self.manually_disabled
    }
    fn manually_disabled_mut(&mut self) -> &mut bool {
        &mut self.manually_disabled
    }
    fn status_reason_mut(&mut self) -> &mut Option<String> {
        &mut self.status_reason
    }
    fn confirmations(&self) -> u64 {
        self.confirmations
    }
    fn false_positives(&self) -> u64 {
        self.false_positives
    }
    fn review_status(&self) -> &str {
        &self.review_status
    }
    fn review_status_mut(&mut self) -> &mut String {
        &mut self.review_status
    }
    fn review_reason_mut(&mut self) -> &mut Option<String> {
        &mut self.review_reason
    }
    fn last_reviewed_at_mut(&mut self) -> &mut Option<String> {
        &mut self.last_reviewed_at
    }
    fn updated_at_mut(&mut self) -> &mut String {
        &mut self.updated_at
    }
}

impl ReviewableAdaptiveArtifact for LearnedTemplate {
    fn manually_disabled(&self) -> bool {
        self.manually_disabled
    }
    fn manually_disabled_mut(&mut self) -> &mut bool {
        &mut self.manually_disabled
    }
    fn status_reason_mut(&mut self) -> &mut Option<String> {
        &mut self.status_reason
    }
    fn confirmations(&self) -> u64 {
        self.confirmations
    }
    fn false_positives(&self) -> u64 {
        self.false_positives
    }
    fn review_status(&self) -> &str {
        &self.review_status
    }
    fn review_status_mut(&mut self) -> &mut String {
        &mut self.review_status
    }
    fn review_reason_mut(&mut self) -> &mut Option<String> {
        &mut self.review_reason
    }
    fn last_reviewed_at_mut(&mut self) -> &mut Option<String> {
        &mut self.last_reviewed_at
    }
    fn updated_at_mut(&mut self) -> &mut String {
        &mut self.updated_at
    }
}

impl ReviewableAdaptiveArtifact for LearnedComposition {
    fn manually_disabled(&self) -> bool {
        self.manually_disabled
    }
    fn manually_disabled_mut(&mut self) -> &mut bool {
        &mut self.manually_disabled
    }
    fn status_reason_mut(&mut self) -> &mut Option<String> {
        &mut self.status_reason
    }
    fn confirmations(&self) -> u64 {
        self.confirmations
    }
    fn false_positives(&self) -> u64 {
        self.false_positives
    }
    fn review_status(&self) -> &str {
        &self.review_status
    }
    fn review_status_mut(&mut self) -> &mut String {
        &mut self.review_status
    }
    fn review_reason_mut(&mut self) -> &mut Option<String> {
        &mut self.review_reason
    }
    fn last_reviewed_at_mut(&mut self) -> &mut Option<String> {
        &mut self.last_reviewed_at
    }
    fn updated_at_mut(&mut self) -> &mut String {
        &mut self.updated_at
    }
}

impl ReviewableAdaptiveArtifact for LearnedEdgeProfile {
    fn manually_disabled(&self) -> bool {
        self.manually_disabled
    }
    fn manually_disabled_mut(&mut self) -> &mut bool {
        &mut self.manually_disabled
    }
    fn status_reason_mut(&mut self) -> &mut Option<String> {
        &mut self.status_reason
    }
    fn confirmations(&self) -> u64 {
        self.confirmations
    }
    fn false_positives(&self) -> u64 {
        self.false_positives
    }
    fn review_status(&self) -> &str {
        &self.review_status
    }
    fn review_status_mut(&mut self) -> &mut String {
        &mut self.review_status
    }
    fn review_reason_mut(&mut self) -> &mut Option<String> {
        &mut self.review_reason
    }
    fn last_reviewed_at_mut(&mut self) -> &mut Option<String> {
        &mut self.last_reviewed_at
    }
    fn updated_at_mut(&mut self) -> &mut String {
        &mut self.updated_at
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_review_decision_to_artifact<T: ReviewableAdaptiveArtifact>(
    item: &mut T,
    decision: &str,
    reason: &Option<String>,
    reviewed_at: &str,
    previous_status: &mut Option<String>,
    new_status: &mut Option<String>,
    review_status_before: &mut Option<String>,
    review_status_after: &mut Option<String>,
    runtime_effect: &mut Option<String>,
    refresh_reasoning: &mut bool,
) -> Result<()> {
    *previous_status = Some(
        adaptive_artifact_status(
            item.manually_disabled(),
            item.confirmations(),
            item.false_positives(),
        )
        .to_string(),
    );
    *review_status_before = Some(item.review_status().to_string());
    match decision {
        "approve" => {
            *item.review_status_mut() = "approved".into();
            *item.review_reason_mut() = reason.clone();
            *item.last_reviewed_at_mut() = Some(reviewed_at.to_string());
            *item.updated_at_mut() = reviewed_at.to_string();
            *new_status = previous_status.clone();
            *runtime_effect = Some("review approved; runtime state unchanged".into());
        }
        "watch" => {
            *item.review_status_mut() = "watch".into();
            *item.review_reason_mut() = reason.clone();
            *item.last_reviewed_at_mut() = Some(reviewed_at.to_string());
            *item.updated_at_mut() = reviewed_at.to_string();
            *new_status = previous_status.clone();
            *runtime_effect = Some("artifact placed under watch; runtime state unchanged".into());
        }
        "reject" => {
            *item.review_status_mut() = "rejected".into();
            *item.review_reason_mut() = reason.clone();
            *item.last_reviewed_at_mut() = Some(reviewed_at.to_string());
            *item.updated_at_mut() = reviewed_at.to_string();
            if !item.manually_disabled() {
                *item.manually_disabled_mut() = true;
                if item.status_reason_mut().is_none() {
                    *item.status_reason_mut() = reason.clone();
                }
                *refresh_reasoning = true;
                *runtime_effect =
                    Some("artifact rejected, disabled, and reasoning refreshed".into());
            } else {
                *runtime_effect = Some("artifact rejected; already disabled".into());
            }
            *new_status = Some(
                adaptive_artifact_status(
                    item.manually_disabled(),
                    item.confirmations(),
                    item.false_positives(),
                )
                .to_string(),
            );
        }
        "reset" => {
            *item.review_status_mut() = default_review_status();
            *item.review_reason_mut() = None;
            *item.last_reviewed_at_mut() = None;
            *item.updated_at_mut() = reviewed_at.to_string();
            *new_status = previous_status.clone();
            *runtime_effect = Some("review state reset; runtime state unchanged".into());
        }
        other => {
            return Err(anyhow::anyhow!(
                "unsupported review decision '{other}', expected approve, watch, reject, or reset"
            ))
        }
    }
    *review_status_after = Some(item.review_status().to_string());
    Ok(())
}

fn apply_runtime_action_to_selected_artifact(
    adaptive: &mut AdaptiveLearningModel,
    artifact_kind: &str,
    artifact_id: &str,
    disable: bool,
    status_reason: &Option<String>,
    updated_at: &str,
) -> Result<Option<AdaptiveArtifactMutationRecord>> {
    let normalized_kind = normalize_adaptive_artifact_kind(artifact_kind)?;
    let update = match normalized_kind {
        "detector" => adaptive
            .learned_detectors
            .iter_mut()
            .find(|item| item.detector_id == artifact_id || item.requirement_name == artifact_id)
            .map(|item| {
                let previous_status = adaptive_artifact_status(
                    item.manually_disabled,
                    item.confirmations,
                    item.false_positives,
                )
                .to_string();
                item.manually_disabled = disable;
                item.status_reason = status_reason.clone();
                item.updated_at = updated_at.to_string();
                AdaptiveArtifactMutationRecord {
                    artifact_kind: "detector".into(),
                    artifact_id: artifact_id.to_string(),
                    label: item.requirement_name.clone(),
                    previous_status,
                    new_status: adaptive_artifact_status(
                        item.manually_disabled,
                        item.confirmations,
                        item.false_positives,
                    )
                    .to_string(),
                    review_status_before: None,
                    review_status_after: None,
                    runtime_effect: Some(if disable {
                        "artifact disabled and reasoning refreshed".into()
                    } else {
                        "artifact enabled and reasoning refreshed".into()
                    }),
                    updated_at: updated_at.to_string(),
                    refresh_reasoning: true,
                }
            }),
        "template" => adaptive
            .learned_templates
            .iter_mut()
            .find(|item| item.template_id == artifact_id || item.template_name == artifact_id)
            .map(|item| {
                let previous_status = adaptive_artifact_status(
                    item.manually_disabled,
                    item.confirmations,
                    item.false_positives,
                )
                .to_string();
                item.manually_disabled = disable;
                item.status_reason = status_reason.clone();
                item.updated_at = updated_at.to_string();
                AdaptiveArtifactMutationRecord {
                    artifact_kind: "template".into(),
                    artifact_id: artifact_id.to_string(),
                    label: item.template_name.clone(),
                    previous_status,
                    new_status: adaptive_artifact_status(
                        item.manually_disabled,
                        item.confirmations,
                        item.false_positives,
                    )
                    .to_string(),
                    review_status_before: None,
                    review_status_after: None,
                    runtime_effect: Some(if disable {
                        "artifact disabled and reasoning refreshed".into()
                    } else {
                        "artifact enabled and reasoning refreshed".into()
                    }),
                    updated_at: updated_at.to_string(),
                    refresh_reasoning: true,
                }
            }),
        "composition" => adaptive
            .learned_compositions
            .iter_mut()
            .find(|item| item.composition_id == artifact_id || item.composition_name == artifact_id)
            .map(|item| {
                let previous_status = adaptive_artifact_status(
                    item.manually_disabled,
                    item.confirmations,
                    item.false_positives,
                )
                .to_string();
                item.manually_disabled = disable;
                item.status_reason = status_reason.clone();
                item.updated_at = updated_at.to_string();
                AdaptiveArtifactMutationRecord {
                    artifact_kind: "composition".into(),
                    artifact_id: artifact_id.to_string(),
                    label: item.composition_name.clone(),
                    previous_status,
                    new_status: adaptive_artifact_status(
                        item.manually_disabled,
                        item.confirmations,
                        item.false_positives,
                    )
                    .to_string(),
                    review_status_before: None,
                    review_status_after: None,
                    runtime_effect: Some(if disable {
                        "artifact disabled and reasoning refreshed".into()
                    } else {
                        "artifact enabled and reasoning refreshed".into()
                    }),
                    updated_at: updated_at.to_string(),
                    refresh_reasoning: true,
                }
            }),
        "edge_profile" => adaptive
            .learned_edge_profiles
            .iter_mut()
            .find(|item| item.profile_id == artifact_id)
            .map(|item| {
                let previous_status = adaptive_artifact_status(
                    item.manually_disabled,
                    item.confirmations,
                    item.false_positives,
                )
                .to_string();
                item.manually_disabled = disable;
                item.status_reason = status_reason.clone();
                item.updated_at = updated_at.to_string();
                AdaptiveArtifactMutationRecord {
                    artifact_kind: "edge_profile".into(),
                    artifact_id: artifact_id.to_string(),
                    label: item.profile_id.clone(),
                    previous_status,
                    new_status: adaptive_artifact_status(
                        item.manually_disabled,
                        item.confirmations,
                        item.false_positives,
                    )
                    .to_string(),
                    review_status_before: None,
                    review_status_after: None,
                    runtime_effect: Some(if disable {
                        "artifact disabled and reasoning refreshed".into()
                    } else {
                        "artifact enabled and reasoning refreshed".into()
                    }),
                    updated_at: updated_at.to_string(),
                    refresh_reasoning: true,
                }
            }),
        _ => None,
    };
    Ok(update)
}

fn apply_review_decision_to_selected_artifact(
    adaptive: &mut AdaptiveLearningModel,
    artifact_kind: &str,
    artifact_id: &str,
    decision: &str,
    review_reason: &Option<String>,
    reviewed_at: &str,
) -> Result<Option<AdaptiveArtifactMutationRecord>> {
    let normalized_kind = normalize_adaptive_artifact_kind(artifact_kind)?;
    let update = match normalized_kind {
        "detector" => adaptive
            .learned_detectors
            .iter_mut()
            .find(|item| item.detector_id == artifact_id || item.requirement_name == artifact_id)
            .map(|item| {
                let mut previous_status = None::<String>;
                let mut new_status = None::<String>;
                let mut review_status_before = None::<String>;
                let mut review_status_after = None::<String>;
                let mut runtime_effect =
                    Some("review recorded without runtime state change".to_string());
                let mut refresh_reasoning = false;
                apply_review_decision_to_artifact(
                    item,
                    decision,
                    review_reason,
                    reviewed_at,
                    &mut previous_status,
                    &mut new_status,
                    &mut review_status_before,
                    &mut review_status_after,
                    &mut runtime_effect,
                    &mut refresh_reasoning,
                )?;
                Ok::<AdaptiveArtifactMutationRecord, anyhow::Error>(
                    AdaptiveArtifactMutationRecord {
                        artifact_kind: "detector".into(),
                        artifact_id: artifact_id.to_string(),
                        label: item.requirement_name.clone(),
                        previous_status: previous_status.unwrap_or_else(|| "unknown".into()),
                        new_status: new_status.unwrap_or_else(|| "unknown".into()),
                        review_status_before,
                        review_status_after,
                        runtime_effect,
                        updated_at: reviewed_at.to_string(),
                        refresh_reasoning,
                    },
                )
            })
            .transpose()?,
        "template" => adaptive
            .learned_templates
            .iter_mut()
            .find(|item| item.template_id == artifact_id || item.template_name == artifact_id)
            .map(|item| {
                let mut previous_status = None::<String>;
                let mut new_status = None::<String>;
                let mut review_status_before = None::<String>;
                let mut review_status_after = None::<String>;
                let mut runtime_effect =
                    Some("review recorded without runtime state change".to_string());
                let mut refresh_reasoning = false;
                apply_review_decision_to_artifact(
                    item,
                    decision,
                    review_reason,
                    reviewed_at,
                    &mut previous_status,
                    &mut new_status,
                    &mut review_status_before,
                    &mut review_status_after,
                    &mut runtime_effect,
                    &mut refresh_reasoning,
                )?;
                Ok::<AdaptiveArtifactMutationRecord, anyhow::Error>(
                    AdaptiveArtifactMutationRecord {
                        artifact_kind: "template".into(),
                        artifact_id: artifact_id.to_string(),
                        label: item.template_name.clone(),
                        previous_status: previous_status.unwrap_or_else(|| "unknown".into()),
                        new_status: new_status.unwrap_or_else(|| "unknown".into()),
                        review_status_before,
                        review_status_after,
                        runtime_effect,
                        updated_at: reviewed_at.to_string(),
                        refresh_reasoning,
                    },
                )
            })
            .transpose()?,
        "composition" => adaptive
            .learned_compositions
            .iter_mut()
            .find(|item| item.composition_id == artifact_id || item.composition_name == artifact_id)
            .map(|item| {
                let mut previous_status = None::<String>;
                let mut new_status = None::<String>;
                let mut review_status_before = None::<String>;
                let mut review_status_after = None::<String>;
                let mut runtime_effect =
                    Some("review recorded without runtime state change".to_string());
                let mut refresh_reasoning = false;
                apply_review_decision_to_artifact(
                    item,
                    decision,
                    review_reason,
                    reviewed_at,
                    &mut previous_status,
                    &mut new_status,
                    &mut review_status_before,
                    &mut review_status_after,
                    &mut runtime_effect,
                    &mut refresh_reasoning,
                )?;
                Ok::<AdaptiveArtifactMutationRecord, anyhow::Error>(
                    AdaptiveArtifactMutationRecord {
                        artifact_kind: "composition".into(),
                        artifact_id: artifact_id.to_string(),
                        label: item.composition_name.clone(),
                        previous_status: previous_status.unwrap_or_else(|| "unknown".into()),
                        new_status: new_status.unwrap_or_else(|| "unknown".into()),
                        review_status_before,
                        review_status_after,
                        runtime_effect,
                        updated_at: reviewed_at.to_string(),
                        refresh_reasoning,
                    },
                )
            })
            .transpose()?,
        "edge_profile" => adaptive
            .learned_edge_profiles
            .iter_mut()
            .find(|item| item.profile_id == artifact_id)
            .map(|item| {
                let mut previous_status = None::<String>;
                let mut new_status = None::<String>;
                let mut review_status_before = None::<String>;
                let mut review_status_after = None::<String>;
                let mut runtime_effect =
                    Some("review recorded without runtime state change".to_string());
                let mut refresh_reasoning = false;
                apply_review_decision_to_artifact(
                    item,
                    decision,
                    review_reason,
                    reviewed_at,
                    &mut previous_status,
                    &mut new_status,
                    &mut review_status_before,
                    &mut review_status_after,
                    &mut runtime_effect,
                    &mut refresh_reasoning,
                )?;
                Ok::<AdaptiveArtifactMutationRecord, anyhow::Error>(
                    AdaptiveArtifactMutationRecord {
                        artifact_kind: "edge_profile".into(),
                        artifact_id: artifact_id.to_string(),
                        label: item.profile_id.clone(),
                        previous_status: previous_status.unwrap_or_else(|| "unknown".into()),
                        new_status: new_status.unwrap_or_else(|| "unknown".into()),
                        review_status_before,
                        review_status_after,
                        runtime_effect,
                        updated_at: reviewed_at.to_string(),
                        refresh_reasoning,
                    },
                )
            })
            .transpose()?,
        _ => None,
    };
    Ok(update)
}

fn adaptive_review_counts(adaptive: &AdaptiveLearningModel) -> serde_json::Value {
    let mut counts = std::collections::BTreeMap::<String, u64>::new();
    for status in adaptive
        .learned_detectors
        .iter()
        .map(|item| item.review_status.as_str())
        .chain(
            adaptive
                .learned_templates
                .iter()
                .map(|item| item.review_status.as_str()),
        )
        .chain(
            adaptive
                .learned_compositions
                .iter()
                .map(|item| item.review_status.as_str()),
        )
        .chain(
            adaptive
                .learned_edge_profiles
                .iter()
                .map(|item| item.review_status.as_str()),
        )
    {
        *counts.entry(status.to_string()).or_default() += 1;
    }
    serde_json::json!(counts)
}

fn adaptive_review_queue(adaptive: &AdaptiveLearningModel) -> Vec<serde_json::Value> {
    let mut queue = adaptive
        .learned_detectors
        .iter()
        .filter(|item| !item.manually_disabled && item.review_status == "unreviewed")
        .map(|item| {
            serde_json::json!({
                "artifact_kind": "detector",
                "artifact_id": item.detector_id,
                "label": item.requirement_name,
                "status": adaptive_artifact_status(item.manually_disabled, item.confirmations, item.false_positives),
                "review_status": item.review_status,
                "confirmations": item.confirmations,
                "false_positives": item.false_positives,
                "updated_at": item.updated_at,
            })
        })
        .chain(adaptive.learned_templates.iter().filter(|item| !item.manually_disabled && item.review_status == "unreviewed").map(|item| {
            serde_json::json!({
                "artifact_kind": "template",
                "artifact_id": item.template_id,
                "label": item.template_name,
                "status": adaptive_artifact_status(item.manually_disabled, item.confirmations, item.false_positives),
                "review_status": item.review_status,
                "confirmations": item.confirmations,
                "false_positives": item.false_positives,
                "updated_at": item.updated_at,
            })
        }))
        .chain(adaptive.learned_compositions.iter().filter(|item| !item.manually_disabled && item.review_status == "unreviewed").map(|item| {
            serde_json::json!({
                "artifact_kind": "composition",
                "artifact_id": item.composition_id,
                "label": item.composition_name,
                "status": adaptive_artifact_status(item.manually_disabled, item.confirmations, item.false_positives),
                "review_status": item.review_status,
                "confirmations": item.confirmations,
                "false_positives": item.false_positives,
                "updated_at": item.updated_at,
            })
        }))
        .chain(adaptive.learned_edge_profiles.iter().filter(|item| !item.manually_disabled && item.review_status == "unreviewed").map(|item| {
            serde_json::json!({
                "artifact_kind": "edge_profile",
                "artifact_id": item.profile_id,
                "label": item.profile_id,
                "status": adaptive_artifact_status(item.manually_disabled, item.confirmations, item.false_positives),
                "review_status": item.review_status,
                "confirmations": item.confirmations,
                "false_positives": item.false_positives,
                "updated_at": item.updated_at,
            })
        }))
        .collect::<Vec<_>>();
    queue.sort_by(|left, right| {
        let left_false_positives = left
            .get("false_positives")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let right_false_positives = right
            .get("false_positives")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        right_false_positives
            .cmp(&left_false_positives)
            .then_with(|| {
                let left_confirmations = left
                    .get("confirmations")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let right_confirmations = right
                    .get("confirmations")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                right_confirmations.cmp(&left_confirmations)
            })
            .then_with(|| {
                let left_label = left
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let right_label = right
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                left_label.cmp(right_label)
            })
    });
    queue
}

fn recent_review_activity(
    audit: &[AdaptiveLearningAuditEntry],
    limit: usize,
) -> Vec<serde_json::Value> {
    audit
        .iter()
        .filter(|entry| entry.review_status_after.is_some())
        .take(limit)
        .map(adaptive_learning_audit_entry_json)
        .collect()
}

fn adaptive_review_comparison_rows(
    adaptive: &AdaptiveLearningModel,
    history_summary: &serde_json::Value,
    active_incident_influence: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let mut history_index =
        std::collections::BTreeMap::<(String, String), serde_json::Value>::new();
    for item in history_summary
        .get("artifacts")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(kind) = item
            .get("artifact_kind")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(artifact_id) = item.get("artifact_id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        history_index.insert((kind.to_string(), artifact_id.to_string()), item.clone());
    }
    let mut influence_index =
        std::collections::BTreeMap::<(String, String), (u64, f64, Vec<String>)>::new();
    for incident in active_incident_influence {
        let incident_id = incident
            .get("incident_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown-incident")
            .to_string();
        for artifact in incident
            .get("learning")
            .and_then(|value| value.get("artifacts"))
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(kind) = artifact.get("kind").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(artifact_id) = artifact
                .get("artifact_id")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let cumulative_impact = artifact
                .get("cumulative_impact")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_default();
            let entry = influence_index
                .entry((kind.to_string(), artifact_id.to_string()))
                .or_insert_with(|| (0, 0.0, Vec::new()));
            entry.0 += 1;
            entry.1 += cumulative_impact;
            if !entry.2.iter().any(|value| value == &incident_id) {
                entry.2.push(incident_id.clone());
            }
        }
    }
    let attention_keys = artifacts_requiring_attention(adaptive)
        .into_iter()
        .filter_map(|item| {
            Some((
                item.get("kind")?.as_str()?.to_string(),
                item.get("detector_id")
                    .or_else(|| item.get("template_id"))
                    .or_else(|| item.get("composition_id"))
                    .or_else(|| item.get("profile_id"))?
                    .as_str()?
                    .to_string(),
            ))
        })
        .collect::<std::collections::BTreeSet<_>>();

    let mut rows = Vec::new();
    for detector in &adaptive.learned_detectors {
        rows.push(adaptive_review_comparison_row(
            "detector",
            &detector.detector_id,
            &detector.requirement_name,
            adaptive_artifact_status(
                detector.manually_disabled,
                detector.confirmations,
                detector.false_positives,
            ),
            &detector.review_status,
            detector.confirmations,
            detector.false_positives,
            detector.manually_disabled,
            detector.updated_at.as_str(),
            detector.last_reviewed_at.as_deref(),
            history_index.get(&("detector".into(), detector.detector_id.clone())),
            influence_index.get(&("detector".into(), detector.detector_id.clone())),
            attention_keys.contains(&("detector".into(), detector.detector_id.clone())),
        ));
    }
    for template in &adaptive.learned_templates {
        rows.push(adaptive_review_comparison_row(
            "template",
            &template.template_id,
            &template.template_name,
            adaptive_artifact_status(
                template.manually_disabled,
                template.confirmations,
                template.false_positives,
            ),
            &template.review_status,
            template.confirmations,
            template.false_positives,
            template.manually_disabled,
            template.updated_at.as_str(),
            template.last_reviewed_at.as_deref(),
            history_index.get(&("template".into(), template.template_id.clone())),
            influence_index.get(&("template".into(), template.template_id.clone())),
            attention_keys.contains(&("template".into(), template.template_id.clone())),
        ));
    }
    for composition in &adaptive.learned_compositions {
        rows.push(adaptive_review_comparison_row(
            "composition",
            &composition.composition_id,
            &composition.composition_name,
            adaptive_artifact_status(
                composition.manually_disabled,
                composition.confirmations,
                composition.false_positives,
            ),
            &composition.review_status,
            composition.confirmations,
            composition.false_positives,
            composition.manually_disabled,
            composition.updated_at.as_str(),
            composition.last_reviewed_at.as_deref(),
            history_index.get(&("composition".into(), composition.composition_id.clone())),
            influence_index.get(&("composition".into(), composition.composition_id.clone())),
            attention_keys.contains(&("composition".into(), composition.composition_id.clone())),
        ));
    }
    for profile in &adaptive.learned_edge_profiles {
        rows.push(adaptive_review_comparison_row(
            "edge_profile",
            &profile.profile_id,
            &profile.profile_id,
            adaptive_artifact_status(
                profile.manually_disabled,
                profile.confirmations,
                profile.false_positives,
            ),
            &profile.review_status,
            profile.confirmations,
            profile.false_positives,
            profile.manually_disabled,
            profile.updated_at.as_str(),
            profile.last_reviewed_at.as_deref(),
            history_index.get(&("edge_profile".into(), profile.profile_id.clone())),
            influence_index.get(&("edge_profile".into(), profile.profile_id.clone())),
            attention_keys.contains(&("edge_profile".into(), profile.profile_id.clone())),
        ));
    }
    rows.sort_by(|left, right| {
        let left_impact = left
            .get("active_incident_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let right_impact = right
            .get("active_incident_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        right_impact
            .cmp(&left_impact)
            .then_with(|| {
                let left_confirmations = left
                    .get("confirmations")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                let right_confirmations = right
                    .get("confirmations")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                right_confirmations.cmp(&left_confirmations)
            })
            .then_with(|| {
                let left_label = left
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let right_label = right
                    .get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                left_label.cmp(right_label)
            })
    });
    rows
}

#[allow(clippy::too_many_arguments)]
fn adaptive_review_comparison_row(
    artifact_kind: &str,
    artifact_id: &str,
    label: &str,
    status: &str,
    review_status: &str,
    confirmations: u64,
    false_positives: u64,
    manually_disabled: bool,
    updated_at: &str,
    last_reviewed_at: Option<&str>,
    history: Option<&serde_json::Value>,
    influence: Option<&(u64, f64, Vec<String>)>,
    attention: bool,
) -> serde_json::Value {
    let noise_ratio = if confirmations + false_positives == 0 {
        0.0
    } else {
        false_positives as f64 / (confirmations + false_positives) as f64
    };
    let (active_incident_count, active_cumulative_impact, incident_ids) = influence
        .map(|value| (value.0, value.1, value.2.clone()))
        .unwrap_or((0, 0.0, Vec::new()));
    let pending_review_age_hours = if review_status == "unreviewed" {
        hours_since_iso(updated_at)
    } else {
        None
    };
    let watch_age_hours = if review_status == "watch" {
        last_reviewed_at.and_then(hours_since_iso)
    } else {
        None
    };
    serde_json::json!({
        "artifact_kind": artifact_kind,
        "artifact_id": artifact_id,
        "label": label,
        "status": status,
        "review_status": review_status,
        "confirmations": confirmations,
        "false_positives": false_positives,
        "noise_ratio": noise_ratio,
        "manually_disabled": manually_disabled,
        "attention": attention,
        "updated_at": updated_at,
        "last_reviewed_at": last_reviewed_at,
        "pending_review_age_hours": pending_review_age_hours,
        "watch_age_hours": watch_age_hours,
        "aging_bucket": review_aging_bucket(review_status, pending_review_age_hours, watch_age_hours),
        "history_observations": history
            .and_then(|value| value.get("observations"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        "latest_estimated_impact": history
            .and_then(|value| value.get("latest_estimated_impact"))
            .and_then(serde_json::Value::as_f64),
        "latest_score": history
            .and_then(|value| value.get("latest_score"))
            .and_then(serde_json::Value::as_f64),
        "best_rank": history
            .and_then(|value| value.get("best_rank"))
            .and_then(serde_json::Value::as_i64),
        "cumulative_score_delta": history
            .and_then(|value| value.get("cumulative_score_delta"))
            .and_then(serde_json::Value::as_f64),
        "cumulative_edge_delta": history
            .and_then(|value| value.get("cumulative_edge_delta"))
            .and_then(serde_json::Value::as_f64),
        "active_incident_count": active_incident_count,
        "active_cumulative_impact": active_cumulative_impact,
        "incident_ids": incident_ids,
    })
}

fn adaptive_review_analytics(comparison_rows: &[serde_json::Value]) -> serde_json::Value {
    let mut by_kind = std::collections::BTreeMap::<String, (u64, u64, u64, u64)>::new();
    for row in comparison_rows {
        let Some(kind) = row.get("artifact_kind").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let entry = by_kind.entry(kind.to_string()).or_default();
        entry.0 += 1;
        if row.get("review_status").and_then(serde_json::Value::as_str) == Some("unreviewed") {
            entry.1 += 1;
        }
        if row
            .get("attention")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            entry.2 += 1;
        }
        if row
            .get("manually_disabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            entry.3 += 1;
        }
    }
    let kind_breakdown = by_kind
        .into_iter()
        .map(
            |(artifact_kind, (total, unreviewed, attention, manually_disabled))| {
                serde_json::json!({
                    "artifact_kind": artifact_kind,
                    "total": total,
                    "unreviewed": unreviewed,
                    "attention": attention,
                    "manually_disabled": manually_disabled,
                })
            },
        )
        .collect::<Vec<_>>();
    serde_json::json!({
        "kind_breakdown": kind_breakdown,
        "top_confirmed": top_adaptive_rows_by(comparison_rows, |row| {
            row.get("confirmations").and_then(serde_json::Value::as_u64).unwrap_or(0) as f64
        }),
        "top_noisy": top_adaptive_rows_by(comparison_rows, |row| {
            row.get("false_positives").and_then(serde_json::Value::as_u64).unwrap_or(0) as f64
                + row.get("noise_ratio").and_then(serde_json::Value::as_f64).unwrap_or(0.0)
        }),
        "top_impact": top_adaptive_rows_by(comparison_rows, |row| {
            row.get("active_cumulative_impact").and_then(serde_json::Value::as_f64).unwrap_or(0.0)
                + row.get("latest_estimated_impact").and_then(serde_json::Value::as_f64).unwrap_or(0.0)
        }),
        "recently_changed": top_adaptive_rows_by(comparison_rows, |row| {
            row.get("updated_at")
                .and_then(serde_json::Value::as_str)
                .and_then(parse_rfc3339)
                .map(|value| value.unix_timestamp_nanos() as f64)
                .unwrap_or(0.0)
        }),
    })
}

fn top_adaptive_rows_by<F>(rows: &[serde_json::Value], score: F) -> Vec<serde_json::Value>
where
    F: Fn(&serde_json::Value) -> f64,
{
    let mut ranked = rows.to_vec();
    ranked.sort_by(|left, right| {
        score(right)
            .partial_cmp(&score(left))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.into_iter().take(5).collect()
}

fn adaptive_review_saved_views(
    incidents: &IncidentsStore,
    comparison_rows: &[serde_json::Value],
) -> Vec<serde_json::Value> {
    let Ok(views) = incidents.list_adaptive_review_views() else {
        return Vec::new();
    };
    views
        .into_iter()
        .map(|view| adaptive_review_saved_view_json(&view, comparison_rows))
        .collect()
}

fn adaptive_review_saved_view_json(
    view: &StoredAdaptiveReviewView,
    comparison_rows: &[serde_json::Value],
) -> serde_json::Value {
    let matches = comparison_rows
        .iter()
        .filter(|row| adaptive_row_matches_saved_view(row, view))
        .cloned()
        .collect::<Vec<_>>();
    let pending_review_count = matches
        .iter()
        .filter(|row| {
            row.get("review_status").and_then(serde_json::Value::as_str) == Some("unreviewed")
        })
        .count();
    let attention_count = matches
        .iter()
        .filter(|row| {
            row.get("attention")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    let active_incident_count = matches
        .iter()
        .map(|row| {
            row.get("active_incident_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        })
        .sum::<u64>();
    let active_cumulative_impact = matches
        .iter()
        .map(|row| {
            row.get("active_cumulative_impact")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0)
        })
        .sum::<f64>();
    let oldest_pending = matches
        .iter()
        .filter_map(|row| {
            Some((
                row.get("pending_review_age_hours")?.as_f64()?,
                row.get("label")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown artifact"),
            ))
        })
        .max_by(|left, right| {
            left.0
                .partial_cmp(&right.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    serde_json::json!({
        "view_id": view.view_id,
        "name": view.name,
        "description": view.description,
        "search_text": view.search_text,
        "assigned_reviewer": view.assigned_reviewer,
        "created_at": view.created_at,
        "updated_at": view.updated_at,
        "last_used_at": view.last_used_at,
        "artifact_selections": view.artifact_selections.iter().map(|selection| serde_json::json!({
            "artifact_kind": selection.artifact_kind,
            "artifact_id": selection.artifact_id,
        })).collect::<Vec<_>>(),
        "match_count": matches.len(),
        "pending_review_count": pending_review_count,
        "attention_count": attention_count,
        "active_incident_count": active_incident_count,
        "active_cumulative_impact": active_cumulative_impact,
        "oldest_pending_age_hours": oldest_pending.map(|value| value.0),
        "oldest_pending_label": oldest_pending.map(|value| value.1.to_string()),
        "aging_bucket": oldest_pending
            .map(|value| saved_view_aging_bucket(value.0))
            .unwrap_or("fresh"),
        "stale_pending": oldest_pending.map(|value| value.0 >= 72.0).unwrap_or(false),
    })
}

fn adaptive_row_matches_saved_view(
    row: &serde_json::Value,
    view: &StoredAdaptiveReviewView,
) -> bool {
    let key_matches = if view.artifact_selections.is_empty() {
        true
    } else {
        let row_kind = row
            .get("artifact_kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let row_id = row
            .get("artifact_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        view.artifact_selections
            .iter()
            .any(|selection| selection.artifact_kind == row_kind && selection.artifact_id == row_id)
    };
    if !key_matches {
        return false;
    }
    let Some(search) = view
        .search_text
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    else {
        return true;
    };
    let needle = search.to_ascii_lowercase();
    [
        row.get("label")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        row.get("artifact_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        row.get("artifact_kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        row.get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        row.get("review_status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
    ]
    .join(" ")
    .to_ascii_lowercase()
    .contains(&needle)
}

fn adaptive_learning_trend_drilldowns(
    incidents: &IncidentsStore,
    limit: usize,
    per_artifact_limit: usize,
) -> Result<Vec<serde_json::Value>> {
    let mut by_artifact =
        std::collections::BTreeMap::<(String, String), Vec<AdaptiveLearningHistoryEntry>>::new();
    for entry in read_adaptive_learning_history(incidents, limit)? {
        by_artifact
            .entry((entry.artifact_kind.clone(), entry.artifact_id.clone()))
            .or_default()
            .push(entry);
    }
    let mut drilldowns = by_artifact
        .into_iter()
        .map(|((artifact_kind, artifact_id), entries)| {
            let label = entries
                .last()
                .map(|entry| entry.artifact_label.clone())
                .unwrap_or_else(|| artifact_id.clone());
            let observations = entries
                .iter()
                .rev()
                .take(per_artifact_limit.max(1))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|entry| {
                    serde_json::json!({
                        "observed_at": entry.observed_at,
                        "incident_id": entry.incident_id,
                        "hypothesis_id": entry.hypothesis_id,
                        "score": entry.score,
                        "rank": entry.rank,
                        "estimated_impact": entry.estimated_impact,
                        "impact_metric": entry.impact_metric,
                        "score_delta": entry.score_delta,
                        "rank_delta": entry.rank_delta,
                        "edge_delta": entry.edge_delta,
                    })
                })
                .collect::<Vec<_>>();
            let total_abs_delta = entries
                .iter()
                .map(|entry| {
                    entry.score_delta.unwrap_or_default().abs()
                        + entry.edge_delta.unwrap_or_default().abs()
                })
                .sum::<f64>();
            serde_json::json!({
                "artifact_kind": artifact_kind,
                "artifact_id": artifact_id,
                "artifact_label": label,
                "observation_count": entries.len(),
                "total_abs_delta": total_abs_delta,
                "observations": observations,
            })
        })
        .collect::<Vec<_>>();
    drilldowns.sort_by(|left, right| {
        right
            .get("total_abs_delta")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default()
            .partial_cmp(
                &left
                    .get("total_abs_delta")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or_default(),
            )
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(drilldowns)
}

fn review_aging_bucket(
    review_status: &str,
    pending_review_age_hours: Option<f64>,
    watch_age_hours: Option<f64>,
) -> &'static str {
    match review_status {
        "unreviewed" => pending_review_age_hours
            .map(saved_view_aging_bucket)
            .unwrap_or("fresh"),
        "watch" => match watch_age_hours.unwrap_or_default() {
            age if age >= 168.0 => "aged",
            age if age >= 72.0 => "stale",
            _ => "fresh",
        },
        _ => "fresh",
    }
}

fn saved_view_aging_bucket(age_hours: f64) -> &'static str {
    if age_hours >= 168.0 {
        "aged"
    } else if age_hours >= 72.0 {
        "stale"
    } else if age_hours >= 24.0 {
        "warm"
    } else {
        "fresh"
    }
}

fn hours_since_iso(raw: &str) -> Option<f64> {
    let parsed = parse_rfc3339(raw)?;
    let now = time::OffsetDateTime::now_utc();
    Some((now - parsed).whole_seconds().max(0) as f64 / 3600.0)
}

fn resolve_runtime_sidecar_path(raw: &str, incidents_db: &Path) -> PathBuf {
    let candidate = PathBuf::from(raw);
    if candidate.is_absolute() {
        return candidate;
    }
    let data_dir = incidents_db.parent().unwrap_or_else(|| Path::new("."));
    let normalized = raw.trim().replace('\\', "/");
    let trimmed = normalized.trim_start_matches("./");
    if let Some(relative_to_data) = trimmed.strip_prefix("data/") {
        return data_dir.join(relative_to_data);
    }
    data_dir.join(candidate)
}

fn archive_db_path(incidents_db: &Path, archived_at: &str) -> PathBuf {
    let stamp = archived_at
        .split('T')
        .next()
        .unwrap_or("1970-01-01")
        .replace('-', "");
    incidents_db
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("archive")
        .join(format!("incidents_{stamp}.db"))
}

fn iso_time_before_seconds(seconds: i64) -> String {
    let target = time::OffsetDateTime::now_utc() - time::Duration::seconds(seconds.max(0));
    target
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

fn load_json_file<T>(path: &Path) -> Result<Option<T>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path)
        .map_err(|error| anyhow::anyhow!("read {}: {error}", path.display()))?;
    let parsed = serde_json::from_str::<T>(&raw)
        .map_err(|error| anyhow::anyhow!("parse {}: {error}", path.display()))?;
    Ok(Some(parsed))
}

fn write_json_file<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| anyhow::anyhow!("create {}: {error}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(value)?;
    std::fs::write(path, raw)
        .map_err(|error| anyhow::anyhow!("write {}: {error}", path.display()))?;
    Ok(())
}

fn apply_feedback_to_calibration(
    config: &TomlValue,
    calibration: &mut CalibrationModel,
    feedback: &inferra_storage::StoredFeedback,
    hypotheses: &[serde_json::Value],
) {
    if calibration
        .processed_feedback_ids
        .iter()
        .any(|item| item == &feedback.feedback_id)
    {
        return;
    }
    calibration
        .processed_feedback_ids
        .push(feedback.feedback_id.clone());
    if feedback.feedback_type == "skipped" {
        return;
    }
    calibration.total_feedback_count += 1;
    let min_samples = calibration_min_samples(config);
    for hypothesis in hypotheses {
        let score = hypothesis
            .get("total_score")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default();
        let bucket_index = calibration_bucket_index(&calibration.buckets, score);
        if let Some(bucket) = calibration.buckets.get_mut(bucket_index) {
            bucket.total_predictions += 1;
            if feedback.feedback_type == "confirmed"
                && hypothesis
                    .get("hypothesis_id")
                    .and_then(serde_json::Value::as_str)
                    == feedback.correct_hypothesis_id.as_deref()
            {
                bucket.correct_predictions += 1;
            }
            bucket.accuracy =
                bucket.correct_predictions as f64 / bucket.total_predictions.max(1) as f64;
            bucket.sample_confidence = if bucket.total_predictions >= min_samples {
                "sufficient"
            } else {
                "insufficient"
            }
            .into();
        }
    }
    let total_correct: u64 = calibration
        .buckets
        .iter()
        .map(|bucket| bucket.correct_predictions)
        .sum();
    let total_predictions: u64 = calibration
        .buckets
        .iter()
        .map(|bucket| bucket.total_predictions)
        .sum();
    calibration.overall_accuracy = total_correct as f64 / total_predictions.max(1) as f64;
    calibration.last_updated = Some(
        feedback
            .created_at
            .clone()
            .unwrap_or_else(|| feedback.resolved_at.clone()),
    );
}

fn apply_feedback_to_weights(
    config: &TomlValue,
    weights: &mut LearnedScoringWeights,
    feedback: &inferra_storage::StoredFeedback,
    hypotheses: &[serde_json::Value],
) {
    if weights
        .processed_feedback_ids
        .iter()
        .any(|item| item == &feedback.feedback_id)
    {
        return;
    }
    weights
        .processed_feedback_ids
        .push(feedback.feedback_id.clone());
    if hypotheses.is_empty() || feedback.feedback_type == "skipped" {
        return;
    }
    let learning_rate = scoring_learning_rate(config);
    let max_drift = scoring_max_drift(config);
    let min_weight = scoring_min_weight(config);
    let defaults = if weights.default_weights.is_empty() {
        scoring_weight_defaults()
    } else {
        weights.default_weights.clone()
    };
    if weights.effective_weights.is_empty() {
        weights.effective_weights = defaults.clone();
    }
    let mut adjustments = std::collections::BTreeMap::new();
    match feedback.feedback_type.as_str() {
        "confirmed" => {
            let Some(correct) = hypotheses.iter().find(|item| {
                item.get("hypothesis_id")
                    .and_then(serde_json::Value::as_str)
                    == feedback.correct_hypothesis_id.as_deref()
            }) else {
                return;
            };
            let competitors = hypotheses
                .iter()
                .filter(|item| {
                    item.get("hypothesis_id")
                        .and_then(serde_json::Value::as_str)
                        != feedback.correct_hypothesis_id.as_deref()
                })
                .collect::<Vec<_>>();
            for component in scoring_component_names() {
                let correct_value = score_breakdown_component(correct, component);
                let competitor_average = if competitors.is_empty() {
                    0.0
                } else {
                    competitors
                        .iter()
                        .map(|item| score_breakdown_component(item, component))
                        .sum::<f64>()
                        / competitors.len() as f64
                };
                let delta = (correct_value - competitor_average) * learning_rate;
                *weights
                    .effective_weights
                    .entry(component.to_string())
                    .or_insert(*defaults.get(component).unwrap_or(&min_weight)) += delta;
                adjustments.insert(component.to_string(), delta);
            }
        }
        "none_correct" => {
            let top = &hypotheses[0];
            for component in scoring_component_names() {
                let delta = -score_breakdown_component(top, component) * learning_rate * 0.5;
                *weights
                    .effective_weights
                    .entry(component.to_string())
                    .or_insert(*defaults.get(component).unwrap_or(&min_weight)) += delta;
                adjustments.insert(component.to_string(), delta);
            }
        }
        _ => {}
    }
    clamp_learned_weights(
        &mut weights.effective_weights,
        &defaults,
        max_drift,
        min_weight,
    );
    renormalize_weights(&mut weights.effective_weights, defaults.values().sum());
    weights.last_updated = Some(
        feedback
            .created_at
            .clone()
            .unwrap_or_else(|| feedback.resolved_at.clone()),
    );
    if !adjustments.is_empty() {
        weights.audit.push(WeightAdjustmentAudit {
            feedback_id: feedback.feedback_id.clone(),
            incident_id: feedback.incident_id.clone(),
            applied_at: feedback
                .created_at
                .clone()
                .unwrap_or_else(|| feedback.resolved_at.clone()),
            adjustments,
        });
    }
}

fn apply_feedback_to_adaptive_learning(
    adaptive: &mut AdaptiveLearningModel,
    feedback: &inferra_storage::StoredFeedback,
    hypotheses: &[serde_json::Value],
    incidents: &IncidentsStore,
    events: Option<&EventsStore>,
) -> Result<()> {
    if adaptive
        .processed_feedback_ids
        .iter()
        .any(|item| item == &feedback.feedback_id)
    {
        return Ok(());
    }
    adaptive
        .processed_feedback_ids
        .push(feedback.feedback_id.clone());
    let Some(events) = events else {
        return Ok(());
    };
    let incident_event_ids = incidents
        .incident_event_ids(&feedback.incident_id)
        .unwrap_or_default();
    let incident_events = events.get_events(&incident_event_ids).unwrap_or_default();
    let incident_graph = incidents
        .inference_graph_snapshot(&feedback.incident_id)
        .ok()
        .flatten()
        .and_then(|value| value.get("graph_data").cloned())
        .and_then(|value| serde_json::from_value::<InferenceGraph>(value).ok())
        .unwrap_or_default();
    match feedback.feedback_type.as_str() {
        "confirmed" => {
            let Some(correct) = hypotheses.iter().find(|item| {
                item.get("hypothesis_id")
                    .and_then(serde_json::Value::as_str)
                    == feedback.correct_hypothesis_id.as_deref()
            }) else {
                return Ok(());
            };
            let supporting_event_ids =
                parse_json_string_array_value(correct.get("supporting_events"));
            let supporting_events = if supporting_event_ids.is_empty() {
                incident_events.clone()
            } else {
                events
                    .get_events(&supporting_event_ids)
                    .unwrap_or_else(|_| incident_events.clone())
            };
            if supporting_events.is_empty() {
                return Ok(());
            }
            let profile = learned_pattern_profile(correct, &supporting_events);
            if profile.positive_terms.is_empty() && profile.tags.is_empty() {
                return Ok(());
            }
            upsert_learned_detector(adaptive, &profile, feedback);
            upsert_learned_template(adaptive, &profile, feedback);
            if let Some(composition) = learned_composition_profile(
                adaptive,
                correct,
                &profile,
                &supporting_events,
                &incident_graph,
            ) {
                upsert_learned_composition(adaptive, &composition, feedback);
            }
            upsert_learned_edge_profiles(
                adaptive,
                &incident_graph,
                &profile.cause_type,
                &supporting_event_ids,
                feedback,
            );
            adaptive.last_updated = Some(
                feedback
                    .created_at
                    .clone()
                    .unwrap_or_else(|| feedback.resolved_at.clone()),
            );
        }
        "none_correct" => {
            if incident_events.is_empty() {
                return Ok(());
            }
            mark_learned_false_positives(adaptive, &incident_events, &incident_graph);
            adaptive.last_updated = Some(
                feedback
                    .created_at
                    .clone()
                    .unwrap_or_else(|| feedback.resolved_at.clone()),
            );
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct LearnedPatternProfile {
    requirement_name: String,
    detector_id: String,
    template_id: String,
    template_name: String,
    cause_type: String,
    cause_subtype: Option<String>,
    title_template: String,
    confidence: f64,
    positive_terms: Vec<String>,
    tags: Vec<String>,
    source_types: Vec<String>,
    min_severity: Option<i64>,
    requires: Vec<String>,
    requires_same_service: bool,
    requires_temporal_order: bool,
}

fn learned_pattern_profile(
    hypothesis: &serde_json::Value,
    supporting_events: &[EventRow],
) -> LearnedPatternProfile {
    let cause_type = hypothesis
        .get("cause_type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let description = hypothesis
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Learned hypothesis pattern")
        .trim()
        .to_string();
    let confidence = hypothesis
        .get("total_score")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.65)
        .clamp(0.1, 0.95);
    let positive_terms = stable_learning_terms(supporting_events);
    let tags = stable_learning_tags(supporting_events);
    let source_types = stable_learning_source_types(supporting_events);
    let primary_term = positive_terms
        .first()
        .cloned()
        .or_else(|| tags.first().cloned())
        .or_else(|| source_types.first().cloned())
        .unwrap_or_else(|| slug(&cause_type));
    let detector_key = format!("{}-{}", cause_type, primary_term);
    let requirement_name = format!("learned_{}", slug(&detector_key));
    let builtin_requirement = built_in_requirement_for_cause(&cause_type, supporting_events);
    let mut requires = vec![requirement_name.clone()];
    if let Some(builtin) = builtin_requirement {
        requires.push(builtin);
    }
    requires.sort();
    requires.dedup();
    LearnedPatternProfile {
        detector_id: requirement_name.clone(),
        requirement_name: requirement_name.clone(),
        template_id: format!("template_{}", requirement_name),
        template_name: format!("learned_{}", slug(&detector_key)),
        cause_type,
        cause_subtype: Some(slug(&primary_term)),
        title_template: description,
        confidence,
        positive_terms,
        tags,
        source_types,
        min_severity: supporting_events.iter().filter_map(event_severity).min(),
        requires,
        requires_same_service: services_in_events(supporting_events).len() <= 1,
        requires_temporal_order: supporting_events.len() > 1,
    }
}

fn learned_composition_profile(
    adaptive: &AdaptiveLearningModel,
    hypothesis: &serde_json::Value,
    profile: &LearnedPatternProfile,
    supporting_events: &[EventRow],
    incident_graph: &InferenceGraph,
) -> Option<LearnedComposition> {
    let mut requires = matched_requirement_names(supporting_events, adaptive);
    merge_unique(&mut requires, &profile.requires, 6);
    requires.sort();
    requires.dedup();
    if requires.len() < 2 {
        return None;
    }
    let preferred_edge_types = preferred_edge_types_for_support(incident_graph, supporting_events);
    let composition_key = format!("{}-{}", profile.cause_type, requires.join("-"));
    Some(LearnedComposition {
        composition_id: format!("composition_{}", slug(&composition_key)),
        composition_name: format!("learned_{}", slug(&composition_key)),
        cause_type: profile.cause_type.clone(),
        cause_subtype: profile.cause_subtype.clone(),
        title_template: hypothesis
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&profile.title_template)
            .trim()
            .to_string(),
        confidence: hypothesis
            .get("total_score")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(profile.confidence)
            .clamp(0.1, 0.95),
        requires,
        requires_same_service: services_in_events(supporting_events).len() <= 1,
        requires_temporal_order: supporting_events.len() > 1,
        preferred_edge_types,
        confirmations: 0,
        false_positives: 0,
        created_from_feedback_id: String::new(),
        updated_at: String::new(),
        manually_disabled: false,
        status_reason: None,
        review_status: default_review_status(),
        review_reason: None,
        last_reviewed_at: None,
    })
}

fn matched_requirement_names(events: &[EventRow], adaptive: &AdaptiveLearningModel) -> Vec<String> {
    let learning = LearningArtifacts {
        adaptive: adaptive.clone(),
        ..LearningArtifacts::default()
    };
    let mut names = built_in_requirement_names()
        .into_iter()
        .filter(|requirement| !event_ids_for_requirement(events, requirement, &learning).is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for detector in adaptive
        .learned_detectors
        .iter()
        .filter(|detector| detector_is_active(detector))
    {
        if !event_ids_for_requirement(events, &detector.requirement_name, &learning).is_empty() {
            names.push(detector.requirement_name.clone());
        }
    }
    names.sort();
    names.dedup();
    names
}

fn built_in_requirement_names() -> [&'static str; 6] {
    [
        "connection_failures_outbound",
        "error_spike",
        "resource_pressure",
        "restart_loop",
        "deployment_event",
        "config_change_event",
    ]
}

fn preferred_edge_types_for_support(
    incident_graph: &InferenceGraph,
    supporting_events: &[EventRow],
) -> Vec<String> {
    let support_ids = supporting_events
        .iter()
        .filter_map(|event| event.event_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut types = incident_graph
        .edges
        .iter()
        .filter(|edge| {
            support_ids.contains(&edge.source_event_id)
                || support_ids.contains(&edge.target_event_id)
        })
        .map(|edge| edge.edge_type.clone())
        .collect::<Vec<_>>();
    types.sort();
    types.dedup();
    types.truncate(4);
    types
}

fn stable_learning_terms(events: &[EventRow]) -> Vec<String> {
    let min_occurrences = (events.len() / 2).max(1);
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for event in events {
        let tokens = tokenize_learning_text(&event_signal_text(event));
        for token in tokens {
            *counts.entry(token).or_default() += 1;
        }
    }
    let mut stable = counts
        .into_iter()
        .filter(|(_, count)| *count >= min_occurrences)
        .collect::<Vec<_>>();
    stable.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    stable.into_iter().map(|(token, _)| token).take(6).collect()
}

fn stable_learning_tags(events: &[EventRow]) -> Vec<String> {
    let min_occurrences = (events.len() / 2).max(1);
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for event in events {
        for tag in event.tags.clone().unwrap_or_default() {
            *counts.entry(tag.to_ascii_lowercase()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count >= min_occurrences)
        .map(|(tag, _)| tag)
        .take(6)
        .collect()
}

fn stable_learning_source_types(events: &[EventRow]) -> Vec<String> {
    let min_occurrences = (events.len() / 2).max(1);
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for event in events {
        if let Some(source_type) = event
            .source_ref
            .as_ref()
            .and_then(|source| source.source_type.clone())
        {
            *counts.entry(source_type.to_ascii_lowercase()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count >= min_occurrences)
        .map(|(source_type, _)| source_type)
        .take(4)
        .collect()
}

fn tokenize_learning_text(text: &str) -> std::collections::BTreeSet<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token.len() >= 4)
        .filter(|token| !token.chars().all(|ch| ch.is_ascii_digit()))
        .filter(|token| !learning_stopwords().contains(&token.as_str()))
        .collect()
}

fn learning_stopwords() -> &'static std::collections::BTreeSet<&'static str> {
    static STOPWORDS: std::sync::OnceLock<std::collections::BTreeSet<&'static str>> =
        std::sync::OnceLock::new();
    STOPWORDS.get_or_init(|| {
        std::collections::BTreeSet::from([
            "after", "before", "calling", "critical", "error", "failed", "from", "host", "into",
            "local", "process", "runtime", "service", "started", "state", "then", "with", "worker",
        ])
    })
}

fn built_in_requirement_for_cause(cause_type: &str, events: &[EventRow]) -> Option<String> {
    match cause_type {
        "dependency_failure" => Some("connection_failures_outbound".into()),
        "resource_pressure" => Some("resource_pressure".into()),
        "service_instability" => Some("restart_loop".into()),
        "configuration_change" => {
            if events.iter().any(|event| {
                has_any_term(
                    &event_signal_text(event),
                    &["deploy", "deployment", "rollout", "release"],
                )
            }) {
                Some("deployment_event".into())
            } else {
                Some("config_change_event".into())
            }
        }
        _ => None,
    }
}

fn upsert_learned_detector(
    adaptive: &mut AdaptiveLearningModel,
    profile: &LearnedPatternProfile,
    feedback: &inferra_storage::StoredFeedback,
) {
    if let Some(existing) = adaptive
        .learned_detectors
        .iter_mut()
        .find(|detector| detector.requirement_name == profile.requirement_name)
    {
        merge_unique(&mut existing.positive_terms, &profile.positive_terms, 8);
        merge_unique(&mut existing.tags, &profile.tags, 8);
        merge_unique(&mut existing.source_types, &profile.source_types, 6);
        existing.min_severity = match (existing.min_severity, profile.min_severity) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (left, right) => left.or(right),
        };
        existing.confirmations += 1;
        existing.updated_at = feedback
            .created_at
            .clone()
            .unwrap_or_else(|| feedback.resolved_at.clone());
    } else {
        adaptive.learned_detectors.push(LearnedDetector {
            detector_id: profile.detector_id.clone(),
            requirement_name: profile.requirement_name.clone(),
            cause_type: profile.cause_type.clone(),
            positive_terms: profile.positive_terms.clone(),
            tags: profile.tags.clone(),
            source_types: profile.source_types.clone(),
            min_severity: profile.min_severity,
            confirmations: 1,
            false_positives: 0,
            created_from_feedback_id: feedback.feedback_id.clone(),
            updated_at: feedback
                .created_at
                .clone()
                .unwrap_or_else(|| feedback.resolved_at.clone()),
            manually_disabled: false,
            status_reason: None,
            review_status: default_review_status(),
            review_reason: None,
            last_reviewed_at: None,
        });
    }
}

fn upsert_learned_template(
    adaptive: &mut AdaptiveLearningModel,
    profile: &LearnedPatternProfile,
    feedback: &inferra_storage::StoredFeedback,
) {
    if let Some(existing) = adaptive
        .learned_templates
        .iter_mut()
        .find(|template| template.template_id == profile.template_id)
    {
        merge_unique(&mut existing.requires, &profile.requires, 4);
        existing.confidence = ((existing.confidence + profile.confidence) / 2.0).clamp(0.1, 0.95);
        existing.confirmations += 1;
        existing.updated_at = feedback
            .created_at
            .clone()
            .unwrap_or_else(|| feedback.resolved_at.clone());
    } else {
        adaptive.learned_templates.push(LearnedTemplate {
            template_id: profile.template_id.clone(),
            template_name: profile.template_name.clone(),
            cause_type: profile.cause_type.clone(),
            cause_subtype: profile.cause_subtype.clone(),
            title_template: profile.title_template.clone(),
            confidence: profile.confidence,
            requires: profile.requires.clone(),
            requires_same_service: profile.requires_same_service,
            requires_temporal_order: profile.requires_temporal_order,
            confirmations: 1,
            false_positives: 0,
            created_from_feedback_id: feedback.feedback_id.clone(),
            updated_at: feedback
                .created_at
                .clone()
                .unwrap_or_else(|| feedback.resolved_at.clone()),
            manually_disabled: false,
            status_reason: None,
            review_status: default_review_status(),
            review_reason: None,
            last_reviewed_at: None,
        });
    }
}

fn upsert_learned_composition(
    adaptive: &mut AdaptiveLearningModel,
    composition: &LearnedComposition,
    feedback: &inferra_storage::StoredFeedback,
) {
    if let Some(existing) = adaptive
        .learned_compositions
        .iter_mut()
        .find(|item| item.composition_id == composition.composition_id)
    {
        merge_unique(&mut existing.requires, &composition.requires, 6);
        merge_unique(
            &mut existing.preferred_edge_types,
            &composition.preferred_edge_types,
            6,
        );
        existing.confidence =
            ((existing.confidence + composition.confidence) / 2.0).clamp(0.1, 0.95);
        existing.confirmations += 1;
        existing.updated_at = feedback
            .created_at
            .clone()
            .unwrap_or_else(|| feedback.resolved_at.clone());
    } else {
        adaptive.learned_compositions.push(LearnedComposition {
            confirmations: 1,
            created_from_feedback_id: feedback.feedback_id.clone(),
            updated_at: feedback
                .created_at
                .clone()
                .unwrap_or_else(|| feedback.resolved_at.clone()),
            manually_disabled: false,
            status_reason: None,
            review_status: default_review_status(),
            review_reason: None,
            last_reviewed_at: None,
            ..composition.clone()
        });
    }
}

fn upsert_learned_edge_profiles(
    adaptive: &mut AdaptiveLearningModel,
    incident_graph: &InferenceGraph,
    cause_type: &str,
    supporting_event_ids: &[String],
    feedback: &inferra_storage::StoredFeedback,
) {
    let support_ids = supporting_event_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let candidate_edges = incident_graph
        .edges
        .iter()
        .filter(|edge| {
            support_ids.is_empty()
                || support_ids.contains(&edge.source_event_id)
                || support_ids.contains(&edge.target_event_id)
                || cause_type_for_edge(&edge.edge_type) == cause_type
        })
        .take(12)
        .cloned()
        .collect::<Vec<_>>();
    for edge in candidate_edges {
        let source_service =
            graph_node(incident_graph, &edge.source_event_id).map(|node| node.service_id.clone());
        let target_service =
            graph_node(incident_graph, &edge.target_event_id).map(|node| node.service_id.clone());
        let profile_id = format!(
            "edge_{}_{}_{}_{}",
            edge.edge_type,
            source_service.as_deref().unwrap_or("any"),
            target_service.as_deref().unwrap_or("any"),
            cause_type
        );
        if let Some(existing) = adaptive.learned_edge_profiles.iter_mut().find(|profile| {
            profile.edge_type == edge.edge_type
                && profile.source_service == source_service
                && profile.target_service == target_service
                && profile.cause_type.as_deref() == Some(cause_type)
        }) {
            existing.confirmations += 1;
            let count = existing.confirmations as f64;
            existing.average_plausibility =
                ((existing.average_plausibility * (count - 1.0)) + edge.plausibility) / count;
            existing.average_latency_ms =
                ((existing.average_latency_ms * (count - 1.0)) + edge.latency_ms) / count;
            existing.updated_at = feedback
                .created_at
                .clone()
                .unwrap_or_else(|| feedback.resolved_at.clone());
        } else {
            adaptive.learned_edge_profiles.push(LearnedEdgeProfile {
                profile_id: slug(&profile_id),
                edge_type: edge.edge_type.clone(),
                source_service,
                target_service,
                cause_type: Some(cause_type.to_string()),
                confirmations: 1,
                false_positives: 0,
                average_plausibility: edge.plausibility,
                average_latency_ms: edge.latency_ms,
                created_from_feedback_id: feedback.feedback_id.clone(),
                updated_at: feedback
                    .created_at
                    .clone()
                    .unwrap_or_else(|| feedback.resolved_at.clone()),
                manually_disabled: false,
                status_reason: None,
                review_status: default_review_status(),
                review_reason: None,
                last_reviewed_at: None,
            });
        }
    }
}

fn mark_learned_false_positives(
    adaptive: &mut AdaptiveLearningModel,
    incident_events: &[EventRow],
    incident_graph: &InferenceGraph,
) {
    let learning = LearningArtifacts {
        adaptive: adaptive.clone(),
        ..LearningArtifacts::default()
    };
    for template in adaptive.learned_templates.iter_mut() {
        if template.requires.is_empty() || !template_is_active(template) {
            continue;
        }
        let all_requirements_match = template.requires.iter().all(|requirement| {
            !event_ids_for_requirement(incident_events, requirement, &learning).is_empty()
        });
        if all_requirements_match {
            template.false_positives += 1;
            template.updated_at = now_iso();
            for requirement in &template.requires {
                if let Some(detector) = adaptive
                    .learned_detectors
                    .iter_mut()
                    .find(|detector| detector.requirement_name == *requirement)
                {
                    detector.false_positives += 1;
                    detector.updated_at = now_iso();
                }
            }
        }
    }
    let incident_event_ids = incident_events
        .iter()
        .filter_map(|event| event.event_id.clone())
        .collect::<Vec<_>>();
    for composition in adaptive.learned_compositions.iter_mut() {
        if composition.requires.is_empty() || !composition_is_active(composition) {
            continue;
        }
        let all_requirements_match = composition.requires.iter().all(|requirement| {
            !event_ids_for_requirement(incident_events, requirement, &learning).is_empty()
        });
        let graph_support = learned_rule_graph_support(
            incident_graph,
            &incident_event_ids,
            &composition.preferred_edge_types,
        );
        if all_requirements_match
            && (composition.preferred_edge_types.is_empty() || graph_support > 0.0)
        {
            composition.false_positives += 1;
            composition.updated_at = now_iso();
        }
    }
    for profile in adaptive.learned_edge_profiles.iter_mut() {
        if edge_profile_is_active(profile) && edge_profile_matches_graph(profile, incident_graph) {
            profile.false_positives += 1;
            profile.updated_at = now_iso();
        }
    }
}

fn merge_unique(target: &mut Vec<String>, additions: &[String], limit: usize) {
    for value in additions {
        if !target.iter().any(|existing| existing == value) {
            target.push(value.clone());
        }
        if target.len() >= limit {
            break;
        }
    }
}

fn parse_json_string_array_value(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn learned_rule_graph_support(
    inference_graph: &InferenceGraph,
    supporting_event_ids: &[String],
    preferred_edge_types: &[String],
) -> f64 {
    if preferred_edge_types.is_empty() {
        return 0.0;
    }
    let support_ids = supporting_event_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let matching = inference_graph
        .edges
        .iter()
        .filter(|edge| {
            preferred_edge_types
                .iter()
                .any(|item| item == &edge.edge_type)
        })
        .filter(|edge| {
            support_ids.is_empty()
                || support_ids.contains(&edge.source_event_id)
                || support_ids.contains(&edge.target_event_id)
        })
        .collect::<Vec<_>>();
    if matching.is_empty() {
        0.0
    } else {
        matching.iter().map(|edge| edge.plausibility).sum::<f64>() / matching.len() as f64
    }
}

fn detector_artifact_refs_for_requirements(
    requirements: &[String],
    learning: &LearningArtifacts,
) -> Vec<LearningArtifactRef> {
    let requirement_set = requirements
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    learning
        .adaptive
        .learned_detectors
        .iter()
        .filter(|detector| detector_is_active(detector))
        .filter(|detector| requirement_set.contains(&detector.requirement_name))
        .map(|detector| LearningArtifactRef {
            kind: "detector".into(),
            artifact_id: detector.detector_id.clone(),
            label: detector.requirement_name.clone(),
            reason: "matched learned detector requirement".into(),
            impact_metric: None,
            impact_value: None,
        })
        .collect()
}

fn candidate_learning_provenance(
    candidate: &Candidate,
    events: &[EventRow],
    inference_graph: &InferenceGraph,
    learning: &LearningArtifacts,
) -> serde_json::Value {
    let support_ids = candidate
        .supporting_events
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut refs = candidate.provenance_refs.clone();
    for detector in learning
        .adaptive
        .learned_detectors
        .iter()
        .filter(|detector| detector_is_active(detector))
    {
        let matched = event_ids_for_requirement(events, &detector.requirement_name, learning);
        if matched
            .iter()
            .any(|event_id| support_ids.contains(event_id))
        {
            refs.push(LearningArtifactRef {
                kind: "detector".into(),
                artifact_id: detector.detector_id.clone(),
                label: detector.requirement_name.clone(),
                reason: "supporting evidence matched learned detector".into(),
                impact_metric: Some("matched_events".into()),
                impact_value: Some(
                    matched
                        .iter()
                        .filter(|event_id| support_ids.contains(*event_id))
                        .count() as f64,
                ),
            });
        }
    }
    refs.extend(edge_profile_refs_for_candidate(
        candidate,
        inference_graph,
        learning,
    ));
    let refs = dedup_learning_artifact_refs(refs);
    let edge_types = candidate_relevant_edge_types(candidate, inference_graph);
    let estimated_total_impact = refs
        .iter()
        .filter(|artifact| {
            matches!(
                artifact.impact_metric.as_deref(),
                Some("prior_contribution" | "plausibility_delta")
            )
        })
        .filter_map(|artifact| artifact.impact_value)
        .sum::<f64>();
    serde_json::json!({
        "has_learned_influence": !refs.is_empty(),
        "estimated_total_impact": estimated_total_impact,
        "artifacts": refs.iter().map(|artifact| serde_json::json!({
            "kind": artifact.kind,
            "artifact_id": artifact.artifact_id,
            "label": artifact.label,
            "reason": artifact.reason,
            "impact_metric": artifact.impact_metric,
            "impact_value": artifact.impact_value,
        })).collect::<Vec<_>>(),
        "matched_edge_types": edge_types,
    })
}

fn edge_profile_refs_for_candidate(
    candidate: &Candidate,
    inference_graph: &InferenceGraph,
    learning: &LearningArtifacts,
) -> Vec<LearningArtifactRef> {
    let support_ids = candidate
        .supporting_events
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let relevant_edges = inference_graph
        .edges
        .iter()
        .filter(|edge| {
            support_ids.contains(&edge.source_event_id)
                || support_ids.contains(&edge.target_event_id)
                || candidate
                    .root_cause_event_id
                    .as_ref()
                    .map(|event_id| {
                        event_id == &edge.source_event_id || event_id == &edge.target_event_id
                    })
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    learning
        .adaptive
        .learned_edge_profiles
        .iter()
        .filter(|profile| edge_profile_is_active(profile))
        .filter(|profile| {
            profile
                .cause_type
                .as_deref()
                .map(|cause_type| cause_type == candidate.cause_type)
                .unwrap_or(true)
        })
        .filter(|profile| {
            relevant_edges.iter().any(|edge| {
                edge.edge_type == profile.edge_type
                    && profile
                        .source_service
                        .as_deref()
                        .map(|service| {
                            graph_node(inference_graph, &edge.source_event_id)
                                .map(|node| node.service_id.as_str() == service)
                                .unwrap_or(false)
                        })
                        .unwrap_or(true)
                    && profile
                        .target_service
                        .as_deref()
                        .map(|service| {
                            graph_node(inference_graph, &edge.target_event_id)
                                .map(|node| node.service_id.as_str() == service)
                                .unwrap_or(false)
                        })
                        .unwrap_or(true)
            })
        })
        .map(|profile| {
            let delta = relevant_edges
                .iter()
                .flat_map(|edge| edge.learned_adjustments.iter())
                .filter(|adjustment| adjustment.artifact_id == profile.profile_id)
                .map(|adjustment| adjustment.delta)
                .sum::<f64>();
            LearningArtifactRef {
                kind: "edge_profile".into(),
                artifact_id: profile.profile_id.clone(),
                label: profile.edge_type.clone(),
                reason: "inference graph plausibility adjusted by learned edge profile".into(),
                impact_metric: Some("plausibility_delta".into()),
                impact_value: Some(delta),
            }
        })
        .collect()
}

fn candidate_relevant_edge_types(
    candidate: &Candidate,
    inference_graph: &InferenceGraph,
) -> Vec<String> {
    let support_ids = candidate
        .supporting_events
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut edge_types = inference_graph
        .edges
        .iter()
        .filter(|edge| {
            support_ids.contains(&edge.source_event_id)
                || support_ids.contains(&edge.target_event_id)
                || candidate
                    .root_cause_event_id
                    .as_ref()
                    .map(|event_id| {
                        event_id == &edge.source_event_id || event_id == &edge.target_event_id
                    })
                    .unwrap_or(false)
        })
        .map(|edge| edge.edge_type.clone())
        .collect::<Vec<_>>();
    edge_types.sort();
    edge_types.dedup();
    edge_types
}

fn dedup_learning_artifact_refs(refs: Vec<LearningArtifactRef>) -> Vec<LearningArtifactRef> {
    let mut by_key = std::collections::BTreeMap::<(String, String), LearningArtifactRef>::new();
    for item in refs {
        let key = (item.kind.clone(), item.artifact_id.clone());
        if let Some(existing) = by_key.get_mut(&key) {
            existing.impact_value = match (existing.impact_value, item.impact_value) {
                (Some(left), Some(right)) => Some(left + right),
                (left @ Some(_), None) => left,
                (None, right) => right,
            };
        } else {
            by_key.insert(key, item);
        }
    }
    by_key.into_values().collect()
}

fn detector_is_active(detector: &LearnedDetector) -> bool {
    !detector.manually_disabled && detector.confirmations > detector.false_positives
}

fn template_is_active(template: &LearnedTemplate) -> bool {
    !template.manually_disabled && template.confirmations > template.false_positives
}

fn composition_is_active(composition: &LearnedComposition) -> bool {
    !composition.manually_disabled && composition.confirmations > composition.false_positives
}

fn edge_profile_is_active(profile: &LearnedEdgeProfile) -> bool {
    !profile.manually_disabled && profile.confirmations > profile.false_positives
}

fn edge_profile_matches_graph(profile: &LearnedEdgeProfile, graph: &InferenceGraph) -> bool {
    graph.edges.iter().any(|edge| {
        edge.edge_type == profile.edge_type
            && profile
                .cause_type
                .as_deref()
                .map(|cause_type| cause_type_for_edge(&edge.edge_type) == cause_type)
                .unwrap_or(true)
            && profile
                .source_service
                .as_deref()
                .map(|service| {
                    graph_node(graph, &edge.source_event_id)
                        .map(|node| node.service_id.as_str() == service)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
            && profile
                .target_service
                .as_deref()
                .map(|service| {
                    graph_node(graph, &edge.target_event_id)
                        .map(|node| node.service_id.as_str() == service)
                        .unwrap_or(false)
                })
                .unwrap_or(true)
    })
}

fn apply_learned_edge_adjustment(
    edge: &mut InferenceEdge,
    graph_events: &[GraphEvent],
    learning: &LearningArtifacts,
) {
    let source = graph_events
        .iter()
        .find(|event| event.event_id == edge.source_event_id);
    let target = graph_events
        .iter()
        .find(|event| event.event_id == edge.target_event_id);
    let cause_type = cause_type_for_edge(&edge.edge_type);
    let profiles = learning
        .adaptive
        .learned_edge_profiles
        .iter()
        .filter(|profile| edge_profile_is_active(profile))
        .filter(|profile| profile.edge_type == edge.edge_type)
        .filter(|profile| {
            profile
                .cause_type
                .as_deref()
                .map(|value| value == cause_type)
                .unwrap_or(true)
        })
        .filter(|profile| {
            profile
                .source_service
                .as_deref()
                .map(|value| source.map(|event| event.service_id.as_str()) == Some(value))
                .unwrap_or(true)
        })
        .filter(|profile| {
            profile
                .target_service
                .as_deref()
                .map(|value| target.map(|event| event.service_id.as_str()) == Some(value))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    if profiles.is_empty() {
        return;
    }
    let success_ratio = profiles
        .iter()
        .map(|profile| {
            profile.confirmations as f64
                / (profile.confirmations + profile.false_positives).max(1) as f64
        })
        .sum::<f64>()
        / profiles.len() as f64;
    let average_plausibility = profiles
        .iter()
        .map(|profile| profile.average_plausibility)
        .sum::<f64>()
        / profiles.len() as f64;
    let average_latency = profiles
        .iter()
        .map(|profile| profile.average_latency_ms)
        .sum::<f64>()
        / profiles.len() as f64;
    let baseline = edge.plausibility;
    let latency_alignment = if average_latency <= 0.0 {
        1.0
    } else {
        let drift = ((edge.latency_ms - average_latency).abs() / average_latency.max(1.0)).min(1.0);
        1.0 - (drift * 0.15)
    };
    let anchored = (edge.plausibility * 0.7) + (average_plausibility * 0.3);
    let multiplier = 0.9 + (success_ratio * 0.2);
    edge.plausibility = (anchored * multiplier * latency_alignment).clamp(0.0, 1.0);
    let total_delta = edge.plausibility - baseline;
    let delta_share = if profiles.is_empty() {
        0.0
    } else {
        total_delta / profiles.len() as f64
    };
    edge.learned_adjustments = profiles
        .iter()
        .map(|profile| InferenceEdgeAdjustment {
            artifact_id: profile.profile_id.clone(),
            artifact_label: profile.edge_type.clone(),
            baseline_plausibility: baseline,
            adjusted_plausibility: edge.plausibility,
            delta: delta_share,
        })
        .collect();
}

fn learned_detector_matches_event(
    detector: &LearnedDetector,
    event: &EventRow,
    text: &str,
) -> bool {
    if detector
        .min_severity
        .is_some_and(|minimum| event_severity(event).unwrap_or(SEVERITY_INFO) < minimum)
    {
        return false;
    }
    if !detector.source_types.is_empty() {
        let event_source = event
            .source_ref
            .as_ref()
            .and_then(|source| source.source_type.as_ref())
            .map(|value| value.to_ascii_lowercase());
        if !event_source
            .as_deref()
            .is_some_and(|source_type| detector.source_types.iter().any(|item| item == source_type))
        {
            return false;
        }
    }
    let term_matches = detector
        .positive_terms
        .iter()
        .filter(|term| text.contains(term.as_str()))
        .count();
    let tag_matches = detector
        .tags
        .iter()
        .filter(|tag| {
            event
                .tags
                .as_ref()
                .map(|items| items.iter().any(|item| item.eq_ignore_ascii_case(tag)))
                .unwrap_or(false)
        })
        .count();
    if detector.positive_terms.is_empty() {
        return tag_matches > 0;
    }
    let needed_terms = detector.positive_terms.len().min(2);
    term_matches >= needed_terms || (term_matches >= 1 && tag_matches >= 1)
}

fn clamp_learned_weights(
    weights: &mut std::collections::BTreeMap<String, f64>,
    defaults: &std::collections::BTreeMap<String, f64>,
    max_drift: f64,
    min_weight: f64,
) {
    for component in scoring_component_names() {
        let default = *defaults.get(component).unwrap_or(&min_weight);
        let lower = (default * (1.0 - max_drift)).max(min_weight);
        let upper = (default * (1.0 + max_drift)).max(lower);
        let slot = weights.entry(component.to_string()).or_insert(default);
        *slot = slot.clamp(lower, upper);
    }
}

fn renormalize_weights(weights: &mut std::collections::BTreeMap<String, f64>, target_total: f64) {
    let current_total = weights.values().sum::<f64>().max(0.0001);
    for value in weights.values_mut() {
        *value = (*value / current_total) * target_total;
    }
}

fn scoring_component_names() -> [&'static str; 6] {
    [
        "temporal_alignment",
        "correlation_strength",
        "frequency_weight",
        "dependency_proximity",
        "evidence_coverage",
        "anomaly_severity",
    ]
}

fn score_breakdown_component(hypothesis: &serde_json::Value, component: &str) -> f64 {
    hypothesis
        .get("score_breakdown")
        .and_then(|value| value.get(component))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_default()
}

fn calibration_enabled(config: &TomlValue) -> bool {
    config
        .get("calibration")
        .and_then(|value| value.get("enabled"))
        .and_then(TomlValue::as_bool)
        .unwrap_or(true)
}

fn calibration_bucket_count(config: &TomlValue) -> usize {
    config
        .get("calibration")
        .and_then(|value| value.get("bucket_count"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(5)
        .clamp(2, 20) as usize
}

fn calibration_min_samples(config: &TomlValue) -> u64 {
    config
        .get("calibration")
        .and_then(|value| value.get("min_samples_per_bucket"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(10)
        .max(1) as u64
}

fn default_calibration_buckets(bucket_count: usize) -> Vec<CalibrationBucket> {
    let bucket_count = bucket_count.max(1);
    let size = 1.0 / bucket_count as f64;
    (0..bucket_count)
        .map(|index| CalibrationBucket {
            score_lower: index as f64 * size,
            score_upper: if index + 1 == bucket_count {
                1.0
            } else {
                (index + 1) as f64 * size
            },
            total_predictions: 0,
            correct_predictions: 0,
            accuracy: 0.0,
            sample_confidence: "insufficient".into(),
        })
        .collect()
}

fn calibration_bucket_index(buckets: &[CalibrationBucket], score: f64) -> usize {
    buckets
        .iter()
        .position(|bucket| score >= bucket.score_lower && score < bucket.score_upper)
        .unwrap_or_else(|| buckets.len().saturating_sub(1))
}

fn calibration_bucket_accuracy(calibration: &CalibrationModel, score: f64) -> f64 {
    calibration
        .buckets
        .get(calibration_bucket_index(&calibration.buckets, score))
        .map(|bucket| bucket.accuracy)
        .unwrap_or_default()
}

fn calibration_bucket_is_sufficient(calibration: &CalibrationModel, score: f64) -> bool {
    calibration
        .buckets
        .get(calibration_bucket_index(&calibration.buckets, score))
        .map(|bucket| bucket.sample_confidence == "sufficient")
        .unwrap_or(false)
}

fn calibration_staleness_status(config: &TomlValue, calibration: &CalibrationModel) -> String {
    if calibration.total_feedback_count < 20 {
        return "insufficient_data".into();
    }
    let stale_days = config
        .get("calibration")
        .and_then(|value| value.get("staleness_threshold_days"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(30)
        .max(1);
    let Some(last_updated) = calibration.last_updated.as_deref().and_then(parse_rfc3339) else {
        return "insufficient_data".into();
    };
    let age = time::OffsetDateTime::now_utc() - last_updated;
    if age.whole_days() >= stale_days {
        "stale".into()
    } else {
        "current".into()
    }
}

fn scoring_weight_defaults() -> std::collections::BTreeMap<String, f64> {
    std::collections::BTreeMap::from([
        ("temporal_alignment".into(), 0.25),
        ("correlation_strength".into(), 0.20),
        ("frequency_weight".into(), 0.15),
        ("dependency_proximity".into(), 0.15),
        ("evidence_coverage".into(), 0.15),
        ("anomaly_severity".into(), 0.10),
    ])
}

fn scoring_learning_rate(config: &TomlValue) -> f64 {
    table_f64(config, &["scoring", "tuning", "learning_rate"], 0.05).clamp(0.0, 1.0)
}

fn scoring_max_drift(config: &TomlValue) -> f64 {
    table_f64(
        config,
        &["scoring", "tuning", "max_drift_from_default"],
        0.5,
    )
    .clamp(0.0, 1.0)
}

fn scoring_min_weight(config: &TomlValue) -> f64 {
    table_f64(config, &["scoring", "tuning", "min_weight"], 0.03).clamp(0.0, 1.0)
}

fn effective_scoring_weights(
    config: &TomlValue,
    learned_weights: &LearnedScoringWeights,
) -> ScoringWeights {
    let defaults = scoring_weights(config);
    let lookup = |key: &str, fallback: f64| {
        learned_weights
            .effective_weights
            .get(key)
            .copied()
            .unwrap_or(fallback)
    };
    let temporal_alignment = lookup("temporal_alignment", defaults.temporal_alignment);
    let correlation_strength = lookup("correlation_strength", defaults.correlation_strength);
    let frequency_weight = lookup("frequency_weight", defaults.frequency_weight);
    let dependency_proximity = lookup("dependency_proximity", defaults.dependency_proximity);
    let evidence_coverage = lookup("evidence_coverage", defaults.evidence_coverage);
    let anomaly_severity = lookup("anomaly_severity", defaults.anomaly_severity);
    let total = temporal_alignment
        + correlation_strength
        + frequency_weight
        + dependency_proximity
        + evidence_coverage
        + anomaly_severity;
    ScoringWeights {
        temporal_alignment,
        correlation_strength,
        frequency_weight,
        dependency_proximity,
        evidence_coverage,
        anomaly_severity,
        total,
    }
}

fn candidate_scoring_components(
    events: &[EventRow],
    candidate: &Candidate,
    domain_metrics: &DomainMetrics,
    inference_graph: &InferenceGraph,
) -> CandidateScoring {
    let support_count = candidate.supporting_events.len().max(1);
    let coverage =
        (support_count as f64 / domain_metrics.event_count.max(1) as f64).clamp(0.0, 1.0);
    let related_services = candidate.affected_services.len().max(1);
    let source_types = distinct_source_types(events).len().max(1);
    let base_correlation = (((related_services.saturating_sub(1)).min(4) as f64) / 4.0
        + ((source_types.saturating_sub(1)).min(4) as f64) / 4.0)
        / 2.0;
    let frequency_weight = (domain_metrics.warn_count.max(domain_metrics.error_count) as f64
        / domain_metrics.event_count.max(1) as f64)
        .clamp(0.0, 1.0);
    let graph_impact = candidate
        .root_cause_event_id
        .as_deref()
        .map(|event_id| graph_origin_impact(inference_graph, event_id))
        .unwrap_or_default();
    let graph_plausibility = candidate
        .root_cause_event_id
        .as_deref()
        .map(|event_id| average_outgoing_plausibility(inference_graph, event_id))
        .unwrap_or_default();
    CandidateScoring {
        temporal_alignment: domain_metrics.temporal_alignment.max(graph_plausibility),
        correlation_strength: base_correlation.max(graph_impact * 0.8),
        frequency_weight,
        dependency_proximity: candidate.dependency_proximity.max(graph_plausibility),
        evidence_coverage: coverage,
        anomaly_severity: domain_metrics.anomaly_score,
        graph_impact,
    }
}

fn candidate_tiebreak_ordering(
    config: &TomlValue,
    left: &Candidate,
    right: &Candidate,
) -> std::cmp::Ordering {
    let order = config
        .get("scoring")
        .and_then(|value| value.get("tuning"))
        .and_then(|value| value.get("tiebreak_order"))
        .and_then(TomlValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(TomlValue::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            vec![
                "evidence_coverage".into(),
                "contradicting_events_asc".into(),
                "root_cause_timestamp_asc".into(),
            ]
        });
    for key in order {
        let ordering = match key.as_str() {
            "evidence_coverage" => cmp_f64_desc(
                left.scoring.evidence_coverage,
                right.scoring.evidence_coverage,
            ),
            "contradicting_events_asc" => left
                .contradicting_events
                .len()
                .cmp(&right.contradicting_events.len()),
            "root_cause_timestamp_asc" => {
                left.root_cause_timestamp.cmp(&right.root_cause_timestamp)
            }
            "graph_impact" => cmp_f64_desc(left.scoring.graph_impact, right.scoring.graph_impact),
            _ => std::cmp::Ordering::Equal,
        };
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    left.description.cmp(&right.description)
}

fn cmp_f64_desc(left: f64, right: f64) -> std::cmp::Ordering {
    right
        .partial_cmp(&left)
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn graph_root_candidates(
    events: &[EventRow],
    all_services: &[String],
    _source_types: &[String],
    domain_metrics: &DomainMetrics,
    inference_graph: &InferenceGraph,
) -> Vec<Candidate> {
    inference_graph
        .root_candidates
        .iter()
        .take(3)
        .filter_map(|event_id| {
            let node = graph_node(inference_graph, event_id)?;
            let support_ids = support_ids_from_root(inference_graph, event_id);
            let edge_type = dominant_outgoing_edge_type(inference_graph, event_id)
                .unwrap_or_else(|| "same_service_escalation".into());
            let cause_type = cause_type_for_edge(&edge_type);
            let graph_impact = graph_origin_impact(inference_graph, event_id);
            let average_plausibility = average_outgoing_plausibility(inference_graph, event_id);
            Some(Candidate {
                cause_type,
                description: format!(
                    "Inference graph origin candidate '{}' reaches {:.0}% of symptom leaves.",
                    node.summary,
                    graph_impact * 100.0
                ),
                prior: (0.45 + graph_impact * 0.35 + average_plausibility * 0.2)
                    .max(domain_metrics.anomaly_score * 0.5)
                    .clamp(0.0, 0.95),
                score: 0.0,
                suggested_checks: assumptions_for_origin(inference_graph, event_id)
                    .into_iter()
                    .take(3)
                    .chain(std::iter::once(
                        "Validate whether the inferred origin event is a real trigger or just the first visible symptom.".into(),
                    ))
                    .collect(),
                supporting_events: support_ids.clone(),
                affected_services: services_for_event_ids(events, &support_ids)
                    .into_iter()
                    .chain(all_services.iter().cloned())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                contradicting_events: Vec::new(),
                dependency_proximity: average_plausibility,
                is_valid: true,
                invalidation_reasons: Vec::new(),
                root_cause_event_id: Some(event_id.clone()),
                root_cause_timestamp: Some(node.timestamp.clone()),
                scoring: CandidateScoring::default(),
                provenance_refs: Vec::new(),
            })
        })
        .collect()
}

fn graph_root_for_cause(inference_graph: &InferenceGraph, cause_type: &str) -> Option<String> {
    inference_graph.root_candidates.iter().find_map(|event_id| {
        let edge_type = dominant_outgoing_edge_type(inference_graph, event_id)?;
        (cause_type_for_edge(&edge_type) == cause_type).then(|| event_id.clone())
    })
}

fn graph_node<'a>(graph: &'a InferenceGraph, event_id: &str) -> Option<&'a InferenceNode> {
    graph.nodes.iter().find(|node| node.event_id == event_id)
}

fn support_ids_from_root(graph: &InferenceGraph, root_event_id: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::from([root_event_id.to_string()]);
    let mut stack = vec![root_event_id.to_string()];
    while let Some(current) = stack.pop() {
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.source_event_id == current)
        {
            if seen.insert(edge.target_event_id.clone()) {
                stack.push(edge.target_event_id.clone());
            }
        }
    }
    seen.into_iter().collect()
}

fn graph_origin_impact(graph: &InferenceGraph, root_event_id: &str) -> f64 {
    let reachable = support_ids_from_root(graph, root_event_id)
        .into_iter()
        .filter(|event_id| graph.leaf_nodes.iter().any(|leaf| leaf == event_id))
        .count();
    reachable as f64 / graph.leaf_nodes.len().max(1) as f64
}

fn dominant_outgoing_edge_type(graph: &InferenceGraph, root_event_id: &str) -> Option<String> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.source_event_id == root_event_id)
    {
        *counts.entry(edge.edge_type.clone()).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)))
        .map(|item| item.0)
}

fn average_outgoing_plausibility(graph: &InferenceGraph, event_id: &str) -> f64 {
    let outgoing = graph
        .edges
        .iter()
        .filter(|edge| edge.source_event_id == event_id)
        .collect::<Vec<_>>();
    if outgoing.is_empty() {
        return 0.0;
    }
    outgoing.iter().map(|edge| edge.plausibility).sum::<f64>() / outgoing.len() as f64
}

fn assumptions_for_origin(graph: &InferenceGraph, event_id: &str) -> Vec<String> {
    graph
        .edges
        .iter()
        .filter(|edge| edge.source_event_id == event_id)
        .flat_map(|edge| edge.requires.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn cause_type_for_edge(edge_type: &str) -> String {
    match edge_type {
        "dependency_propagation" | "timeout_chain" => "dependency_failure",
        "resource_preceded_error" => "resource_pressure",
        "restart_preceded_disconnection" | "same_service_escalation" => "service_instability",
        "config_preceded_error" => "configuration_change",
        "shared_fate" => "shared_fate",
        _ => "unknown",
    }
    .into()
}

#[cfg(test)]
fn build_inference_graph(config: &TomlValue, events: &[EventRow]) -> InferenceGraph {
    build_inference_graph_with_learning(config, events, &LearningArtifacts::default())
}

fn build_inference_graph_with_learning(
    config: &TomlValue,
    events: &[EventRow],
    learning: &LearningArtifacts,
) -> InferenceGraph {
    let max_events = config
        .get("inference_graph")
        .and_then(|value| value.get("max_events_for_graph"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(500)
        .clamp(10, 5_000) as usize;
    let plausibility_threshold =
        table_f64(config, &["inference_graph", "plausibility_threshold"], 0.15).clamp(0.0, 1.0);
    let max_edges_per_node = config
        .get("inference_graph")
        .and_then(|value| value.get("max_edges_per_node"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(10)
        .clamp(1, 100) as usize;
    let mut graph_events = events
        .iter()
        .filter_map(graph_event_from_row)
        .collect::<Vec<_>>();
    graph_events.sort_by(|left, right| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    let truncated = graph_events.len() > max_events;
    if truncated {
        graph_events = graph_events[graph_events.len().saturating_sub(max_events)..].to_vec();
    }
    if graph_events.is_empty() {
        return InferenceGraph {
            nodes: Vec::new(),
            edges: Vec::new(),
            root_candidates: Vec::new(),
            leaf_nodes: Vec::new(),
            truncated,
        };
    }
    let by_service = group_graph_events_by_service(&graph_events);
    let topology = topology_edges(config);
    let mut candidate_edges = Vec::<InferenceEdge>::new();
    if inference_strategy_enabled(config, "dependency_propagation") {
        candidate_edges.extend(dependency_propagation_edges(&topology, &by_service));
    }
    if inference_strategy_enabled(config, "same_service_escalation") {
        candidate_edges.extend(same_service_escalation_edges(&by_service));
    }
    if inference_strategy_enabled(config, "resource_preceded_error") {
        candidate_edges.extend(resource_preceded_error_edges(&by_service));
    }
    if inference_strategy_enabled(config, "config_preceded_error") {
        candidate_edges.extend(config_preceded_error_edges(&topology, &by_service));
    }
    if inference_strategy_enabled(config, "restart_preceded_disconnection") {
        candidate_edges.extend(restart_preceded_disconnection_edges(&topology, &by_service));
    }
    if inference_strategy_enabled(config, "shared_fate") {
        candidate_edges.extend(shared_fate_edges(&graph_events));
    }
    if inference_strategy_enabled(config, "timeout_chain") {
        candidate_edges.extend(timeout_chain_edges(&topology, &by_service));
    }
    for edge in &mut candidate_edges {
        apply_learned_edge_adjustment(edge, &graph_events, learning);
    }
    candidate_edges.retain(|edge| edge.plausibility >= plausibility_threshold);
    candidate_edges.sort_by(|left, right| {
        right
            .plausibility
            .partial_cmp(&left.plausibility)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.source_event_id.cmp(&right.source_event_id))
            .then_with(|| left.target_event_id.cmp(&right.target_event_id))
            .then_with(|| left.edge_type.cmp(&right.edge_type))
    });
    let mut accepted = Vec::<InferenceEdge>::new();
    let mut outgoing = std::collections::BTreeMap::<String, usize>::new();
    let mut incoming = std::collections::BTreeMap::<String, usize>::new();
    let mut seen = std::collections::BTreeSet::<(String, String, String)>::new();
    for edge in candidate_edges {
        let key = (
            edge.source_event_id.clone(),
            edge.target_event_id.clone(),
            edge.edge_type.clone(),
        );
        if !seen.insert(key) {
            continue;
        }
        if outgoing
            .get(&edge.source_event_id)
            .copied()
            .unwrap_or_default()
            >= max_edges_per_node
        {
            continue;
        }
        if incoming
            .get(&edge.target_event_id)
            .copied()
            .unwrap_or_default()
            >= max_edges_per_node
        {
            continue;
        }
        *outgoing.entry(edge.source_event_id.clone()).or_default() += 1;
        *incoming.entry(edge.target_event_id.clone()).or_default() += 1;
        accepted.push(edge);
    }
    let nodes = graph_events
        .iter()
        .map(|event| {
            let in_degree = incoming.get(&event.event_id).copied().unwrap_or_default();
            let out_degree = outgoing.get(&event.event_id).copied().unwrap_or_default();
            InferenceNode {
                event_id: event.event_id.clone(),
                service_id: event.service_id.clone(),
                timestamp: event.timestamp_raw.clone(),
                severity: event.severity,
                summary: event.summary.clone(),
                node_type: if in_degree == 0 && out_degree > 0 {
                    "origin_candidate"
                } else if out_degree == 0 && in_degree > 0 {
                    "symptom"
                } else {
                    "intermediate"
                }
                .into(),
                in_degree,
                out_degree,
            }
        })
        .collect::<Vec<_>>();
    let mut root_candidates = nodes
        .iter()
        .filter(|node| node.in_degree == 0)
        .map(|node| node.event_id.clone())
        .collect::<Vec<_>>();
    let leaf_nodes = nodes
        .iter()
        .filter(|node| node.out_degree == 0)
        .map(|node| node.event_id.clone())
        .collect::<Vec<_>>();
    let graph = InferenceGraph {
        nodes,
        edges: accepted,
        root_candidates: Vec::new(),
        leaf_nodes,
        truncated,
    };
    root_candidates.sort_by(|left, right| {
        graph_origin_impact(&graph, right)
            .partial_cmp(&graph_origin_impact(&graph, left))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.cmp(right))
    });
    InferenceGraph {
        root_candidates,
        ..graph
    }
}

fn graph_event_from_row(event: &EventRow) -> Option<GraphEvent> {
    Some(GraphEvent {
        event_id: event.event_id.clone()?,
        service_id: event.service_id.clone().unwrap_or_else(|| "runtime".into()),
        timestamp: parse_rfc3339(event.timestamp.as_deref()?)?,
        timestamp_raw: event.timestamp.clone()?,
        severity: event_severity(event).unwrap_or(SEVERITY_INFO),
        summary: event
            .message
            .clone()
            .or_else(|| event.summary.clone())
            .unwrap_or_default(),
        source_type: event
            .source_ref
            .as_ref()
            .and_then(|source| source.source_type.clone())
            .unwrap_or_else(|| "runtime".into()),
        tags: event.tags.clone().unwrap_or_default(),
    })
}

fn group_graph_events_by_service(
    events: &[GraphEvent],
) -> std::collections::BTreeMap<String, Vec<GraphEvent>> {
    let mut grouped = std::collections::BTreeMap::<String, Vec<GraphEvent>>::new();
    for event in events {
        grouped
            .entry(event.service_id.clone())
            .or_default()
            .push(event.clone());
    }
    grouped
}

fn topology_edges(config: &TomlValue) -> Vec<(String, String)> {
    config
        .get("topology")
        .and_then(|value| value.get("edges"))
        .and_then(TomlValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(TomlValue::as_table)
                .filter_map(|edge| {
                    let source = edge.get("source").and_then(TomlValue::as_str)?;
                    let target = edge.get("target").and_then(TomlValue::as_str)?;
                    (!source.is_empty() && !target.is_empty())
                        .then(|| (source.to_string(), target.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn inference_strategy_enabled(config: &TomlValue, strategy: &str) -> bool {
    config
        .get("inference_graph")
        .and_then(|value| value.get("strategies"))
        .and_then(|value| value.get(strategy))
        .and_then(TomlValue::as_bool)
        .unwrap_or(true)
}

fn dependency_propagation_edges(
    topology: &[(String, String)],
    by_service: &std::collections::BTreeMap<String, Vec<GraphEvent>>,
) -> Vec<InferenceEdge> {
    let mut edges = Vec::new();
    for (source, target) in topology {
        let Some(source_events) = by_service.get(source) else {
            continue;
        };
        let Some(target_events) = by_service.get(target) else {
            continue;
        };
        for source_event in source_events
            .iter()
            .filter(|event| event.severity >= SEVERITY_ERROR)
        {
            for target_event in target_events
                .iter()
                .filter(|event| event.severity >= SEVERITY_WARN)
            {
                if let Some(edge) = make_graph_edge(
                    source_event,
                    target_event,
                    "dependency_propagation",
                    10.0,
                    60.0,
                    format!("{source} errored before dependent {target}"),
                    vec![
                        format!("service graph edge {source}->{target} is correct"),
                        "temporal proximity implies contribution".into(),
                    ],
                ) {
                    edges.push(edge);
                }
            }
        }
    }
    edges
}

fn same_service_escalation_edges(
    by_service: &std::collections::BTreeMap<String, Vec<GraphEvent>>,
) -> Vec<InferenceEdge> {
    let mut edges = Vec::new();
    for (service_id, events) in by_service {
        for window in events.windows(2) {
            let [left, right] = window else { continue };
            if right.severity > left.severity {
                if let Some(edge) = make_graph_edge(
                    left,
                    right,
                    "same_service_escalation",
                    10.0,
                    30.0,
                    format!("{service_id} severity escalated"),
                    vec!["severity escalation is progressive, not coincidental".into()],
                ) {
                    edges.push(edge);
                }
            }
        }
    }
    edges
}

fn resource_preceded_error_edges(
    by_service: &std::collections::BTreeMap<String, Vec<GraphEvent>>,
) -> Vec<InferenceEdge> {
    let mut edges = Vec::new();
    for (service_id, events) in by_service {
        let resource_events = events.iter().filter(|event| {
            event.source_type == "host_metrics"
                || event.source_type == "process_snapshot"
                || has_any_term(
                    &format!("{} {}", event.summary, event.tags.join(" ")).to_ascii_lowercase(),
                    &["cpu", "memory", "disk", "oom", "saturation", "resource"],
                )
        });
        let error_events = events
            .iter()
            .filter(|event| event.severity >= SEVERITY_ERROR);
        for resource_event in resource_events {
            for error_event in error_events.clone() {
                if let Some(edge) = make_graph_edge(
                    resource_event,
                    error_event,
                    "resource_preceded_error",
                    12.0,
                    60.0,
                    format!("resource pressure on {service_id} preceded error"),
                    vec!["resource metric is related to the error".into()],
                ) {
                    edges.push(edge);
                }
            }
        }
    }
    edges
}

fn config_preceded_error_edges(
    topology: &[(String, String)],
    by_service: &std::collections::BTreeMap<String, Vec<GraphEvent>>,
) -> Vec<InferenceEdge> {
    let mut edges = Vec::new();
    for (service_id, events) in by_service {
        let config_events = events.iter().filter(|event| {
            has_any_term(
                &format!("{} {}", event.summary, event.tags.join(" ")).to_ascii_lowercase(),
                &["config", "deploy", "deployment", "rollout", "release"],
            )
        });
        for config_event in config_events {
            for candidate_service in std::iter::once(service_id).chain(
                topology
                    .iter()
                    .filter_map(|(source, target)| (source == service_id).then_some(target)),
            ) {
                if let Some(related_events) = by_service.get(candidate_service.as_str()) {
                    for error_event in related_events
                        .iter()
                        .filter(|event| event.severity >= SEVERITY_ERROR)
                    {
                        if let Some(edge) = make_graph_edge(
                            config_event,
                            error_event,
                            "config_preceded_error",
                            60.0,
                            300.0,
                            format!(
                                "configuration change on {} preceded error on {}",
                                service_id, candidate_service
                            ),
                            vec![
                                "config change is relevant to the error".into(),
                                "temporal proximity implies contribution (weak)".into(),
                            ],
                        ) {
                            edges.push(edge);
                        }
                    }
                }
            }
        }
    }
    edges
}

fn restart_preceded_disconnection_edges(
    topology: &[(String, String)],
    by_service: &std::collections::BTreeMap<String, Vec<GraphEvent>>,
) -> Vec<InferenceEdge> {
    let mut edges = Vec::new();
    for (service_id, events) in by_service {
        let restarts = events.iter().filter(|event| {
            has_any_term(
                &format!("{} {}", event.summary, event.tags.join(" ")).to_ascii_lowercase(),
                &["restart", "crash", "restarted", "panic"],
            )
        });
        let dependents = topology
            .iter()
            .filter_map(|(source, target)| (target == service_id).then_some(source))
            .collect::<Vec<_>>();
        for restart_event in restarts {
            for dependent in &dependents {
                if let Some(dep_events) = by_service.get(*dependent) {
                    for dep_event in dep_events.iter().filter(|event| {
                        has_any_term(
                            &format!("{} {}", event.summary, event.tags.join(" "))
                                .to_ascii_lowercase(),
                            &["connection refused", "timeout", "unavailable"],
                        )
                    }) {
                        if let Some(edge) = make_graph_edge(
                            restart_event,
                            dep_event,
                            "restart_preceded_disconnection",
                            8.0,
                            30.0,
                            format!(
                                "{service_id} restart preceded dependent failure on {dependent}"
                            ),
                            vec!["restart caused brief unavailability".into()],
                        ) {
                            edges.push(edge);
                        }
                    }
                }
            }
        }
    }
    edges
}

fn shared_fate_edges(events: &[GraphEvent]) -> Vec<InferenceEdge> {
    let mut edges = Vec::new();
    for window in events.windows(2) {
        let [left, right] = window else { continue };
        if left.service_id != right.service_id
            && left.severity >= SEVERITY_WARN
            && right.severity >= SEVERITY_WARN
        {
            let latency = (right.timestamp - left.timestamp).whole_seconds().abs() as f64;
            if latency <= 10.0 {
                edges.push(InferenceEdge {
                    source_event_id: left.event_id.clone(),
                    target_event_id: right.event_id.clone(),
                    edge_type: "shared_fate".into(),
                    plausibility: temporal_plausibility_seconds(latency, 5.0) * 0.6,
                    latency_ms: latency * 1000.0,
                    evidence: format!(
                        "{} and {} degraded within {:.1}s",
                        left.service_id, right.service_id, latency
                    ),
                    requires: vec!["shared timing implies common fault domain".into()],
                    learned_adjustments: Vec::new(),
                });
            }
        }
    }
    edges
}

fn timeout_chain_edges(
    topology: &[(String, String)],
    by_service: &std::collections::BTreeMap<String, Vec<GraphEvent>>,
) -> Vec<InferenceEdge> {
    let mut edges = Vec::new();
    for (caller, callee) in topology {
        let Some(caller_events) = by_service.get(caller) else {
            continue;
        };
        let Some(callee_events) = by_service.get(callee) else {
            continue;
        };
        for callee_event in callee_events.iter().filter(|event| {
            has_any_term(
                &format!("{} {}", event.summary, event.tags.join(" ")).to_ascii_lowercase(),
                &["timeout", "connection refused", "upstream", "unavailable"],
            )
        }) {
            for caller_event in caller_events.iter().filter(|event| {
                has_any_term(
                    &format!("{} {}", event.summary, event.tags.join(" ")).to_ascii_lowercase(),
                    &["timeout", "connection refused", "upstream", "unavailable"],
                )
            }) {
                if let Some(edge) = make_graph_edge(
                    callee_event,
                    caller_event,
                    "timeout_chain",
                    8.0,
                    60.0,
                    format!("{callee} timeout plausibly propagated to caller {caller}"),
                    vec![format!("service graph edge {caller}->{callee} is correct")],
                ) {
                    edges.push(edge);
                }
            }
        }
    }
    edges
}

fn make_graph_edge(
    source: &GraphEvent,
    target: &GraphEvent,
    edge_type: &str,
    halflife_seconds: f64,
    max_latency_seconds: f64,
    evidence: String,
    requires: Vec<String>,
) -> Option<InferenceEdge> {
    let latency = (target.timestamp - source.timestamp).whole_seconds() as f64;
    if !(0.0 < latency && latency <= max_latency_seconds) {
        return None;
    }
    Some(InferenceEdge {
        source_event_id: source.event_id.clone(),
        target_event_id: target.event_id.clone(),
        edge_type: edge_type.into(),
        plausibility: temporal_plausibility_seconds(latency, halflife_seconds).clamp(0.0, 1.0),
        latency_ms: latency * 1000.0,
        evidence,
        requires,
        learned_adjustments: Vec::new(),
    })
}

fn temporal_plausibility_seconds(latency_seconds: f64, halflife_seconds: f64) -> f64 {
    if latency_seconds <= 0.0 {
        return 0.0;
    }
    let decay = (-0.693_f64 * latency_seconds / halflife_seconds.max(0.1)).exp();
    decay.clamp(0.0, 1.0)
}

fn services_for_event_ids(events: &[EventRow], event_ids: &[String]) -> Vec<String> {
    let wanted = event_ids.iter().collect::<std::collections::BTreeSet<_>>();
    events
        .iter()
        .filter(|event| {
            event
                .event_id
                .as_ref()
                .map(|id| wanted.contains(id))
                .unwrap_or(false)
        })
        .filter_map(|event| event.service_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn discover_runtime_containers() -> Option<Vec<RuntimeContainer>> {
    for binary in ["docker", "podman"] {
        let output = std::process::Command::new(binary)
            .args(["ps", "--format", "{{.Names}}\t{{.Image}}\t{{.Status}}"])
            .output();
        let Ok(output) = output else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let containers = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(parse_container_line)
            .take(30)
            .collect::<Vec<_>>();
        if !containers.is_empty() {
            return Some(containers);
        }
    }
    None
}

fn parse_container_line(raw: &str) -> Option<RuntimeContainer> {
    let mut parts = raw.splitn(3, '\t');
    let name = parts.next()?.trim();
    let image = parts.next()?.trim();
    let status = parts.next()?.trim();
    if name.is_empty() || image.is_empty() || status.is_empty() {
        return None;
    }
    let state = status
        .split_whitespace()
        .next()
        .unwrap_or(status)
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    Some(RuntimeContainer {
        name: name.into(),
        image: image.into(),
        state: if state.is_empty() {
            "unknown".into()
        } else {
            state
        },
    })
}

fn custom_rule_candidates(
    config: &TomlValue,
    events: &[EventRow],
    all_services: &[String],
    source_types: &[String],
    domain_metrics: &DomainMetrics,
    inference_graph: &InferenceGraph,
    learning: &LearningArtifacts,
) -> Vec<Candidate> {
    #[derive(Debug, Clone)]
    struct RuleSpec {
        cause_type: String,
        cause_subtype: Option<String>,
        title_template: String,
        confidence: f64,
        requires: Vec<String>,
        requires_same_service: bool,
        requires_temporal_order: bool,
        preferred_edge_types: Vec<String>,
        artifact_kind: Option<String>,
        artifact_id: Option<String>,
        artifact_label: Option<String>,
        learned: bool,
    }

    let mut rules = Vec::<RuleSpec>::new();
    if let Some(config_rules) = config
        .get("hypothesis_engine")
        .and_then(|value| value.get("custom_rules"))
        .and_then(TomlValue::as_array)
    {
        for rule in config_rules.iter().filter_map(TomlValue::as_table) {
            let requires = rule
                .get("requires")
                .and_then(TomlValue::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(TomlValue::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if requires.is_empty() {
                continue;
            }
            rules.push(RuleSpec {
                cause_type: rule
                    .get("cause_type")
                    .and_then(TomlValue::as_str)
                    .unwrap_or("custom_rule")
                    .to_string(),
                cause_subtype: rule
                    .get("cause_subtype")
                    .and_then(TomlValue::as_str)
                    .map(str::to_string),
                title_template: rule
                    .get("title_template")
                    .and_then(TomlValue::as_str)
                    .unwrap_or("Custom hypothesis rule matched the observed evidence")
                    .to_string(),
                confidence: rule
                    .get("confidence")
                    .and_then(TomlValue::as_float)
                    .or_else(|| {
                        rule.get("confidence")
                            .and_then(TomlValue::as_integer)
                            .map(|value| value as f64)
                    })
                    .unwrap_or(0.6)
                    .clamp(0.0, 1.0),
                requires,
                requires_same_service: rule
                    .get("requires_same_service")
                    .and_then(TomlValue::as_bool)
                    .unwrap_or(false),
                requires_temporal_order: rule
                    .get("requires_temporal_order")
                    .and_then(TomlValue::as_bool)
                    .unwrap_or(false),
                preferred_edge_types: Vec::new(),
                artifact_kind: None,
                artifact_id: None,
                artifact_label: None,
                learned: false,
            });
        }
    }
    for template in learning
        .adaptive
        .learned_templates
        .iter()
        .filter(|template| template_is_active(template))
    {
        if template.requires.is_empty() {
            continue;
        }
        rules.push(RuleSpec {
            cause_type: template.cause_type.clone(),
            cause_subtype: template.cause_subtype.clone(),
            title_template: template.title_template.clone(),
            confidence: template.confidence,
            requires: template.requires.clone(),
            requires_same_service: template.requires_same_service,
            requires_temporal_order: template.requires_temporal_order,
            preferred_edge_types: Vec::new(),
            artifact_kind: Some("template".into()),
            artifact_id: Some(template.template_id.clone()),
            artifact_label: Some(template.template_name.clone()),
            learned: true,
        });
    }
    for composition in learning
        .adaptive
        .learned_compositions
        .iter()
        .filter(|composition| composition_is_active(composition))
    {
        if composition.requires.is_empty() {
            continue;
        }
        rules.push(RuleSpec {
            cause_type: composition.cause_type.clone(),
            cause_subtype: composition.cause_subtype.clone(),
            title_template: composition.title_template.clone(),
            confidence: composition.confidence,
            requires: composition.requires.clone(),
            requires_same_service: composition.requires_same_service,
            requires_temporal_order: composition.requires_temporal_order,
            preferred_edge_types: composition.preferred_edge_types.clone(),
            artifact_kind: Some("composition".into()),
            artifact_id: Some(composition.composition_id.clone()),
            artifact_label: Some(composition.composition_name.clone()),
            learned: true,
        });
    }
    if rules.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    for rule in rules {
        let requires = rule.requires.clone();
        let mut supporting = Vec::new();
        let mut matched_terms = Vec::new();
        for requirement in &requires {
            let matches = event_ids_for_requirement(events, requirement, learning);
            if matches.is_empty() {
                supporting.clear();
                break;
            }
            matched_terms.push(requirement.clone());
            supporting.extend(matches);
        }
        if supporting.is_empty() {
            continue;
        }
        supporting.sort();
        supporting.dedup();
        if rule.requires_same_service && services_in_events(events).len() > 1 {
            continue;
        }
        if rule.requires_temporal_order
            && !requirements_have_temporal_order(events, &requires, learning)
        {
            continue;
        }
        let graph_support =
            learned_rule_graph_support(inference_graph, &supporting, &rule.preferred_edge_types);
        let mut provenance_refs = detector_artifact_refs_for_requirements(&requires, learning);
        if let (Some(kind), Some(artifact_id)) =
            (rule.artifact_kind.as_deref(), rule.artifact_id.as_deref())
        {
            provenance_refs.push(LearningArtifactRef {
                kind: kind.to_string(),
                artifact_id: artifact_id.to_string(),
                label: rule
                    .artifact_label
                    .clone()
                    .unwrap_or_else(|| artifact_id.to_string()),
                reason: if kind == "composition" {
                    "matched learned composition rule".into()
                } else {
                    "matched learned hypothesis template".into()
                },
                impact_metric: None,
                impact_value: None,
            });
        }
        let description = rule.title_template;
        let confidence = if rule.preferred_edge_types.is_empty() {
            rule.confidence
        } else if graph_support > 0.0 {
            (rule.confidence + graph_support * 0.15).clamp(0.0, 1.0)
        } else {
            (rule.confidence * 0.8).clamp(0.0, 1.0)
        };
        let rule_prior_contribution = (confidence.max(domain_metrics.anomaly_score * 0.5)
            - (domain_metrics.anomaly_score * 0.5))
            .max(0.0);
        for item in provenance_refs
            .iter_mut()
            .filter(|item| matches!(item.kind.as_str(), "template" | "composition"))
        {
            item.impact_metric = Some("prior_contribution".into());
            item.impact_value = Some(rule_prior_contribution);
        }
        let root_cause_event_id = supporting.first().cloned();
        let root_cause_timestamp = root_cause_event_id.as_ref().and_then(|event_id| {
            events
                .iter()
                .find(|event| event.event_id.as_deref() == Some(event_id))
                .and_then(|event| event.timestamp.clone())
        });
        candidates.push(Candidate {
            cause_type: rule.cause_type,
            description: if let Some(subtype) = rule.cause_subtype {
                format!("{description} ({subtype})")
            } else {
                description
            },
            prior: confidence.max(domain_metrics.anomaly_score * 0.5),
            score: 0.0,
            suggested_checks: if rule.learned {
                vec![
                    format!("Validate learned evidence: {}", matched_terms.join(", ")),
                    "Confirm the new learned pattern still reflects the same failure mode.".into(),
                ]
            } else {
                vec![
                    format!("Verify custom rule evidence: {}", matched_terms.join(", ")),
                    "Inspect the event timeline for missing or contradictory context".into(),
                ]
            },
            supporting_events: supporting,
            affected_services: all_services.to_vec(),
            contradicting_events: Vec::new(),
            dependency_proximity: ((source_types.len() > 1) as usize as f64 * 0.2 + 0.5)
                .clamp(0.0, 1.0),
            is_valid: true,
            invalidation_reasons: Vec::new(),
            root_cause_event_id,
            root_cause_timestamp,
            scoring: CandidateScoring::default(),
            provenance_refs,
        });
    }
    candidates
}

fn event_ids_for_requirement(
    events: &[EventRow],
    requirement: &str,
    learning: &LearningArtifacts,
) -> Vec<String> {
    events
        .iter()
        .filter(|event| requirement_matches_event(requirement, event, learning))
        .filter_map(|event| event.event_id.clone())
        .collect()
}

fn requirement_matches_event(
    requirement: &str,
    event: &EventRow,
    learning: &LearningArtifacts,
) -> bool {
    let text = event_signal_text(event);
    let normalized = requirement.trim().to_ascii_lowercase();
    if let Some(detector) =
        learning.adaptive.learned_detectors.iter().find(|detector| {
            detector.requirement_name == normalized && detector_is_active(detector)
        })
    {
        return learned_detector_matches_event(detector, event, &text);
    }
    match normalized.as_str() {
        "connection_failures_outbound" => has_any_term(
            &text,
            &[
                "timeout",
                "connection refused",
                "upstream",
                "dependency",
                "dns",
            ],
        ),
        "error_spike" => {
            event_severity(event).unwrap_or(SEVERITY_INFO) >= SEVERITY_ERROR
                || has_any_term(&text, &["spike", "burst", "error rate"])
        }
        "resource_pressure" => has_any_term(
            &text,
            &[
                "cpu",
                "memory",
                "disk",
                "oom",
                "resource pressure",
                "saturation",
            ],
        ),
        "restart_loop" => has_any_term(&text, &["restart", "crash", "panic", "backoff"]),
        "deployment_event" => has_any_term(&text, &["deploy", "deployment", "rollout", "release"]),
        "config_change_event" => has_any_term(
            &text,
            &[
                "config change",
                "configuration",
                "feature flag",
                "setting updated",
            ],
        ),
        other => text.contains(other),
    }
}

fn requirements_have_temporal_order(
    events: &[EventRow],
    requirements: &[String],
    learning: &LearningArtifacts,
) -> bool {
    let mut last_index = None;
    for requirement in requirements {
        let Some(index) = events
            .iter()
            .position(|event| requirement_matches_event(requirement, event, learning))
        else {
            return false;
        };
        if last_index.map(|previous| index < previous).unwrap_or(false) {
            return false;
        }
        last_index = Some(index);
    }
    true
}

fn contradiction_events_for_candidate(
    config: &TomlValue,
    events: &[EventRow],
    cause_type: &str,
) -> Vec<String> {
    if !contradictions_enabled(config) {
        return Vec::new();
    }
    let generic_terms = [
        "healthy",
        "recovered",
        "restored",
        "back to normal",
        "stable",
    ];
    let resource_terms = [
        "cpu normal",
        "memory normal",
        "disk normal",
        "resource recovered",
    ];
    let dependency_terms = [
        "connection restored",
        "dependency healthy",
        "upstream healthy",
    ];
    let instability_terms = ["started successfully", "steady state", "running normally"];
    let terms: &[&str] = match cause_type {
        "resource_pressure" => &resource_terms,
        "dependency_failure" => &dependency_terms,
        "service_instability" => &instability_terms,
        _ => &generic_terms,
    };
    events
        .iter()
        .filter(|event| has_any_term(&event_signal_text(event), terms))
        .filter_map(|event| event.event_id.clone())
        .collect()
}

fn contradiction_penalty(config: &TomlValue, contradiction_ratio: f64) -> f64 {
    if !contradictions_enabled(config) {
        return 0.0;
    }
    if contradiction_ratio <= 0.0 {
        return 0.0;
    }
    let strong = config
        .get("contradiction_handling")
        .and_then(|value| value.get("strong_penalty_per_contradiction"))
        .and_then(TomlValue::as_float)
        .or_else(|| {
            config
                .get("contradiction_handling")
                .and_then(|value| value.get("strong_penalty_per_contradiction"))
                .and_then(TomlValue::as_integer)
                .map(|value| value as f64)
        })
        .unwrap_or(0.15);
    let weak = config
        .get("contradiction_handling")
        .and_then(|value| value.get("weak_penalty_per_contradiction"))
        .and_then(TomlValue::as_float)
        .or_else(|| {
            config
                .get("contradiction_handling")
                .and_then(|value| value.get("weak_penalty_per_contradiction"))
                .and_then(TomlValue::as_integer)
                .map(|value| value as f64)
        })
        .unwrap_or(0.05);
    let multiplier = config
        .get("contradiction_handling")
        .and_then(|value| value.get("min_penalty_multiplier"))
        .and_then(TomlValue::as_float)
        .or_else(|| {
            config
                .get("contradiction_handling")
                .and_then(|value| value.get("min_penalty_multiplier"))
                .and_then(TomlValue::as_integer)
                .map(|value| value as f64)
        })
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let per_ratio = if contradiction_ratio >= contradiction_fail_threshold(config) {
        strong
    } else {
        weak
    };
    (contradiction_ratio * per_ratio)
        .max(multiplier * weak)
        .clamp(0.0, 0.6)
}

fn contradictions_enabled(config: &TomlValue) -> bool {
    config
        .get("contradiction_handling")
        .and_then(|value| value.get("enabled"))
        .and_then(TomlValue::as_bool)
        .unwrap_or(true)
}

fn contradiction_fail_threshold(config: &TomlValue) -> f64 {
    config
        .get("hypothesis_validation")
        .and_then(|value| value.get("contradiction_ratio_fail"))
        .and_then(TomlValue::as_float)
        .or_else(|| {
            config
                .get("hypothesis_validation")
                .and_then(|value| value.get("contradiction_ratio_fail"))
                .and_then(TomlValue::as_integer)
                .map(|value| value as f64)
        })
        .unwrap_or(0.6)
        .clamp(0.0, 1.0)
}

fn contradiction_warn_threshold(config: &TomlValue) -> f64 {
    config
        .get("hypothesis_validation")
        .and_then(|value| value.get("contradiction_ratio_warn"))
        .and_then(TomlValue::as_float)
        .or_else(|| {
            config
                .get("hypothesis_validation")
                .and_then(|value| value.get("contradiction_ratio_warn"))
                .and_then(TomlValue::as_integer)
                .map(|value| value as f64)
        })
        .unwrap_or(0.3)
        .clamp(0.0, 1.0)
}

fn dependency_proximity_score(cause_type: &str, events: &[EventRow]) -> f64 {
    let text = events
        .iter()
        .map(event_signal_text)
        .collect::<Vec<_>>()
        .join(" ");
    match cause_type {
        "dependency_failure" => {
            if has_any_term(
                &text,
                &[
                    "timeout",
                    "upstream",
                    "database",
                    "redis",
                    "dns",
                    "connection refused",
                ],
            ) {
                0.95
            } else {
                0.55
            }
        }
        "shared_fate" => {
            if services_in_events(events).len() > 1 {
                0.8
            } else {
                0.45
            }
        }
        "orchestration_change" => {
            if has_any_term(
                &text,
                &["docker", "kubernetes", "container", "pod", "rollout"],
            ) {
                0.85
            } else {
                0.4
            }
        }
        "resource_pressure"
            if has_any_term(&text, &["cpu", "memory", "disk", "oom", "saturation"]) =>
        {
            0.8
        }
        "resource_pressure" => 0.35,
        _ => 0.35,
    }
}

fn candidate_overlap_ratio(left: &Candidate, right: &Candidate) -> f64 {
    let left_set = left
        .supporting_events
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let right_set = right
        .supporting_events
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    let intersection = left_set.intersection(&right_set).count() as f64;
    let smaller = left_set.len().min(right_set.len()).max(1) as f64;
    intersection / smaller
}

fn anomaly_trigger_threshold(config: &TomlValue) -> f64 {
    let spike_z = config
        .get("anomaly_detection")
        .and_then(|value| value.get("spike_z_threshold"))
        .and_then(TomlValue::as_float)
        .or_else(|| {
            config
                .get("anomaly_detection")
                .and_then(|value| value.get("spike_z_threshold"))
                .and_then(TomlValue::as_integer)
                .map(|value| value as f64)
        })
        .unwrap_or(3.0);
    (spike_z / 5.0).clamp(0.45, 0.9)
}

fn anomaly_weights(config: &TomlValue) -> AnomalyWeights {
    let error_rate = table_f64(
        config,
        &["anomaly_detection", "weights", "error_rate"],
        0.35,
    );
    let event_volume = table_f64(
        config,
        &["anomaly_detection", "weights", "event_volume"],
        0.2,
    );
    let new_fingerprint_rate = table_f64(
        config,
        &["anomaly_detection", "weights", "new_fingerprint_rate"],
        0.2,
    );
    let restart_count = table_f64(
        config,
        &["anomaly_detection", "weights", "restart_count"],
        0.15,
    );
    let warn_rate = table_f64(config, &["anomaly_detection", "weights", "warn_rate"], 0.1);
    let total =
        (error_rate + event_volume + new_fingerprint_rate + restart_count + warn_rate).max(0.0001);
    AnomalyWeights {
        error_rate: error_rate / total,
        event_volume: event_volume / total,
        new_fingerprint_rate: new_fingerprint_rate / total,
        restart_count: restart_count / total,
        warn_rate: warn_rate / total,
    }
}

#[derive(Debug, Clone, Copy)]
struct AnomalyWeights {
    error_rate: f64,
    event_volume: f64,
    new_fingerprint_rate: f64,
    restart_count: f64,
    warn_rate: f64,
}

fn scoring_weights(config: &TomlValue) -> ScoringWeights {
    let temporal_alignment = table_f64(config, &["scoring", "temporal_alignment"], 0.25);
    let correlation_strength = table_f64(config, &["scoring", "correlation_strength"], 0.2);
    let frequency_weight = table_f64(config, &["scoring", "frequency_weight"], 0.15);
    let dependency_proximity = table_f64(config, &["scoring", "dependency_proximity"], 0.15);
    let evidence_coverage = table_f64(config, &["scoring", "evidence_coverage"], 0.15);
    let anomaly_severity = table_f64(config, &["scoring", "anomaly_severity"], 0.1);
    let total = temporal_alignment
        + correlation_strength
        + frequency_weight
        + dependency_proximity
        + evidence_coverage
        + anomaly_severity;
    ScoringWeights {
        temporal_alignment,
        correlation_strength,
        frequency_weight,
        dependency_proximity,
        evidence_coverage,
        anomaly_severity,
        total,
    }
}

fn calibration_thresholds(config: &TomlValue) -> (f64, f64) {
    let high =
        table_f64(config, &["calibration", "defaults", "high_threshold"], 0.75).clamp(0.0, 1.0);
    let medium = table_f64(
        config,
        &["calibration", "defaults", "medium_threshold"],
        0.4,
    )
    .clamp(0.0, high);
    (high, medium)
}

fn table_f64(config: &TomlValue, path: &[&str], default: f64) -> f64 {
    let mut current = config;
    for segment in path {
        let Some(next) = current.get(*segment) else {
            return default;
        };
        current = next;
    }
    current
        .as_float()
        .or_else(|| current.as_integer().map(|value| value as f64))
        .unwrap_or(default)
}

fn event_signal_text(event: &EventRow) -> String {
    format!(
        "{} {} {} {}",
        event.service_id.clone().unwrap_or_default(),
        event
            .message
            .clone()
            .or_else(|| event.summary.clone())
            .unwrap_or_default(),
        event
            .source_ref
            .as_ref()
            .and_then(|source| source.source_type.clone())
            .unwrap_or_default(),
        event.tags.clone().unwrap_or_default().join(" ")
    )
    .to_ascii_lowercase()
}

fn slug(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn unix_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn now_iso() -> String {
    let seconds = unix_seconds();
    chrono_like_iso(seconds)
}

fn chrono_like_iso(seconds: i64) -> String {
    time::OffsetDateTime::from_unix_timestamp(seconds)
        .ok()
        .and_then(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

fn severity_counts_payload(events: &[EventRow]) -> serde_json::Value {
    let mut counts = serde_json::Map::new();
    for event in events {
        let severity = event_severity(event).unwrap_or(SEVERITY_INFO).to_string();
        let current = counts
            .get(&severity)
            .and_then(|value| value.as_i64())
            .unwrap_or(0);
        counts.insert(severity, serde_json::Value::from(current + 1));
    }
    serde_json::Value::Object(counts)
}

fn event_rate_payload(events: &[EventRow]) -> serde_json::Value {
    let mut timestamps = events
        .iter()
        .filter_map(|event| event.timestamp.as_deref())
        .filter_map(parse_rfc3339)
        .collect::<Vec<_>>();
    timestamps.sort();
    let span_minutes = if let (Some(first), Some(last)) = (timestamps.first(), timestamps.last()) {
        let seconds = (*last - *first).whole_seconds().max(60);
        seconds as f64 / 60.0
    } else {
        1.0
    };
    serde_json::json!({
        "events": events.len(),
        "per_minute": ((events.len() as f64) / span_minutes * 100.0).round() / 100.0,
        "sample_window_minutes": (span_minutes * 100.0).round() / 100.0,
    })
}

fn dedup_payload(
    config: &TomlValue,
    sampled_events: usize,
    governance: &GovernanceSummary,
) -> serde_json::Value {
    let dedup = config.get("deduplication").and_then(TomlValue::as_table);
    serde_json::json!({
        "enabled": dedup.and_then(|table| table.get("enabled")).and_then(TomlValue::as_bool).unwrap_or(true),
        "window_seconds": dedup.and_then(|table| table.get("window_seconds")).and_then(TomlValue::as_integer).unwrap_or(60),
        "max_tracked_fingerprints": dedup.and_then(|table| table.get("max_tracked_fingerprints")).and_then(TomlValue::as_integer).unwrap_or(10_000),
        "sampled_events": sampled_events,
        "tracked_fingerprints": governance.tracked_fingerprints,
        "active_windows": governance.active_dedup_windows,
        "suppressed_total": governance.dedup_suppressed_total,
        "active_window_suppressed": governance.active_window_suppressed,
        "inserted_total": governance.inserted_total,
    })
}

fn noise_payload(
    config: &TomlValue,
    event_rate: &serde_json::Value,
    governance: &GovernanceSummary,
) -> serde_json::Value {
    let noise = config.get("noise_filter").and_then(TomlValue::as_table);
    let per_minute = event_rate
        .get("per_minute")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or_default();
    let threshold = noise
        .and_then(|table| table.get("high_rate_threshold_per_minute"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(100) as f64;
    serde_json::json!({
        "enabled": noise.and_then(|table| table.get("enabled")).and_then(TomlValue::as_bool).unwrap_or(true),
        "registry_enabled": noise.and_then(|table| table.get("registry_enabled")).and_then(TomlValue::as_bool).unwrap_or(true),
        "high_rate_threshold_per_minute": threshold,
        "suppression_active": per_minute >= threshold || governance.noise_suppressed_total > 0,
        "suppressed_total": governance.noise_suppressed_total,
        "allowlisted_total": governance.allowlisted_total,
        "retained_due_to_severity_total": governance.retained_due_to_severity_total,
        "last_noise_reason": governance.last_noise_reason,
    })
}

fn parse_rfc3339(raw: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339).ok()
}

pub fn enrich_incident_rows_with_latest_traces(
    incidents: Vec<IncidentRow>,
    incidents_db: Option<&IncidentsStore>,
    events_db: Option<&EventsStore>,
) -> Result<Vec<IncidentRow>> {
    let (Some(incidents_db), Some(events_db)) = (incidents_db, events_db) else {
        return Ok(incidents);
    };
    incidents
        .into_iter()
        .map(|mut incident| {
            let event_ids = incidents_db.incident_event_ids(&incident.incident_id)?;
            incident.latest_trace_summary =
                events_db.latest_trace_summary_for_event_ids(&event_ids)?;
            Ok(incident)
        })
        .collect()
}

fn enrich_service_rows(
    stats: &[ServiceStats],
    incidents: &[IncidentRow],
    events_db: Option<&EventsStore>,
) -> Result<Vec<ServiceRow>> {
    let mut incident_by_service: std::collections::HashMap<String, Vec<IncidentRow>> =
        std::collections::HashMap::new();
    for inc in incidents {
        let mut ids: Vec<String> = inc.affected_services.clone().unwrap_or_default();
        if !inc.primary_service.is_empty() {
            ids.push(inc.primary_service.clone());
        }
        for sid in ids {
            incident_by_service
                .entry(sid)
                .or_default()
                .push(inc.clone());
        }
    }

    stats
        .iter()
        .map(|s| -> Result<ServiceRow> {
            let event_count = s.event_count;
            let error_count = s.error_count;
            let ratio = if event_count > 0 {
                error_count as f64 / event_count as f64
            } else {
                0.0
            };
            let related = incident_by_service
                .get(&s.service_id)
                .cloned()
                .unwrap_or_default();
            let status =
                if !related.is_empty() && related.iter().any(|i| i.severity >= SEVERITY_ERROR) {
                    "critical"
                } else if !related.is_empty() || ratio >= 0.25 {
                    "degraded"
                } else if error_count > 0 {
                    "elevated"
                } else {
                    "healthy"
                };
            Ok(ServiceRow {
                service_id: s.service_id.clone(),
                status: status.into(),
                event_count: Some(event_count),
                error_count: Some(error_count),
                error_ratio: Some((ratio * 1000.0).round() / 1000.0),
                last_event_at: s.last_event_at.clone(),
                active_incidents: if related.is_empty() {
                    None
                } else {
                    Some(related)
                },
                latest_trace_summary: if let Some(events_db) = events_db {
                    events_db.latest_trace_summary_for_service(&s.service_id)?
                } else {
                    None
                },
            })
        })
        .collect()
}

pub fn service_row_for_id(
    service_id: &str,
    paths: &Paths,
    incidents: &[IncidentRow],
) -> Result<Option<ServiceRow>> {
    let events_db = match EventsStore::open(&paths.events_db)? {
        Some(db) => db,
        None => return Ok(None),
    };
    let stats = match events_db.service_stats_for_id(service_id)? {
        Some(stats) => stats,
        None => return Ok(None),
    };
    Ok(enrich_service_rows(&[stats], incidents, Some(&events_db))?
        .into_iter()
        .next())
}

const STRONG_WORKSPACE_PROJECT_MARKERS: &[(&str, &str)] = &[
    ("pnpm-workspace.yaml", "pnpm_workspace"),
    ("package.json", "node"),
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
    ("pubspec.yaml", "flutter"),
];

const WEAK_WORKSPACE_PROJECT_MARKERS: &[(&str, &str)] = &[
    ("yarn.lock", "yarn"),
    ("package-lock.json", "npm"),
    ("Makefile", "make"),
    ("compose.yaml", "compose"),
    ("compose.yml", "compose"),
    ("docker-compose.yaml", "compose"),
    ("docker-compose.yml", "compose"),
    ("Dockerfile", "docker"),
    (".git", "git"),
];

fn discover_projects(config: &TomlValue, root: &Path) -> Vec<WorkspaceProject> {
    let mut out = Vec::new();
    let max_depth = workspace_max_depth(config);
    let max_results = workspace_max_results(config);

    for scan_root in workspace_roots(config, root) {
        for entry in WalkDir::new(&scan_root)
            .max_depth(max_depth)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| should_scan_workspace_entry(entry.path()))
            .filter_map(|e| e.ok())
        {
            if out.len() >= max_results {
                break;
            }
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if !should_accept_workspace_project(path) {
                continue;
            }
            if let Some((marker, kind)) = workspace_project_marker(path) {
                let rel = display_path(path);
                if out.iter().any(|p: &WorkspaceProject| p.path == rel) {
                    continue;
                }
                out.push(WorkspaceProject {
                    path: rel,
                    kind: kind.into(),
                    marker: marker.into(),
                });
            }
        }
        if out.len() >= max_results {
            break;
        }
    }
    out
}

fn workspace_enabled(config: &TomlValue) -> bool {
    config
        .get("workspace")
        .and_then(|value| value.get("enabled"))
        .and_then(TomlValue::as_bool)
        .unwrap_or(true)
}

fn workspace_max_depth(config: &TomlValue) -> usize {
    config
        .get("workspace")
        .and_then(|value| value.get("max_depth"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(4)
        .clamp(1, 16) as usize
}

fn workspace_max_results(config: &TomlValue) -> usize {
    config
        .get("workspace")
        .and_then(|value| value.get("max_results"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(100)
        .clamp(1, 1000) as usize
}

fn workspace_roots(config: &TomlValue, default_root: &Path) -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    let base_root = default_root
        .canonicalize()
        .unwrap_or_else(|_| default_root.to_path_buf());
    push_workspace_root(&mut roots, base_root.clone());
    let has_config_roots = config
        .get("workspace")
        .and_then(|value| value.get("roots"))
        .and_then(TomlValue::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    if let Some(items) = config
        .get("workspace")
        .and_then(|value| value.get("roots"))
        .and_then(TomlValue::as_array)
    {
        for item in items.iter().filter_map(TomlValue::as_str) {
            let candidate = std::path::PathBuf::from(item);
            let resolved = if candidate.is_absolute() {
                candidate
            } else {
                base_root.join(candidate)
            };
            let resolved = resolved.canonicalize().unwrap_or(resolved);
            push_workspace_root(&mut roots, resolved);
        }
    }
    if !has_config_roots {
        if let Some(home) = std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
        {
            for relative in [
                "Projects",
                "projects",
                "src",
                "Source",
                "source",
                "repos",
                "code",
                "workspace",
                "Workspace",
                "Documents\\Projects",
                "Documents\\repos",
                "Desktop",
            ] {
                let candidate = home.join(relative);
                if candidate.is_dir() {
                    push_workspace_root(&mut roots, candidate);
                }
            }
        }
    }
    roots
}

fn push_workspace_root(roots: &mut Vec<PathBuf>, candidate: PathBuf) {
    let resolved = candidate.canonicalize().unwrap_or(candidate);
    if !roots.iter().any(|existing| existing == &resolved) {
        roots.push(resolved);
    }
}

fn display_path(path: &Path) -> String {
    clean_display_path(&path.to_string_lossy())
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

fn config_workspace_mappings(config: &TomlValue) -> Vec<WorkspaceMapping> {
    let Some(arr) = config
        .get("workspace")
        .and_then(|w| w.get("service_mappings"))
        .and_then(|v| v.as_array())
    else {
        return vec![];
    };

    arr.iter()
        .filter_map(|item| item.as_table())
        .map(|table| WorkspaceMapping {
            service_id: table
                .get("service_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            project_path: table
                .get("project_path")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            confidence: table
                .get("confidence")
                .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
                .unwrap_or(1.0),
            source: table
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string(),
            notes: table
                .get("notes")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            signals: vec![WorkspaceMappingSignal {
                name: "config".into(),
                confidence: 1.0,
                detail: "Declared in workspace.service_mappings".into(),
            }],
        })
        .filter(|m| !m.service_id.is_empty() && !m.project_path.is_empty())
        .collect()
}

fn should_scan_workspace_entry(path: &Path) -> bool {
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

fn workspace_marker_exists(path: &Path, marker: &str) -> bool {
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

fn workspace_project_from_path(path: &Path) -> Option<WorkspaceProject> {
    let root = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !root.is_dir() {
        return None;
    }
    if !should_accept_workspace_project(&root) {
        return None;
    }
    workspace_project_marker(&root).map(|(marker, kind)| WorkspaceProject {
        path: display_path(&root),
        kind: kind.into(),
        marker: marker.into(),
    })
}

fn should_accept_workspace_project(path: &Path) -> bool {
    if path.join(".inferra").join("app.toml").exists()
        || path.join(".inferra").join("inferra.toml").exists()
        || path.join(".inferra").join("workspace.toml").exists()
    {
        return true;
    }
    let rendered = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    if is_user_profile_root(path) {
        return false;
    }
    if rendered.contains("\\windows\\")
        || rendered.contains("\\program files\\")
        || rendered.contains("\\program files (x86)\\")
        || rendered.contains("\\node_modules\\")
        || rendered.ends_with("\\node_modules")
    {
        return false;
    }
    true
}

fn workspace_project_marker(path: &Path) -> Option<(&'static str, &'static str)> {
    for (marker, kind) in STRONG_WORKSPACE_PROJECT_MARKERS {
        if workspace_marker_exists(path, marker) {
            return Some((marker, kind));
        }
    }
    for (marker, kind) in WEAK_WORKSPACE_PROJECT_MARKERS {
        if workspace_marker_exists(path, marker) && weak_workspace_marker_allowed(path, marker) {
            return Some((marker, kind));
        }
    }
    None
}

fn weak_workspace_marker_allowed(path: &Path, marker: &str) -> bool {
    has_workspace_registration(path)
        || STRONG_WORKSPACE_PROJECT_MARKERS
            .iter()
            .any(|(candidate, _)| *candidate != marker && workspace_marker_exists(path, candidate))
        || has_workspace_source_evidence(path)
}

fn has_workspace_registration(path: &Path) -> bool {
    path.join(".inferra").join("app.toml").exists()
        || path.join(".inferra").join("inferra.toml").exists()
        || path.join(".inferra").join("workspace.toml").exists()
}

fn has_workspace_source_evidence(path: &Path) -> bool {
    [
        "src", "app", "lib", "cmd", "pkg", "internal", "services", "server", "client", "frontend",
        "backend",
    ]
    .iter()
    .any(|name| path.join(name).is_dir())
}

fn is_user_profile_root(path: &Path) -> bool {
    let windows = clean_display_path(&path.to_string_lossy())
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let windows_segments: Vec<&str> = windows
        .split('\\')
        .filter(|segment| !segment.is_empty())
        .collect();
    if windows_segments.len() == 3
        && matches!(windows_segments[1], "users" | "documents and settings")
    {
        return true;
    }

    let unix = clean_display_path(&path.to_string_lossy())
        .trim_end_matches('/')
        .to_ascii_lowercase();
    let unix_segments: Vec<&str> = unix
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    unix_segments.len() == 2 && matches!(unix_segments[0], "home" | "users")
}

fn discover_nearest_workspace_project(path: &Path) -> Option<WorkspaceProject> {
    let mut current = if path.is_file() {
        path.parent().map(Path::to_path_buf)?
    } else {
        path.to_path_buf()
    };
    current = current.canonicalize().unwrap_or(current);
    for ancestor in current.ancestors().take(12) {
        if is_workspace_discovery_boundary(ancestor) {
            return None;
        }
        if let Some(project) = workspace_project_from_path(ancestor) {
            return Some(project);
        }
    }
    None
}

fn is_workspace_discovery_boundary(path: &Path) -> bool {
    let rendered = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    rendered == "c:\\"
        || rendered == "c:\\windows"
        || rendered == "c:\\windows\\system32"
        || rendered == "c:\\windows\\syswow64"
        || rendered == "c:\\program files"
        || rendered == "c:\\program files (x86)"
        || rendered == "c:\\programdata"
        || rendered.ends_with("\\node_modules")
}

#[derive(Clone, Copy)]
struct WorkspaceSignature {
    id: &'static str,
    label: &'static str,
    support_type: &'static str,
    detects: &'static [&'static str],
    log_hints: &'static [&'static str],
}

const LANGUAGE_SIGNATURES: &[WorkspaceSignature] = &[
    WorkspaceSignature {
        id: "python",
        label: "Python",
        support_type: "language",
        detects: &[
            "python",
            "py.exe",
            "uvicorn",
            "gunicorn",
            "daphne",
            "celery",
            "flask",
            "django",
            "manage.py",
        ],
        log_hints: &[
            "logging",
            "structlog",
            "loguru",
            "uvicorn access/error logs",
        ],
    },
    WorkspaceSignature {
        id: "nodejs",
        label: "Node.js",
        support_type: "language",
        detects: &[
            "node",
            "npm",
            "npx",
            "pnpm",
            "yarn",
            "tsx",
            "ts-node",
            "next",
            "vite",
            "nuxt",
            "nest",
            "node --watch",
        ],
        log_hints: &["console", "pino", "winston", "morgan", "debug", "PM2"],
    },
    WorkspaceSignature {
        id: "bun",
        label: "Bun",
        support_type: "language",
        detects: &["bun"],
        log_hints: &["console", "Bun.serve"],
    },
    WorkspaceSignature {
        id: "deno",
        label: "Deno",
        support_type: "language",
        detects: &["deno"],
        log_hints: &["console", "std/log"],
    },
    WorkspaceSignature {
        id: "java",
        label: "Java",
        support_type: "language",
        detects: &["java", ".jar", "spring"],
        log_hints: &["logback", "log4j", "java.util.logging"],
    },
    WorkspaceSignature {
        id: "dotnet",
        label: ".NET",
        support_type: "language",
        detects: &["dotnet", ".dll"],
        log_hints: &["ILogger", "Serilog", "NLog"],
    },
    WorkspaceSignature {
        id: "ruby",
        label: "Ruby",
        support_type: "language",
        detects: &["ruby", "rails", "puma", "sidekiq"],
        log_hints: &["Logger", "Rails logs"],
    },
    WorkspaceSignature {
        id: "php",
        label: "PHP",
        support_type: "language",
        detects: &["php", "artisan", "composer"],
        log_hints: &["Monolog", "Laravel logs", "PHP error log"],
    },
    WorkspaceSignature {
        id: "go",
        label: "Go",
        support_type: "language",
        detects: &["go run", "air"],
        log_hints: &["slog", "zap", "zerolog", "log"],
    },
    WorkspaceSignature {
        id: "rust",
        label: "Rust",
        support_type: "language",
        detects: &["cargo run", "rust"],
        log_hints: &["tracing", "log", "env_logger"],
    },
];

const PROCESS_SIGNATURES: &[WorkspaceSignature] = &[
    WorkspaceSignature {
        id: "pm2",
        label: "PM2",
        support_type: "process_manager",
        detects: &["pm2 jlist", "pm_cwd", "pm_exec_path", "exec_interpreter"],
        log_hints: &["pm2 logs", "~/.pm2/logs/*.log"],
    },
    WorkspaceSignature {
        id: "os_process",
        label: "OS process table",
        support_type: "process",
        detects: &["pid", "cwd", "exe", "cmdline"],
        log_hints: &["stdout", "stderr", "service manager logs"],
    },
    WorkspaceSignature {
        id: "container",
        label: "Container runtime",
        support_type: "process_manager",
        detects: &["docker", "compose", "container image/name/state"],
        log_hints: &["docker logs", "compose service logs"],
    },
];

const FRAMEWORK_SIGNATURES: &[WorkspaceSignature] = &[
    WorkspaceSignature {
        id: "uvicorn",
        label: "Uvicorn",
        support_type: "framework",
        detects: &["uvicorn"],
        log_hints: &["uvicorn access/error logs"],
    },
    WorkspaceSignature {
        id: "gunicorn",
        label: "Gunicorn",
        support_type: "framework",
        detects: &["gunicorn"],
        log_hints: &["gunicorn access/error logs"],
    },
    WorkspaceSignature {
        id: "django",
        label: "Django",
        support_type: "framework",
        detects: &["manage.py", "django"],
        log_hints: &["Django logging config"],
    },
    WorkspaceSignature {
        id: "flask",
        label: "Flask",
        support_type: "framework",
        detects: &["flask"],
        log_hints: &["werkzeug", "Flask app logs"],
    },
    WorkspaceSignature {
        id: "fastapi",
        label: "FastAPI",
        support_type: "framework",
        detects: &["fastapi"],
        log_hints: &["uvicorn", "structlog", "logging"],
    },
    WorkspaceSignature {
        id: "celery",
        label: "Celery",
        support_type: "worker",
        detects: &["celery"],
        log_hints: &["celery worker logs"],
    },
    WorkspaceSignature {
        id: "nextjs",
        label: "Next.js",
        support_type: "framework",
        detects: &["next"],
        log_hints: &["next dev/start stdout"],
    },
    WorkspaceSignature {
        id: "vite",
        label: "Vite",
        support_type: "dev_server",
        detects: &["vite"],
        log_hints: &["vite dev server stdout"],
    },
    WorkspaceSignature {
        id: "nuxt",
        label: "Nuxt",
        support_type: "framework",
        detects: &["nuxt"],
        log_hints: &["nuxt server logs"],
    },
    WorkspaceSignature {
        id: "nestjs",
        label: "NestJS",
        support_type: "framework",
        detects: &["nest"],
        log_hints: &["Nest logger", "pino", "winston"],
    },
    WorkspaceSignature {
        id: "rails",
        label: "Rails",
        support_type: "framework",
        detects: &["rails"],
        log_hints: &["log/development.log", "log/production.log"],
    },
    WorkspaceSignature {
        id: "laravel",
        label: "Laravel",
        support_type: "framework",
        detects: &["artisan"],
        log_hints: &["storage/logs/*.log"],
    },
    WorkspaceSignature {
        id: "spring",
        label: "Spring",
        support_type: "framework",
        detects: &["spring"],
        log_hints: &["logback", "application logs"],
    },
    WorkspaceSignature {
        id: "express",
        label: "Express",
        support_type: "framework",
        detects: &["express"],
        log_hints: &["morgan", "pino-http", "winston", "stdout"],
    },
    WorkspaceSignature {
        id: "fastify",
        label: "Fastify",
        support_type: "framework",
        detects: &["fastify"],
        log_hints: &["pino", "stdout", "logs/app.log"],
    },
    WorkspaceSignature {
        id: "koa",
        label: "Koa",
        support_type: "framework",
        detects: &["koa"],
        log_hints: &["koa-logger", "pino", "winston"],
    },
    WorkspaceSignature {
        id: "hono",
        label: "Hono",
        support_type: "framework",
        detects: &["hono"],
        log_hints: &["console", "pino"],
    },
    WorkspaceSignature {
        id: "remix",
        label: "Remix",
        support_type: "framework",
        detects: &["@remix-run/node", "remix"],
        log_hints: &["server stdout", "logs/app.log"],
    },
    WorkspaceSignature {
        id: "astro",
        label: "Astro",
        support_type: "framework",
        detects: &["astro"],
        log_hints: &["dev server stdout", "adapter logs"],
    },
    WorkspaceSignature {
        id: "sveltekit",
        label: "SvelteKit",
        support_type: "framework",
        detects: &["@sveltejs/kit", "sveltekit"],
        log_hints: &["dev server stdout", "adapter logs"],
    },
    WorkspaceSignature {
        id: "axum",
        label: "Axum",
        support_type: "framework",
        detects: &["axum"],
        log_hints: &["tracing", "stdout", "logs/app.log"],
    },
    WorkspaceSignature {
        id: "actix-web",
        label: "Actix Web",
        support_type: "framework",
        detects: &["actix-web"],
        log_hints: &["tracing", "env_logger", "logs/app.log"],
    },
    WorkspaceSignature {
        id: "rocket",
        label: "Rocket",
        support_type: "framework",
        detects: &["rocket"],
        log_hints: &["Rocket logs", "tracing"],
    },
    WorkspaceSignature {
        id: "gin",
        label: "Gin",
        support_type: "framework",
        detects: &["gin-gonic", "gin"],
        log_hints: &["gin logger", "zap", "zerolog"],
    },
    WorkspaceSignature {
        id: "fiber",
        label: "Fiber",
        support_type: "framework",
        detects: &["gofiber", "fiber"],
        log_hints: &["fiber logger", "zap", "zerolog"],
    },
];

const LIBRARY_SIGNATURES: &[WorkspaceSignature] = &[
    WorkspaceSignature {
        id: "pino",
        label: "Pino",
        support_type: "logging_library",
        detects: &["pino"],
        log_hints: &["JSON stdout"],
    },
    WorkspaceSignature {
        id: "winston",
        label: "Winston",
        support_type: "logging_library",
        detects: &["winston"],
        log_hints: &["file transports", "stdout"],
    },
    WorkspaceSignature {
        id: "morgan",
        label: "Morgan",
        support_type: "logging_library",
        detects: &["morgan"],
        log_hints: &["HTTP access logs"],
    },
    WorkspaceSignature {
        id: "structlog",
        label: "structlog",
        support_type: "logging_library",
        detects: &["structlog"],
        log_hints: &["JSON/application logs"],
    },
    WorkspaceSignature {
        id: "loguru",
        label: "Loguru",
        support_type: "logging_library",
        detects: &["loguru"],
        log_hints: &["sink files", "stderr"],
    },
    WorkspaceSignature {
        id: "logback",
        label: "Logback",
        support_type: "logging_library",
        detects: &["logback"],
        log_hints: &["Spring/Java application logs"],
    },
    WorkspaceSignature {
        id: "log4j",
        label: "Log4j",
        support_type: "logging_library",
        detects: &["log4j"],
        log_hints: &["Java application logs"],
    },
    WorkspaceSignature {
        id: "tracing",
        label: "Rust tracing",
        support_type: "logging_library",
        detects: &["tracing"],
        log_hints: &["tracing subscriber output"],
    },
    WorkspaceSignature {
        id: "serilog",
        label: "Serilog",
        support_type: "logging_library",
        detects: &["serilog"],
        log_hints: &["structured .NET logs"],
    },
    WorkspaceSignature {
        id: "nlog",
        label: "NLog",
        support_type: "logging_library",
        detects: &["nlog"],
        log_hints: &[".NET application logs"],
    },
    WorkspaceSignature {
        id: "monolog",
        label: "Monolog",
        support_type: "logging_library",
        detects: &["monolog"],
        log_hints: &["PHP/Laravel logs"],
    },
    WorkspaceSignature {
        id: "zap",
        label: "Zap",
        support_type: "logging_library",
        detects: &["zap"],
        log_hints: &["JSON/application logs"],
    },
    WorkspaceSignature {
        id: "zerolog",
        label: "Zerolog",
        support_type: "logging_library",
        detects: &["zerolog"],
        log_hints: &["JSON/application logs"],
    },
    WorkspaceSignature {
        id: "slog",
        label: "slog",
        support_type: "logging_library",
        detects: &["slog"],
        log_hints: &["Rust structured logs"],
    },
    WorkspaceSignature {
        id: "env_logger",
        label: "env_logger",
        support_type: "logging_library",
        detects: &["env_logger"],
        log_hints: &["RUST_LOG stdout/stderr"],
    },
    WorkspaceSignature {
        id: "bunyan",
        label: "Bunyan",
        support_type: "logging_library",
        detects: &["bunyan"],
        log_hints: &["JSON stdout", "file streams"],
    },
    WorkspaceSignature {
        id: "log4js",
        label: "Log4js",
        support_type: "logging_library",
        detects: &["log4js"],
        log_hints: &["appenders", "logs/app.log"],
    },
    WorkspaceSignature {
        id: "debug",
        label: "debug",
        support_type: "logging_library",
        detects: &["debug"],
        log_hints: &["DEBUG namespace stderr"],
    },
];

fn workspace_support_layers() -> Vec<WorkspaceSupportLayer> {
    vec![
        language_support_layer(),
        support_layer(
            "processes",
            "Processes / managers",
            "Supervisors and process sources used to bind live apps to project directories.",
            PROCESS_SIGNATURES,
        ),
        support_layer(
            "frameworks",
            "Frameworks / app types",
            "Application frameworks, web servers, workers, and dev servers that refine runtime identity.",
            FRAMEWORK_SIGNATURES,
        ),
        support_layer(
            "libraries",
            "Logging libraries",
            "Known logging libraries and conventions that will drive log discovery and AI monitoring.",
            LIBRARY_SIGNATURES,
        ),
    ]
}

fn language_support_layer() -> WorkspaceSupportLayer {
    WorkspaceSupportLayer {
        layer: "languages".into(),
        title: "Languages / runtimes".into(),
        description: "Primary runtime family detected from process names, command lines, scripts, and project markers.".into(),
        items: LANGUAGE_SIGNATURES
            .iter()
            .map(|signature| {
                let children = language_library_ids(signature.id)
                    .iter()
                    .filter_map(|id| LIBRARY_SIGNATURES.iter().find(|library| library.id == *id))
                    .map(support_item)
                    .collect();
                let mut item = support_item(signature);
                item.children = children;
                item
            })
            .collect(),
    }
}

fn language_library_ids(language_id: &str) -> &'static [&'static str] {
    match language_id {
        "python" => &["structlog", "loguru"],
        "nodejs" | "bun" | "deno" => &["pino", "winston", "morgan", "bunyan", "log4js", "debug"],
        "java" => &["logback", "log4j"],
        "dotnet" => &["serilog", "nlog"],
        "php" => &["monolog"],
        "go" => &["zap", "zerolog"],
        "rust" => &["tracing", "slog", "env_logger"],
        _ => &[],
    }
}

fn support_layer(
    layer: &str,
    title: &str,
    description: &str,
    signatures: &[WorkspaceSignature],
) -> WorkspaceSupportLayer {
    WorkspaceSupportLayer {
        layer: layer.into(),
        title: title.into(),
        description: description.into(),
        items: signatures.iter().map(support_item).collect(),
    }
}

fn support_item(signature: &WorkspaceSignature) -> WorkspaceSupportItem {
    WorkspaceSupportItem {
        id: signature.id.into(),
        label: signature.label.into(),
        support_type: signature.support_type.into(),
        detects: signature
            .detects
            .iter()
            .map(|item| (*item).into())
            .collect(),
        log_hints: signature
            .log_hints
            .iter()
            .map(|item| (*item).into())
            .collect(),
        children: Vec::new(),
    }
}

fn discover_workspace_runtime_apps(projects: &[WorkspaceProject]) -> Vec<WorkspaceRuntimeApp> {
    let mut apps = discover_pm2_runtime_apps(projects);
    let pm2_pids: std::collections::HashSet<u32> = apps.iter().filter_map(|app| app.pid).collect();
    for app in discover_process_runtime_apps(projects) {
        if app.pid.map(|pid| pm2_pids.contains(&pid)).unwrap_or(false) {
            continue;
        }
        upsert_runtime_app(&mut apps, app);
    }
    apps.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.name.cmp(&right.name))
    });
    apps.truncate(100);
    apps
}

fn discover_pm2_runtime_apps(projects: &[WorkspaceProject]) -> Vec<WorkspaceRuntimeApp> {
    let Some(output) = command_output_with_timeout("pm2", &["jlist"], 6_000) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return Vec::new();
    };
    let Some(items) = payload.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .map(|item| {
            let env = item.get("pm2_env").unwrap_or(&serde_json::Value::Null);
            let name = item
                .get("name")
                .or_else(|| env.get("name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("pm2-app")
                .to_string();
            let pid = item
                .get("pid")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0);
            let status = env
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let cwd = env
                .get("pm_cwd")
                .or_else(|| item.get("cwd"))
                .and_then(serde_json::Value::as_str)
                .map(clean_display_path);
            let script = env
                .get("pm_exec_path")
                .or_else(|| env.get("script"))
                .or_else(|| item.get("pm_exec_path"))
                .and_then(serde_json::Value::as_str)
                .map(clean_display_path);
            let interpreter = env
                .get("exec_interpreter")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let command = Some(
                [interpreter, script.as_deref().unwrap_or_default()]
                    .into_iter()
                    .filter(|part| !part.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(" "),
            )
            .filter(|value| !value.is_empty());
            let runtime = classify_runtime(
                &name,
                command.as_deref().unwrap_or_default(),
                script.as_deref(),
            );
            let framework =
                detect_framework(command.as_deref().unwrap_or_default(), script.as_deref());
            let libraries =
                detect_libraries(command.as_deref().unwrap_or_default(), script.as_deref());
            let project_path = project_for_paths(projects, cwd.as_deref(), script.as_deref());
            let log_hints =
                runtime_log_hints(&runtime, framework.as_deref(), &libraries, Some("pm2"));
            let mut signals = vec![WorkspaceMappingSignal {
                name: "pm2_jlist".into(),
                confidence: 0.95,
                detail: "PM2 reported this app in jlist".into(),
            }];
            if cwd.is_some() {
                signals.push(WorkspaceMappingSignal {
                    name: "pm2_cwd".into(),
                    confidence: 0.9,
                    detail: "PM2 process working directory is available".into(),
                });
            }
            if script.is_some() {
                signals.push(WorkspaceMappingSignal {
                    name: "pm2_script".into(),
                    confidence: 0.85,
                    detail: "PM2 process script path is available".into(),
                });
            }
            let confidence = if project_path.is_some() { 0.95 } else { 0.75 };
            let endpoints = workspace_app_endpoints(
                command.as_deref(),
                framework.as_deref(),
                project_path.as_deref(),
                Some(env),
            );
            let app_url = primary_app_url(&endpoints);
            let log_sources = workspace_log_sources(
                Some("pm2"),
                &runtime,
                framework.as_deref(),
                &libraries,
                project_path.as_deref(),
                cwd.as_deref(),
                script.as_deref(),
                Some(env),
            );
            let resources = pm2_resources(env);
            let app_state = pm2_app_state(env, status.clone());
            let app_location =
                workspace_app_location(project_path.clone(), cwd.clone(), script.clone(), None);
            let context_capabilities = workspace_app_capabilities(
                &log_sources,
                &endpoints,
                None,
                app_location.as_ref(),
                resources.as_ref(),
                app_state.as_ref(),
            );
            let app_structure = project_path
                .as_deref()
                .map(project_structure)
                .unwrap_or_default();
            let mut app = WorkspaceRuntimeApp {
                pid,
                name,
                display_name: Some(runtime_app_display_name(
                    project_path.as_deref(),
                    script.as_deref(),
                    None,
                ))
                .filter(|value| !value.is_empty()),
                language: Some(runtime.clone()),
                process_kind: Some(process_kind_for(command.as_deref(), framework.as_deref())),
                runtime,
                framework,
                libraries,
                log_hints,
                log_sources,
                app_url,
                endpoints,
                health_endpoint: None,
                app_location,
                resources,
                app_state,
                context_capabilities,
                app_structure,
                manager: Some("pm2".into()),
                status,
                cwd,
                script,
                command,
                project_path,
                latest_trace_summary: None,
                confidence,
                source: "pm2".into(),
                signals,
            };
            apply_workspace_manifest(&mut app);
            app
        })
        .collect()
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

fn discover_process_runtime_apps(projects: &[WorkspaceProject]) -> Vec<WorkspaceRuntimeApp> {
    let mut sys = System::new_all();
    sys.refresh_all();
    let logical_processors = system_logical_processors(&sys);
    let mut apps = Vec::new();
    for process in sys.processes().values() {
        let name = process.name().to_string_lossy().into_owned();
        let cmd_parts: Vec<String> = process
            .cmd()
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();
        let command = if cmd_parts.is_empty() {
            name.clone()
        } else {
            cmd_parts.join(" ")
        };
        let cwd = process.cwd().map(display_path);
        let exe = process.exe().map(display_path);
        let script = infer_script_path(&cmd_parts, cwd.as_deref());
        let project_path = project_for_paths(
            projects,
            cwd.as_deref(),
            script.as_deref().or(exe.as_deref()),
        );
        if project_path.is_none() {
            continue;
        }
        let runtime = classify_runtime(&name, &command, script.as_deref().or(exe.as_deref()));
        let Some(runtime) = runtime_app_runtime(runtime, project_path.as_deref(), exe.as_deref())
        else {
            continue;
        };
        let framework = detect_framework(&command, script.as_deref());
        let endpoints = workspace_app_endpoints(
            Some(&command),
            framework.as_deref(),
            project_path.as_deref(),
            None,
        );
        if !should_keep_process_runtime_app(
            &name,
            &command,
            project_path.as_deref(),
            cwd.as_deref(),
            script.as_deref(),
            exe.as_deref(),
            framework.as_deref(),
            &endpoints,
        ) {
            continue;
        }
        let app_name = runtime_app_name(&name, script.as_deref(), project_path.as_deref());
        let libraries = detect_libraries(&command, script.as_deref());
        let log_hints = runtime_log_hints(&runtime, framework.as_deref(), &libraries, None);
        let mut confidence = if project_path.is_some() { 0.72 } else { 0.45 };
        let mut signals = vec![WorkspaceMappingSignal {
            name: "process_scan".into(),
            confidence,
            detail: "Matched a live OS process runtime".into(),
        }];
        if script.is_some() {
            confidence = confidence.max(0.78);
            signals.push(WorkspaceMappingSignal {
                name: "process_script".into(),
                confidence: 0.78,
                detail: "Command line contains a likely app script".into(),
            });
        }
        if project_path.is_some() {
            confidence = confidence.max(0.82);
            signals.push(WorkspaceMappingSignal {
                name: "process_project_path".into(),
                confidence: 0.82,
                detail: "Process cwd/script/executable is inside a detected project".into(),
            });
        }
        if framework.is_some() {
            confidence = confidence.max(0.58);
            signals.push(WorkspaceMappingSignal {
                name: "process_framework".into(),
                confidence: 0.58,
                detail: "Command line suggests a recognized app framework".into(),
            });
        }
        if !endpoints.is_empty() {
            confidence = confidence.max(0.62);
            signals.push(WorkspaceMappingSignal {
                name: "process_endpoint".into(),
                confidence: 0.62,
                detail: "Command line or framework exposed a likely app endpoint".into(),
            });
        }
        let app_url = primary_app_url(&endpoints);
        let log_sources = workspace_log_sources(
            None,
            &runtime,
            framework.as_deref(),
            &libraries,
            project_path.as_deref(),
            cwd.as_deref(),
            script.as_deref(),
            None,
        );
        let raw_cpu = f64::from(process.cpu_usage());
        let resources = Some(WorkspaceAppResources {
            cpu_percent: Some(normalize_process_cpu_to_host_percent(
                raw_cpu,
                logical_processors,
            )),
            cpu_raw_percent: Some(round_f64(raw_cpu, 2)),
            cpu_percent_scope: Some("host_total".into()),
            cpu_logical_processors: Some(logical_processors),
            memory_mb: Some(round_f64(process.memory() as f64 / (1024.0 * 1024.0), 2)),
            virtual_memory_mb: Some(round_f64(
                process.virtual_memory() as f64 / (1024.0 * 1024.0),
                2,
            )),
            uptime_seconds: Some(process.run_time()),
            process_status: Some(format!("{:?}", process.status())),
        });
        let app_state = Some(WorkspaceAppState {
            health: if process.pid().as_u32() > 0 {
                "running".into()
            } else {
                "unknown".into()
            },
            status: Some(format!("{:?}", process.status())),
            reason: Some("Observed in the local OS process table".into()),
            started_at: None,
            restarts: None,
            observed_by: "process".into(),
        });
        let app_location = workspace_app_location(
            project_path.clone(),
            cwd.clone(),
            script.clone(),
            exe.clone(),
        );
        let context_capabilities = workspace_app_capabilities(
            &log_sources,
            &endpoints,
            None,
            app_location.as_ref(),
            resources.as_ref(),
            app_state.as_ref(),
        );
        let app_structure = project_path
            .as_deref()
            .map(project_structure)
            .unwrap_or_default();
        let mut app = WorkspaceRuntimeApp {
            pid: Some(process.pid().as_u32()),
            name: app_name,
            display_name: Some(runtime_app_display_name(
                project_path.as_deref(),
                script.as_deref(),
                Some(&name),
            ))
            .filter(|value| !value.is_empty()),
            language: Some(runtime.clone()).filter(|value| value != "native"),
            process_kind: Some(process_kind_for(Some(&command), framework.as_deref())),
            runtime,
            framework,
            libraries,
            log_hints,
            log_sources,
            app_url,
            endpoints,
            health_endpoint: None,
            app_location,
            resources,
            app_state,
            context_capabilities,
            app_structure,
            manager: None,
            status: None,
            cwd,
            script,
            command: Some(command),
            project_path,
            latest_trace_summary: None,
            confidence,
            source: "process".into(),
            signals,
        };
        apply_workspace_manifest(&mut app);
        upsert_runtime_app(&mut apps, app);
    }
    apps
}

fn should_keep_process_runtime_app(
    process_name: &str,
    command: &str,
    project_path: Option<&str>,
    cwd: Option<&str>,
    script: Option<&str>,
    executable: Option<&str>,
    framework: Option<&str>,
    endpoints: &[WorkspaceAppEndpoint],
) -> bool {
    let script_in_project = path_is_within_project(project_path, script);
    let executable_in_project = path_is_within_project(project_path, executable);
    let cwd_in_project = path_is_within_project(project_path, cwd);
    let script_app_signal = script_supports_runtime_app(script);

    if script_in_project || executable_in_project {
        return true;
    }
    if is_utility_process(process_name, command, script, executable) {
        return script_app_signal;
    }
    if framework.is_some() || !endpoints.is_empty() {
        return true;
    }
    script_app_signal && !cwd_in_project
}

fn path_is_within_project(project_path: Option<&str>, candidate: Option<&str>) -> bool {
    let (Some(project_path), Some(candidate)) = (project_path, candidate) else {
        return false;
    };
    let project = PathBuf::from(project_path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(project_path));
    let candidate = PathBuf::from(candidate)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(candidate));
    candidate.starts_with(project)
}

fn is_utility_process(
    process_name: &str,
    command: &str,
    script: Option<&str>,
    executable: Option<&str>,
) -> bool {
    let name = process_name.to_ascii_lowercase();
    let command = command.to_ascii_lowercase();
    let script = script
        .unwrap_or_default()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let executable = executable
        .and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if matches!(
        name.as_str(),
        "git"
            | "git.exe"
            | "cmd"
            | "cmd.exe"
            | "powershell"
            | "powershell.exe"
            | "pwsh"
            | "pwsh.exe"
            | "bash"
            | "bash.exe"
            | "sh"
            | "zsh"
            | "nu"
            | "nu.exe"
            | "cargo"
            | "cargo.exe"
            | "npm"
            | "npm.cmd"
            | "npx"
            | "npx.cmd"
            | "pnpm"
            | "pnpm.cmd"
            | "yarn"
            | "yarn.cmd"
    ) {
        return true;
    }

    if matches!(
        executable.as_str(),
        "git.exe"
            | "cmd.exe"
            | "powershell.exe"
            | "pwsh.exe"
            | "bash.exe"
            | "cargo.exe"
            | "npm.cmd"
            | "npx.cmd"
            | "pnpm.cmd"
            | "yarn.cmd"
    ) {
        return true;
    }

    command.contains(" git ")
        || command.contains(" status --porcelain")
        || script.contains("\\node_modules\\pm2\\lib\\daemon.js")
        || script.ends_with("\\npm-cli.js")
        || script.ends_with("\\npx-cli.js")
        || script.ends_with("\\pnpm.cjs")
        || script.ends_with("\\yarn.js")
}

fn script_supports_runtime_app(script: Option<&str>) -> bool {
    let Some(script) = script else {
        return false;
    };
    let normalized = script.replace('/', "\\").to_ascii_lowercase();
    if script.contains(std::path::MAIN_SEPARATOR) || script.contains('/') || script.contains('\\') {
        return looks_like_script_path(&normalized)
            && !is_utility_process("", "", Some(script), None);
    }
    matches!(
        normalized.as_str(),
        "uvicorn"
            | "gunicorn"
            | "daphne"
            | "celery"
            | "flask"
            | "django"
            | "vite"
            | "next"
            | "nuxt"
            | "nest"
            | "tsx"
            | "ts-node"
    )
}

fn runtime_app_runtime(
    runtime: String,
    project_path: Option<&str>,
    exe: Option<&str>,
) -> Option<String> {
    if runtime != "unknown" {
        return Some(runtime);
    }
    if project_path.is_some() && exe.is_some() {
        return Some("native".into());
    }
    None
}

fn upsert_runtime_app(apps: &mut Vec<WorkspaceRuntimeApp>, app: WorkspaceRuntimeApp) {
    if let Some(existing) = apps.iter_mut().find(|existing| {
        (existing.pid.is_some() && existing.pid == app.pid)
            || (existing.name == app.name && existing.project_path == app.project_path)
    }) {
        if app.confidence > existing.confidence {
            *existing = app;
        }
    } else {
        apps.push(app);
    }
}

fn mapping_from_runtime_app(app: &WorkspaceRuntimeApp) -> Option<WorkspaceMapping> {
    let project_path = app.project_path.clone()?;
    let source = app.manager.clone().unwrap_or_else(|| app.source.clone());
    let mut signals = app.signals.clone();
    signals.push(WorkspaceMappingSignal {
        name: "runtime_app".into(),
        confidence: app.confidence,
        detail: format!("{} app detected from {}", app.runtime, source),
    });
    Some(WorkspaceMapping {
        service_id: app.name.clone(),
        project_path,
        confidence: (app.confidence * 1000.0).round() / 1000.0,
        source,
        notes: app
            .framework
            .as_ref()
            .map(|framework| format!("framework={framework}")),
        signals,
    })
}

fn upsert_workspace_mapping(
    mappings: &mut Vec<WorkspaceMapping>,
    mapping: WorkspaceMapping,
    explicit_keys: &std::collections::HashSet<(String, String)>,
) {
    let key = (mapping.service_id.clone(), mapping.project_path.clone());
    if explicit_keys.contains(&key) {
        return;
    }
    if let Some(existing) = mappings
        .iter_mut()
        .find(|existing| existing.service_id == mapping.service_id)
    {
        if existing.source == "user" || existing.source == "config" {
            return;
        }
        if mapping.confidence > existing.confidence {
            *existing = mapping;
        }
    } else {
        mappings.push(mapping);
    }
}

fn mapping_from_service_tokens(
    service_id: &str,
    projects: &[WorkspaceProject],
) -> Option<WorkspaceMapping> {
    let service_tokens = tokenize_workspace_value(service_id);
    if service_tokens.is_empty() {
        return None;
    }
    let mut best: Option<WorkspaceMapping> = None;
    for project in projects {
        let path_tokens = tokenize_workspace_value(&project.path);
        let overlap: Vec<String> = service_tokens
            .iter()
            .filter(|token| path_tokens.contains(*token))
            .cloned()
            .collect();
        if overlap.is_empty() {
            continue;
        }
        let exact_segment = Path::new(&project.path).components().any(|part| {
            part.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(service_id)
        });
        let mut confidence = (0.25 + 0.2 * overlap.len() as f64).min(0.65);
        if exact_segment {
            confidence = confidence.max(0.7);
        }
        let mut signals = vec![WorkspaceMappingSignal {
            name: "path_token_match".into(),
            confidence,
            detail: format!("shared tokens: {}", overlap.join(",")),
        }];
        if exact_segment {
            signals.push(WorkspaceMappingSignal {
                name: "exact_path_segment".into(),
                confidence: 0.7,
                detail: "service id is a directory segment".into(),
            });
        }
        let mapping = WorkspaceMapping {
            service_id: service_id.to_string(),
            project_path: project.path.clone(),
            confidence: (confidence * 1000.0).round() / 1000.0,
            source: "auto".into(),
            notes: Some(format!("{} marker {}", project.kind, project.marker)),
            signals,
        };
        if best
            .as_ref()
            .map(|current| mapping.confidence > current.confidence)
            .unwrap_or(true)
        {
            best = Some(mapping);
        }
    }
    best
}

fn project_for_paths(
    projects: &[WorkspaceProject],
    cwd: Option<&str>,
    candidate: Option<&str>,
) -> Option<String> {
    let candidates = [cwd, candidate];
    let mut best: Option<&WorkspaceProject> = None;
    for raw in candidates.into_iter().flatten() {
        let path = PathBuf::from(raw);
        let normalized = path.canonicalize().unwrap_or(path);
        for project in projects {
            let project_path = PathBuf::from(&project.path)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&project.path));
            if normalized.starts_with(&project_path)
                && best
                    .as_ref()
                    .map(|existing| project.path.len() > existing.path.len())
                    .unwrap_or(true)
            {
                best = Some(project);
            }
        }
    }
    if let Some(project) = best {
        return Some(project.path.clone());
    }
    for raw in candidates.into_iter().flatten() {
        let path = PathBuf::from(raw);
        if let Some(project) = discover_nearest_workspace_project(&path) {
            return Some(project.path);
        }
    }
    None
}

fn workspace_app_location(
    project_path: Option<String>,
    cwd: Option<String>,
    script: Option<String>,
    executable: Option<String>,
) -> Option<WorkspaceAppLocation> {
    let installation_dir = executable
        .as_deref()
        .and_then(|value| Path::new(value).parent())
        .map(display_path);
    if project_path.is_none() && cwd.is_none() && script.is_none() && executable.is_none() {
        return None;
    }
    Some(WorkspaceAppLocation {
        project_path,
        cwd,
        script,
        executable,
        installation_dir,
    })
}

fn runtime_app_display_name(
    project_path: Option<&str>,
    script: Option<&str>,
    fallback: Option<&str>,
) -> String {
    project_path
        .and_then(|path| Path::new(path).file_name())
        .or_else(|| script.and_then(|path| Path::new(path).file_stem()))
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| fallback.map(str::to_string))
        .unwrap_or_default()
}

fn workspace_log_sources(
    manager: Option<&str>,
    runtime: &str,
    framework: Option<&str>,
    libraries: &[String],
    project_path: Option<&str>,
    cwd: Option<&str>,
    script: Option<&str>,
    pm2_env: Option<&serde_json::Value>,
) -> Vec<WorkspaceLogSource> {
    let mut sources = Vec::new();
    if matches!(manager, Some("pm2")) {
        sources.push(WorkspaceLogSource {
            kind: "manager".into(),
            label: "PM2 logs".into(),
            path: None,
            command: Some("pm2 logs <app>".into()),
            stream: Some("stdout/stderr".into()),
            exists: None,
            readable: None,
            source: "pm2".into(),
            confidence: 0.92,
        });
    } else {
        sources.push(WorkspaceLogSource {
            kind: "stream".into(),
            label: "Process stdout/stderr".into(),
            path: None,
            command: None,
            stream: Some("stdout/stderr".into()),
            exists: None,
            readable: None,
            source: "process".into(),
            confidence: 0.55,
        });
    }
    if let Some(env) = pm2_env {
        for (key, label) in [
            ("pm_out_log_path", "PM2 stdout file"),
            ("pm_err_log_path", "PM2 stderr file"),
            ("out_file", "PM2 stdout file"),
            ("error_file", "PM2 stderr file"),
        ] {
            if let Some(path) = env
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(clean_display_path)
                .filter(|value| !value.trim().is_empty() && value != "/dev/null")
            {
                push_file_log_source(&mut sources, label, &path, "pm2", 0.95);
            }
        }
    }
    if matches!(manager, Some("pm2")) {
        for path in pm2_home_log_candidates(project_path, cwd, script) {
            let label = if path.to_ascii_lowercase().contains("-error") {
                "PM2 stderr file"
            } else {
                "PM2 stdout file"
            };
            push_file_log_source(&mut sources, label, &path, "pm2_home", 0.9);
        }
    } else if matches!(runtime, "nodejs" | "bun" | "deno")
        || matches!(
            framework,
            Some("nextjs" | "nestjs" | "express" | "fastify" | "koa" | "hono")
        )
    {
        for path in pm2_home_log_candidates(project_path, cwd, script) {
            push_file_log_source(
                &mut sources,
                "PM2 archived app log",
                &path,
                "pm2_home",
                0.62,
            );
        }
    }
    if matches!(runtime, "nodejs" | "bun" | "deno")
        || matches!(
            framework,
            Some("nextjs" | "nestjs" | "express" | "fastify" | "koa" | "hono")
        )
    {
        for path in npm_cache_log_candidates() {
            push_file_log_source(&mut sources, "npm cache log", &path, "npm_cache", 0.48);
        }
    }
    for root in [project_path, cwd].into_iter().flatten() {
        for path in discover_project_log_files(root, runtime, framework, libraries, script) {
            push_file_log_source(&mut sources, "Project log file", &path, "project", 0.78);
        }
    }
    sources.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.label.cmp(&right.label))
    });
    sources.dedup_by(|left, right| {
        left.kind == right.kind
            && left.path == right.path
            && left.command == right.command
            && left.stream == right.stream
    });
    sources.truncate(32);
    sources
}

fn push_file_log_source(
    sources: &mut Vec<WorkspaceLogSource>,
    label: &str,
    path: &str,
    source: &str,
    confidence: f64,
) {
    let p = Path::new(path);
    let exists = p.exists();
    let readable = exists && std::fs::File::open(p).is_ok();
    sources.push(WorkspaceLogSource {
        kind: "file".into(),
        label: label.into(),
        path: Some(clean_display_path(path)),
        command: None,
        stream: None,
        exists: Some(exists),
        readable: Some(readable),
        source: source.into(),
        confidence,
    });
}

fn discover_project_log_files(
    root: &str,
    runtime: &str,
    framework: Option<&str>,
    libraries: &[String],
    script: Option<&str>,
) -> Vec<String> {
    let root = Path::new(root);
    let mut candidates = Vec::new();
    for rel in [
        "logs",
        "log",
        "logger",
        "storage/logs",
        "storage/log",
        "var/log",
        "tmp/log",
        "tmp/logs",
        "runtime/logs",
        "run/logs",
        "output/logs",
        ".pm2/logs",
    ] {
        collect_log_files(root.join(rel), &mut candidates);
    }
    for rel in framework_log_candidates(runtime, framework, libraries, script) {
        collect_log_files(root.join(rel), &mut candidates);
    }
    collect_project_wide_log_files(root, &mut candidates);
    candidates.sort();
    candidates.dedup();
    candidates.truncate(24);
    candidates
}

fn collect_project_wide_log_files(root: &Path, out: &mut Vec<String>) {
    collect_log_files_inner(root.to_path_buf(), out, 0);
}

fn collect_log_files(path: PathBuf, out: &mut Vec<String>) {
    if path.is_file() && is_log_file(&path) {
        out.push(display_path(&path));
        return;
    }
    collect_log_files_inner(path, out, 0);
}

fn collect_log_files_inner(path: PathBuf, out: &mut Vec<String>, depth: usize) {
    if depth > 4 || out.len() >= 40 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten().take(48) {
        let p = entry.path();
        if p.is_file() && is_log_file(&p) {
            out.push(display_path(&p));
        } else if p.is_dir()
            && p.file_name()
                .and_then(|value| value.to_str())
                .map(|name| !matches!(name, "node_modules" | ".git" | "target" | ".venv" | "venv"))
                .unwrap_or(true)
        {
            collect_log_files_inner(p, out, depth + 1);
        }
    }
}

fn is_log_file(path: &Path) -> bool {
    let lower = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if lower == ".env" || lower == ".env.local" || lower.starts_with(".env.") {
        return false;
    }
    lower.ends_with(".log")
        || lower.ends_with(".log.json")
        || lower.ends_with(".log.jsonl")
        || lower.ends_with("-debug-0.log")
        || lower.ends_with("-eresolve-report.txt")
        || lower.ends_with(".out")
        || lower.ends_with(".err")
        || lower.ends_with(".stderr")
        || lower.ends_with(".stdout")
        || lower.contains(".log.")
        || (lower.ends_with(".jsonl") && lower.contains("log"))
        || (lower.ends_with(".txt") && lower.contains("log"))
        || matches!(
            lower.as_str(),
            "npm-debug.log"
                | "yarn-error.log"
                | "pnpm-debug.log"
                | "uvicorn.log"
                | "gunicorn.log"
                | "celery.log"
                | "django.log"
                | "flask.log"
                | "fastapi.log"
                | "application.log"
                | "server.log"
                | "app.log"
                | "error.log"
                | "access.log"
                | "trace"
        )
}

fn framework_log_candidates(
    runtime: &str,
    framework: Option<&str>,
    libraries: &[String],
    script: Option<&str>,
) -> Vec<&'static str> {
    let mut out = Vec::new();
    match framework {
        Some("django" | "flask" | "fastapi") => out.extend([
            "app.log",
            "server.log",
            "uvicorn.log",
            "gunicorn.log",
            "logs/app.log",
            "logs/error.log",
            "logs/access.log",
        ]),
        Some("rails") => out.extend(["log/development.log", "log/production.log"]),
        Some("laravel") => out.extend(["storage/logs/laravel.log"]),
        Some("spring") => {
            out.extend(["logs/spring.log", "logs/application.log", "application.log"])
        }
        Some("nextjs" | "vite" | "nuxt" | "nestjs" | "express" | "fastify" | "koa" | "hono") => out
            .extend([
                ".next/trace",
                ".next/diagnostics/build-diagnostics.json",
                "npm-debug.log",
                "yarn-error.log",
                "pnpm-debug.log",
                "logs/app.log",
                "logs/server.log",
                "logs/error.log",
                "logs/access.log",
                "server.log",
                "app.log",
            ]),
        Some("phoenix") => out.extend(["log/dev.log", "log/prod.log"]),
        Some("symfony") => out.extend(["var/log/dev.log", "var/log/prod.log"]),
        _ => {}
    }
    if runtime == "python" {
        out.extend([
            "app.log",
            "error.log",
            "access.log",
            "logs/app.log",
            "logs/error.log",
            "logs/access.log",
        ]);
    }
    if matches!(runtime, "go" | "rust" | "java" | "dotnet") {
        out.extend(["logs/app.log", "logs/error.log", "application.log"]);
    }
    if libraries.iter().any(|item| {
        matches!(
            item.as_str(),
            "winston" | "pino" | "bunyan" | "log4js" | "morgan"
        )
    }) {
        out.extend(["logs/app.log", "logs/error.log", "logs/access.log"]);
    }
    if libraries
        .iter()
        .any(|item| matches!(item.as_str(), "structlog" | "loguru" | "logging"))
    {
        out.extend(["logs/app.log", "logs/error.log", "app.log"]);
    }
    if script
        .map(|value| value.to_ascii_lowercase().contains("celery"))
        .unwrap_or(false)
    {
        out.push("logs/celery.log");
    }
    out
}

fn pm2_home_log_candidates(
    project_path: Option<&str>,
    cwd: Option<&str>,
    script: Option<&str>,
) -> Vec<String> {
    let Some(home) = user_home_dir() else {
        return Vec::new();
    };
    let logs_dir = home.join(".pm2").join("logs");
    let mut names = Vec::new();
    for value in [project_path, cwd, script].into_iter().flatten() {
        if let Some(stem) = Path::new(value)
            .file_stem()
            .or_else(|| Path::new(value).file_name())
            .and_then(|name| name.to_str())
            .map(sanitize_pm2_log_name)
            .filter(|name| !name.is_empty())
        {
            names.push(stem);
        }
    }
    names.sort();
    names.dedup();
    let mut out = Vec::new();
    for name in names {
        for suffix in ["out", "error", "err"] {
            let path = logs_dir.join(format!("{name}-{suffix}.log"));
            if path.is_file() {
                out.push(display_path(&path));
            }
        }
    }
    out
}

fn sanitize_pm2_log_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn npm_cache_log_candidates() -> Vec<String> {
    let mut roots = Vec::new();
    if let Some(value) = std::env::var_os("NPM_CONFIG_CACHE") {
        roots.push(PathBuf::from(value));
    }
    if let Some(value) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(value).join("npm-cache"));
    }
    if let Some(value) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(value).join("npm-cache"));
    }
    if let Some(home) = user_home_dir() {
        roots.push(home.join(".npm"));
    }
    roots.sort();
    roots.dedup();
    let mut files = Vec::new();
    for root in roots {
        let log_dir = root.join("_logs");
        let Ok(entries) = std::fs::read_dir(log_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_log_file(&path) {
                let modified = entry
                    .metadata()
                    .and_then(|meta| meta.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                files.push((modified, display_path(&path)));
            }
        }
    }
    files.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    files.into_iter().map(|(_, path)| path).take(8).collect()
}

fn workspace_app_endpoints(
    command: Option<&str>,
    framework: Option<&str>,
    project_path: Option<&str>,
    pm2_env: Option<&serde_json::Value>,
) -> Vec<WorkspaceAppEndpoint> {
    let mut endpoints = Vec::new();
    if let Some(env) = pm2_env {
        if let Some(url) = env_value(env, &["APP_URL", "URL", "BASE_URL", "PUBLIC_URL"]) {
            endpoints.push(endpoint_from_url(&url, "env", 0.92));
        }
        if let Some(port) = env_port(env) {
            let host = env_value(env, &["HOST", "HOSTNAME"]).unwrap_or_else(|| "127.0.0.1".into());
            endpoints.push(endpoint_from_host_port(&host, port, "env", 0.86));
        }
    }
    if let Some(command) = command {
        if let Some(port) = port_from_command(command) {
            endpoints.push(endpoint_from_host_port("127.0.0.1", port, "command", 0.72));
        }
    }
    if endpoints.is_empty() {
        if let Some(port) = default_port_for_framework(framework, project_path) {
            endpoints.push(endpoint_from_host_port(
                "127.0.0.1",
                port,
                "framework_default",
                0.45,
            ));
        }
    }
    endpoints.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    endpoints.dedup_by(|left, right| left.url == right.url);
    endpoints.truncate(4);
    endpoints
}

fn primary_app_url(endpoints: &[WorkspaceAppEndpoint]) -> Option<String> {
    endpoints
        .iter()
        .find(|endpoint| endpoint.confidence >= 0.7)
        .map(|endpoint| endpoint.url.clone())
}

fn endpoint_from_url(url: &str, source: &str, confidence: f64) -> WorkspaceAppEndpoint {
    let protocol = url.split("://").next().unwrap_or("http").to_string();
    WorkspaceAppEndpoint {
        url: url.to_string(),
        host: None,
        port: None,
        protocol,
        source: source.into(),
        confidence,
    }
}

fn endpoint_from_host_port(
    host: &str,
    port: u16,
    source: &str,
    confidence: f64,
) -> WorkspaceAppEndpoint {
    let host = if host.trim().is_empty() {
        "127.0.0.1"
    } else {
        host.trim()
    };
    WorkspaceAppEndpoint {
        url: format!("http://{host}:{port}"),
        host: Some(host.into()),
        port: Some(port),
        protocol: "http".into(),
        source: source.into(),
        confidence,
    }
}

fn env_value(env: &serde_json::Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = env.get(*key).and_then(serde_json::Value::as_str) {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
        if let Some(value) = env
            .get("env")
            .and_then(|inner| inner.get(*key))
            .and_then(serde_json::Value::as_str)
        {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

fn env_port(env: &serde_json::Value) -> Option<u16> {
    for key in [
        "PORT",
        "SERVER_PORT",
        "APP_PORT",
        "VITE_PORT",
        "NEXT_PORT",
        "FLASK_RUN_PORT",
    ] {
        if let Some(value) = env
            .get(key)
            .or_else(|| env.get("env").and_then(|inner| inner.get(key)))
        {
            if let Some(port) = value.as_u64().and_then(|value| u16::try_from(value).ok()) {
                return Some(port);
            }
            if let Some(port) = value
                .as_str()
                .and_then(|raw| raw.trim().parse::<u16>().ok())
            {
                return Some(port);
            }
        }
    }
    None
}

fn port_from_command(command: &str) -> Option<u16> {
    let parts = split_command_like(command);
    for (idx, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        if matches!(lower.as_str(), "--port" | "--http-port" | "--listen") {
            if let Some(port) = parts.get(idx + 1).and_then(|next| parse_port_token(next)) {
                return Some(port);
            }
        }
        if let Some(raw) = lower
            .strip_prefix("--port=")
            .or_else(|| lower.strip_prefix("--http-port="))
        {
            if let Some(port) = parse_port_token(raw) {
                return Some(port);
            }
        }
        if let Some(port) = parse_host_port_token(&lower) {
            return Some(port);
        }
    }
    None
}

fn split_command_like(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .map(|part| part.trim_matches('"').trim_matches('\'').to_string())
        .collect()
}

fn parse_port_token(token: &str) -> Option<u16> {
    token
        .trim()
        .trim_start_matches(':')
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse::<u16>().ok())
}

fn parse_host_port_token(token: &str) -> Option<u16> {
    if !(token.starts_with("http://")
        || token.starts_with("https://")
        || token.starts_with("localhost:")
        || token.starts_with("127.0.0.1:"))
    {
        return None;
    }
    let pos = token.rfind(':')?;
    parse_port_token(&token[pos + 1..]).filter(|port| *port >= 1024 || matches!(*port, 80 | 443))
}

fn default_port_for_framework(framework: Option<&str>, project_path: Option<&str>) -> Option<u16> {
    match framework {
        Some("nextjs" | "nestjs" | "rails") => Some(3000),
        Some("vite") => Some(5173),
        Some("flask") => Some(5000),
        Some("django" | "fastapi" | "uvicorn" | "laravel") => Some(8000),
        Some("spring") => Some(8080),
        _ => project_path.and_then(project_default_port),
    }
}

fn project_default_port(project_path: &str) -> Option<u16> {
    let package = Path::new(project_path).join("package.json");
    if package.exists() {
        return Some(3000);
    }
    let pyproject = Path::new(project_path).join("pyproject.toml");
    if pyproject.exists() {
        return Some(8000);
    }
    None
}

fn apply_workspace_manifest(app: &mut WorkspaceRuntimeApp) {
    let Some(project_path) = app.project_path.clone() else {
        return;
    };
    let deps = project_dependency_hints(&project_path);
    extend_unique(&mut app.libraries, deps.libraries);
    if app.framework.is_none() {
        app.framework = deps.framework;
    }
    if app.language.is_none() {
        app.language = deps.language.clone();
    }
    if app.runtime == "unknown" {
        if let Some(language) = deps.language {
            app.runtime = language;
        }
    }
    if app.app_structure.is_empty() {
        app.app_structure = project_structure(&project_path);
    }
    for root in [Some(project_path.as_str()), app.cwd.as_deref()]
        .into_iter()
        .flatten()
    {
        for path in discover_project_log_files(
            root,
            &app.runtime,
            app.framework.as_deref(),
            &app.libraries,
            app.script.as_deref(),
        ) {
            upsert_log_source(
                &mut app.log_sources,
                WorkspaceLogSource {
                    kind: "file".into(),
                    label: "Project log file".into(),
                    path: Some(path),
                    command: None,
                    stream: None,
                    exists: Some(true),
                    readable: Some(true),
                    source: "project_dependency_scan".into(),
                    confidence: 0.82,
                },
            );
        }
    }
    if let Some(manifest) = read_inferra_app_manifest(&project_path) {
        apply_manifest_value(app, &project_path, &manifest);
    }
    app.log_hints = runtime_log_hints(
        &app.runtime,
        app.framework.as_deref(),
        &app.libraries,
        app.manager.as_deref(),
    );
    app.context_capabilities = workspace_app_capabilities(
        &app.log_sources,
        &app.endpoints,
        app.health_endpoint.as_ref(),
        app.app_location.as_ref(),
        app.resources.as_ref(),
        app.app_state.as_ref(),
    );
}

#[derive(Default)]
struct ProjectDependencyHints {
    language: Option<String>,
    framework: Option<String>,
    libraries: Vec<String>,
}

fn project_dependency_hints(project_path: &str) -> ProjectDependencyHints {
    let mut hints = ProjectDependencyHints::default();
    let project = Path::new(project_path);
    let package = project.join("package.json");
    if let Ok(text) = std::fs::read_to_string(package) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            hints.language = Some("nodejs".into());
            let deps = package_dependency_names(&value);
            hints.framework = node_framework_from_deps(&deps);
            hints
                .libraries
                .extend(deps.into_iter().filter_map(|dep| node_package_signal(&dep)));
        }
    }
    let pyproject = project.join("pyproject.toml");
    if let Ok(text) = std::fs::read_to_string(pyproject) {
        if let Ok(value) = text.parse::<TomlValue>() {
            hints.language.get_or_insert_with(|| "python".into());
            let deps = toml_dependency_names(&value);
            hints.framework = hints
                .framework
                .or_else(|| python_framework_from_deps(&deps));
            hints.libraries.extend(
                deps.into_iter()
                    .filter_map(|dep| python_package_signal(&dep)),
            );
        }
    }
    for req in [
        "requirements.txt",
        "requirements/base.txt",
        "requirements/dev.txt",
    ] {
        if let Ok(text) = std::fs::read_to_string(project.join(req)) {
            hints.language.get_or_insert_with(|| "python".into());
            let deps = requirements_dependency_names(&text);
            hints.framework = hints
                .framework
                .or_else(|| python_framework_from_deps(&deps));
            hints.libraries.extend(
                deps.into_iter()
                    .filter_map(|dep| python_package_signal(&dep)),
            );
        }
    }
    if project.join("Cargo.toml").exists() {
        hints.language.get_or_insert_with(|| "rust".into());
        if let Ok(text) = std::fs::read_to_string(project.join("Cargo.toml")) {
            if let Ok(value) = text.parse::<TomlValue>() {
                let deps = toml_dependency_names(&value);
                hints.framework = hints.framework.or_else(|| rust_framework_from_deps(&deps));
                hints
                    .libraries
                    .extend(deps.into_iter().filter_map(|dep| rust_package_signal(&dep)));
            }
        }
    }
    hints.libraries.sort();
    hints.libraries.dedup();
    hints
}

fn package_dependency_names(value: &serde_json::Value) -> Vec<String> {
    let mut names = Vec::new();
    for section in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
    ] {
        if let Some(obj) = value.get(section).and_then(serde_json::Value::as_object) {
            names.extend(obj.keys().map(|key| key.to_ascii_lowercase()));
        }
    }
    names
}

fn toml_dependency_names(value: &TomlValue) -> Vec<String> {
    let mut names = Vec::new();
    collect_toml_dependency_names(value, &mut names);
    names.sort();
    names.dedup();
    names
}

fn collect_toml_dependency_names(value: &TomlValue, names: &mut Vec<String>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, value) in table {
        if key.to_ascii_lowercase().contains("dependencies") {
            if let Some(dep_table) = value.as_table() {
                names.extend(dep_table.keys().map(|name| name.to_ascii_lowercase()));
            }
            if let Some(arr) = value.as_array() {
                names.extend(
                    arr.iter()
                        .filter_map(TomlValue::as_str)
                        .filter_map(|raw| raw.split(['=', '<', '>', '~', ' ']).next())
                        .map(|name| name.trim().to_ascii_lowercase())
                        .filter(|name| !name.is_empty()),
                );
            }
        }
        collect_toml_dependency_names(value, names);
    }
}

fn requirements_dependency_names(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split(['=', '<', '>', '~', '[', ';', ' ']).next())
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect()
}

fn node_framework_from_deps(deps: &[String]) -> Option<String> {
    for (dep, framework) in [
        ("next", "nextjs"),
        ("@nestjs/core", "nestjs"),
        ("vite", "vite"),
        ("nuxt", "nuxt"),
        ("express", "express"),
        ("fastify", "fastify"),
        ("koa", "koa"),
        ("hono", "hono"),
        ("@hapi/hapi", "hapi"),
        ("@remix-run/node", "remix"),
        ("astro", "astro"),
        ("@sveltejs/kit", "sveltekit"),
        ("apollo-server", "apollo"),
        ("graphql-yoga", "graphql-yoga"),
    ] {
        if deps.iter().any(|item| item == dep) {
            return Some(framework.into());
        }
    }
    None
}

fn python_framework_from_deps(deps: &[String]) -> Option<String> {
    for (dep, framework) in [
        ("fastapi", "fastapi"),
        ("django", "django"),
        ("flask", "flask"),
        ("celery", "celery"),
        ("uvicorn", "uvicorn"),
        ("gunicorn", "gunicorn"),
        ("starlette", "starlette"),
        ("litestar", "litestar"),
        ("starlite", "litestar"),
        ("tornado", "tornado"),
        ("sanic", "sanic"),
        ("aiohttp", "aiohttp"),
        ("rq", "rq"),
        ("dramatiq", "dramatiq"),
    ] {
        if deps.iter().any(|item| item == dep) {
            return Some(framework.into());
        }
    }
    None
}

fn rust_framework_from_deps(deps: &[String]) -> Option<String> {
    for (dep, framework) in [
        ("axum", "axum"),
        ("actix-web", "actix-web"),
        ("rocket", "rocket"),
        ("warp", "warp"),
        ("poem", "poem"),
        ("salvo", "salvo"),
        ("tonic", "tonic"),
    ] {
        if deps.iter().any(|item| item == dep) {
            return Some(framework.into());
        }
    }
    None
}

fn node_package_signal(dep: &str) -> Option<String> {
    match dep {
        "express" | "fastify" | "koa" | "@hapi/hapi" | "hono" | "apollo-server"
        | "graphql-yoga" => Some(dep.to_string()),
        "next" => Some("nextjs".into()),
        "@nestjs/core" => Some("nestjs".into()),
        "@remix-run/node" => Some("remix".into()),
        "@sveltejs/kit" => Some("sveltekit".into()),
        "nuxt" | "vite" | "astro" | "sveltekit" | "webpack" | "esbuild" | "tsx" | "ts-node" => {
            Some(dep.to_string())
        }
        "pino" | "pino-http" | "winston" | "morgan" | "debug" | "bunyan" | "log4js"
        | "koa-logger" => Some(dep.to_string()),
        "prisma" | "@prisma/client" => Some("prisma".into()),
        "typeorm" | "sequelize" | "mongoose" | "knex" | "drizzle-orm" | "mikro-orm" => {
            Some(dep.to_string())
        }
        "pg" | "mysql2" | "mongodb" | "redis" | "ioredis" | "amqplib" | "kafkajs" => {
            Some(dep.to_string())
        }
        "bullmq" | "bull" | "bee-queue" | "agenda" => Some(dep.to_string()),
        "socket.io" | "ws" | "graphql" | "axios" | "got" | "undici" | "superagent" => {
            Some(dep.to_string())
        }
        "prom-client" | "@opentelemetry/api" | "@opentelemetry/sdk-node" | "@sentry/node" => {
            Some(dep.to_string())
        }
        _ => None,
    }
}

fn python_package_signal(dep: &str) -> Option<String> {
    match dep {
        "fastapi" | "django" | "flask" | "celery" | "uvicorn" | "gunicorn" | "starlette"
        | "litestar" | "starlite" | "tornado" | "sanic" | "aiohttp" | "rq" | "dramatiq" => {
            Some(dep.to_string())
        }
        "structlog" | "loguru" | "sentry-sdk" | "opentelemetry-api" | "opentelemetry-sdk" => {
            Some(dep.to_string())
        }
        "sqlalchemy" | "psycopg2" | "psycopg" | "asyncpg" | "pymongo" | "redis" | "aioredis"
        | "celery-redbeat" | "kombu" | "pika" | "confluent-kafka" => Some(dep.to_string()),
        _ => None,
    }
}

fn rust_package_signal(dep: &str) -> Option<String> {
    match dep {
        "tracing" | "log" | "env_logger" | "slog" | "tracing-subscriber" => Some(dep.to_string()),
        "axum" | "actix-web" | "rocket" | "warp" | "poem" | "salvo" => Some(dep.to_string()),
        "sqlx" | "diesel" | "redis" | "mongodb" | "tonic" | "opentelemetry" | "sentry"
        | "lapin" | "rdkafka" => Some(dep.to_string()),
        _ => None,
    }
}

fn read_inferra_app_manifest(project_path: &str) -> Option<TomlValue> {
    let root = Path::new(project_path).join(".inferra");
    for name in ["app.toml", "inferra.toml", "workspace.toml"] {
        let path = root.join(name);
        if path.is_file() {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(value) = text.parse::<TomlValue>() {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn apply_manifest_value(app: &mut WorkspaceRuntimeApp, project_path: &str, manifest: &TomlValue) {
    let app_table = manifest.get("app").unwrap_or(manifest);
    if let Some(name) = toml_string(app_table, "name") {
        app.display_name = Some(name);
    }
    if let Some(runtime) = toml_string(app_table, "runtime") {
        app.runtime = runtime.clone();
        app.language = Some(runtime);
    }
    if let Some(framework) = toml_string(app_table, "framework") {
        app.framework = Some(framework);
    }
    if let Some(process_kind) = toml_string(app_table, "process_kind") {
        app.process_kind = Some(process_kind);
    }
    if let Some(url) = toml_string(app_table, "url").or_else(|| toml_string(app_table, "app_url")) {
        let endpoint = endpoint_from_url(&url, "inferra_manifest", 1.0);
        app.app_url = Some(endpoint.url.clone());
        upsert_endpoint(&mut app.endpoints, endpoint);
    }
    if let Some(health) = manifest.get("health").or_else(|| manifest.get("heartbeat")) {
        if let Some(endpoint) = manifest_health_endpoint(health, app.app_url.as_deref()) {
            app.health_endpoint = Some(endpoint.clone());
            upsert_endpoint(&mut app.endpoints, endpoint);
        }
    }
    if let Some(items) = manifest.get("endpoints").and_then(TomlValue::as_array) {
        for item in items {
            if let Some(endpoint) = manifest_endpoint(item) {
                upsert_endpoint(&mut app.endpoints, endpoint);
            }
        }
    }
    if let Some(items) = manifest.get("logs").and_then(TomlValue::as_array) {
        for item in items {
            if let Some(source) = manifest_log_source(project_path, item) {
                upsert_log_source(&mut app.log_sources, source);
            }
        }
    }
    if let Some(table) = manifest.get("logs").and_then(TomlValue::as_table) {
        if let Some(source) = manifest_log_source(project_path, &TomlValue::Table(table.clone())) {
            upsert_log_source(&mut app.log_sources, source);
        }
    }
    if let Some(resources) = manifest.get("resources") {
        if let Some(process_name) = toml_string(resources, "process_name") {
            app.context_capabilities.push(WorkspaceAppCapability {
                key: "resource_match".into(),
                supported: true,
                source: "inferra_manifest".into(),
                detail: Some(process_name),
            });
        }
    }
    app.signals.push(WorkspaceMappingSignal {
        name: "inferra_manifest".into(),
        confidence: 1.0,
        detail: ".inferra/app.toml provided explicit workspace metadata".into(),
    });
    app.endpoints.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    app.app_url = app
        .endpoints
        .iter()
        .find(|endpoint| endpoint.confidence >= 0.7)
        .map(|endpoint| endpoint.url.clone());
}

fn toml_string(value: &TomlValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(TomlValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn manifest_endpoint(value: &TomlValue) -> Option<WorkspaceAppEndpoint> {
    if let Some(url) = toml_string(value, "url") {
        return Some(endpoint_from_url(&url, "inferra_manifest", 1.0));
    }
    let port = value
        .get("port")
        .and_then(TomlValue::as_integer)
        .and_then(|value| u16::try_from(value).ok())?;
    let host = toml_string(value, "host").unwrap_or_else(|| "127.0.0.1".into());
    Some(endpoint_from_host_port(
        &host,
        port,
        "inferra_manifest",
        1.0,
    ))
}

fn manifest_health_endpoint(
    value: &TomlValue,
    app_url: Option<&str>,
) -> Option<WorkspaceAppEndpoint> {
    if let Some(url) = toml_string(value, "url") {
        return Some(endpoint_from_url(&url, "inferra_manifest_health", 1.0));
    }
    if let Some(path) = toml_string(value, "path") {
        if let Some(base) = app_url {
            return Some(endpoint_from_url(
                &format!(
                    "{}{}",
                    base.trim_end_matches('/'),
                    ensure_leading_slash(&path)
                ),
                "inferra_manifest_health",
                1.0,
            ));
        }
    }
    manifest_endpoint(value).map(|mut endpoint| {
        endpoint.source = "inferra_manifest_health".into();
        endpoint
    })
}

fn ensure_leading_slash(value: &str) -> String {
    if value.starts_with('/') {
        value.to_string()
    } else {
        format!("/{value}")
    }
}

fn manifest_log_source(project_path: &str, value: &TomlValue) -> Option<WorkspaceLogSource> {
    let path =
        toml_string(value, "path").map(|raw| resolve_project_manifest_path(project_path, &raw));
    let command = toml_string(value, "command");
    let stream = toml_string(value, "stream");
    if path.is_none() && command.is_none() && stream.is_none() {
        return None;
    }
    let kind = toml_string(value, "kind")
        .unwrap_or_else(|| if path.is_some() { "file" } else { "stream" }.into());
    let label = toml_string(value, "label").unwrap_or_else(|| "Inferra manifest log source".into());
    let (exists, readable) = path
        .as_deref()
        .map(|path| {
            let p = Path::new(path);
            (
                Some(p.exists()),
                Some(p.exists() && std::fs::File::open(p).is_ok()),
            )
        })
        .unwrap_or((None, None));
    Some(WorkspaceLogSource {
        kind,
        label,
        path,
        command,
        stream,
        exists,
        readable,
        source: "inferra_manifest".into(),
        confidence: 1.0,
    })
}

fn resolve_project_manifest_path(project_path: &str, raw: &str) -> String {
    let path = Path::new(raw);
    if path.is_absolute() {
        clean_display_path(raw)
    } else {
        display_path(&Path::new(project_path).join(path))
    }
}

fn upsert_endpoint(endpoints: &mut Vec<WorkspaceAppEndpoint>, endpoint: WorkspaceAppEndpoint) {
    if let Some(existing) = endpoints.iter_mut().find(|item| item.url == endpoint.url) {
        if endpoint.confidence > existing.confidence {
            *existing = endpoint;
        }
    } else {
        endpoints.push(endpoint);
    }
}

fn upsert_log_source(sources: &mut Vec<WorkspaceLogSource>, source: WorkspaceLogSource) {
    if let Some(existing) = sources.iter_mut().find(|item| {
        item.kind == source.kind
            && item.path == source.path
            && item.command == source.command
            && item.stream == source.stream
    }) {
        if source.source == "inferra_manifest" || source.confidence > existing.confidence {
            *existing = source;
        }
        return;
    }
    sources.push(source);
}

fn project_structure(project_path: &str) -> Vec<WorkspaceAppStructureItem> {
    let root = Path::new(project_path);
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten().take(80) {
        let path = entry.path();
        let Some(file_name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        if should_hide_project_structure_entry(&file_name) {
            continue;
        }
        out.push(WorkspaceAppStructureItem {
            path: file_name.clone(),
            kind: if path.is_dir() { "directory" } else { "file" }.into(),
            role: project_structure_role(&file_name, path.is_dir()),
        });
    }
    out.sort_by(|left, right| left.path.cmp(&right.path));
    out.truncate(40);
    out
}

fn should_hide_project_structure_entry(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower == ".env.local"
        || lower.starts_with(".env.")
        || matches!(
            lower.as_str(),
            ".git"
                | "node_modules"
                | "target"
                | ".venv"
                | "venv"
                | "__pycache__"
                | ".next"
                | "dist"
                | "build"
        )
}

fn project_structure_role(name: &str, is_dir: bool) -> String {
    let lower = name.to_ascii_lowercase();
    if lower == ".inferra" {
        "inferra_config".into()
    } else if matches!(
        lower.as_str(),
        "package.json" | "pyproject.toml" | "cargo.toml" | "composer.json" | "pom.xml" | "go.mod"
    ) {
        "manifest".into()
    } else if matches!(
        lower.as_str(),
        "src" | "app" | "pages" | "api" | "server" | "cmd" | "crates"
    ) && is_dir
    {
        "source".into()
    } else if matches!(lower.as_str(), "logs" | "log" | "storage") && is_dir {
        "logs".into()
    } else if matches!(
        lower.as_str(),
        "dockerfile" | "docker-compose.yml" | "compose.yml"
    ) {
        "runtime".into()
    } else {
        "project".into()
    }
}

fn extend_unique(target: &mut Vec<String>, items: Vec<String>) {
    for item in items {
        if !target.iter().any(|existing| existing == &item) {
            target.push(item);
        }
    }
    target.sort();
    target.dedup();
}

fn pm2_resources(env: &serde_json::Value) -> Option<WorkspaceAppResources> {
    let monit = env.get("monit").unwrap_or(&serde_json::Value::Null);
    let logical_processors = host_logical_processors();
    let raw_cpu_percent = monit
        .get("cpu")
        .and_then(serde_json::Value::as_f64)
        .map(|v| round_f64(v, 2));
    let cpu_percent =
        raw_cpu_percent.map(|v| normalize_process_cpu_to_host_percent(v, logical_processors));
    let memory_mb = monit
        .get("memory")
        .and_then(serde_json::Value::as_f64)
        .map(|value| round_f64(value / (1024.0 * 1024.0), 2));
    if cpu_percent.is_none() && memory_mb.is_none() {
        return None;
    }
    Some(WorkspaceAppResources {
        cpu_percent,
        cpu_raw_percent: raw_cpu_percent,
        cpu_percent_scope: cpu_percent.map(|_| "host_total".into()),
        cpu_logical_processors: cpu_percent.map(|_| logical_processors),
        memory_mb,
        virtual_memory_mb: None,
        uptime_seconds: None,
        process_status: env
            .get("status")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

fn pm2_app_state(env: &serde_json::Value, status: Option<String>) -> Option<WorkspaceAppState> {
    let restarts = env
        .get("restart_time")
        .or_else(|| env.get("unstable_restarts"))
        .and_then(serde_json::Value::as_u64);
    let started_at = env
        .get("pm_uptime")
        .and_then(serde_json::Value::as_i64)
        .map(|value| format!("unix_ms:{value}"));
    Some(WorkspaceAppState {
        health: match status.as_deref() {
            Some("online") => "running".into(),
            Some("stopped" | "errored") => "degraded".into(),
            Some(_) => "unknown".into(),
            None => "unknown".into(),
        },
        status,
        reason: Some("Reported by PM2 jlist".into()),
        started_at,
        restarts,
        observed_by: "pm2".into(),
    })
}

fn workspace_app_capabilities(
    log_sources: &[WorkspaceLogSource],
    endpoints: &[WorkspaceAppEndpoint],
    health_endpoint: Option<&WorkspaceAppEndpoint>,
    location: Option<&WorkspaceAppLocation>,
    resources: Option<&WorkspaceAppResources>,
    state: Option<&WorkspaceAppState>,
) -> Vec<WorkspaceAppCapability> {
    vec![
        WorkspaceAppCapability {
            key: "logs".into(),
            supported: !log_sources.is_empty(),
            source: "workspace_detector".into(),
            detail: Some(format!("{} log source(s) discovered or inferred", log_sources.len())),
        },
        WorkspaceAppCapability {
            key: "app_state".into(),
            supported: state.is_some(),
            source: "workspace_detector".into(),
            detail: state.and_then(|value| value.status.clone()),
        },
        WorkspaceAppCapability {
            key: "app_url".into(),
            supported: !endpoints.is_empty(),
            source: "workspace_detector".into(),
            detail: endpoints.first().map(|value| value.url.clone()),
        },
        WorkspaceAppCapability {
            key: "heartbeat".into(),
            supported: health_endpoint.is_some(),
            source: "workspace_detector".into(),
            detail: health_endpoint.map(|value| value.url.clone()),
        },
        WorkspaceAppCapability {
            key: "app_location".into(),
            supported: location.is_some(),
            source: "workspace_detector".into(),
            detail: location
                .and_then(|value| value.project_path.clone().or_else(|| value.cwd.clone())),
        },
        WorkspaceAppCapability {
            key: "resources".into(),
            supported: resources.is_some(),
            source: "workspace_detector".into(),
            detail: resources.and_then(|value| {
                value
                    .memory_mb
                    .map(|mb| format!("memory {mb:.1} MB"))
                    .or_else(|| value.cpu_percent.map(|cpu| format!("host cpu {cpu:.1}%")))
            }),
        },
        WorkspaceAppCapability {
            key: "ai_context".into(),
            supported: true,
            source: "workspace_detector".into(),
            detail: Some("Logs, runtime state, location, endpoints, resources, and detection signals are forwarded to AI when available.".into()),
        },
    ]
}

fn round_f64(value: f64, decimals: u32) -> f64 {
    let factor = 10_f64.powi(decimals as i32);
    (value * factor).round() / factor
}

fn host_logical_processors() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .max(1)
}

fn system_logical_processors(sys: &System) -> usize {
    sys.cpus().len().max(host_logical_processors()).max(1)
}

fn normalize_process_cpu_to_host_percent(raw_percent: f64, logical_processors: usize) -> f64 {
    let processors = logical_processors.max(1) as f64;
    round_f64((raw_percent / processors).clamp(0.0, 100.0), 2)
}

fn classify_runtime(name: &str, command: &str, script: Option<&str>) -> String {
    let haystack = format!(
        "{} {} {}",
        name.to_ascii_lowercase(),
        command.to_ascii_lowercase(),
        script.unwrap_or_default().to_ascii_lowercase()
    );
    match_signature(&haystack, LANGUAGE_SIGNATURES)
        .map(|signature| signature.id.to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn detect_framework(command: &str, script: Option<&str>) -> Option<String> {
    let haystack = format!(
        "{} {}",
        command.to_ascii_lowercase(),
        script.unwrap_or_default().to_ascii_lowercase()
    );
    match_signature(&haystack, FRAMEWORK_SIGNATURES).map(|signature| signature.id.to_string())
}

fn detect_libraries(command: &str, script: Option<&str>) -> Vec<String> {
    let haystack = format!(
        "{} {}",
        command.to_ascii_lowercase(),
        script.unwrap_or_default().to_ascii_lowercase()
    );
    LIBRARY_SIGNATURES
        .iter()
        .filter(|signature| contains_any(&haystack, signature.detects))
        .map(|signature| signature.id.to_string())
        .collect()
}

fn runtime_log_hints(
    runtime: &str,
    framework: Option<&str>,
    libraries: &[String],
    manager: Option<&str>,
) -> Vec<String> {
    let mut hints = Vec::new();
    if let Some(signature) = LANGUAGE_SIGNATURES
        .iter()
        .find(|signature| signature.id == runtime)
    {
        hints.extend(signature.log_hints.iter().map(|hint| (*hint).to_string()));
    }
    if let Some(framework) = framework {
        if let Some(signature) = FRAMEWORK_SIGNATURES
            .iter()
            .find(|signature| signature.id == framework)
        {
            hints.extend(signature.log_hints.iter().map(|hint| (*hint).to_string()));
        }
    }
    for library in libraries {
        if let Some(signature) = LIBRARY_SIGNATURES
            .iter()
            .find(|signature| signature.id == library)
        {
            hints.extend(signature.log_hints.iter().map(|hint| (*hint).to_string()));
        }
    }
    if let Some(manager) = manager {
        if let Some(signature) = PROCESS_SIGNATURES
            .iter()
            .find(|signature| signature.id == manager)
        {
            hints.extend(signature.log_hints.iter().map(|hint| (*hint).to_string()));
        }
    }
    hints.sort();
    hints.dedup();
    hints
}

fn process_kind_for(command: Option<&str>, framework: Option<&str>) -> String {
    let haystack = command.unwrap_or_default().to_ascii_lowercase();
    if matches!(framework, Some("celery" | "sidekiq"))
        || contains_any(&haystack, &["worker", "queue"])
    {
        "worker".into()
    } else if matches!(framework, Some("vite")) || contains_any(&haystack, &[" dev", "vite"]) {
        "dev_server".into()
    } else if framework.is_some()
        || contains_any(
            &haystack,
            &[
                "server",
                "serve",
                "uvicorn",
                "gunicorn",
                "next start",
                "listen",
            ],
        )
    {
        "server".into()
    } else if contains_any(&haystack, &["test", "pytest", "vitest", "jest"]) {
        "test_runner".into()
    } else {
        "application".into()
    }
}

fn match_signature<'a>(
    haystack: &str,
    signatures: &'a [WorkspaceSignature],
) -> Option<&'a WorkspaceSignature> {
    signatures
        .iter()
        .find(|signature| contains_any(haystack, signature.detects))
}

fn infer_script_path(cmd: &[String], cwd: Option<&str>) -> Option<String> {
    for (idx, arg) in cmd.iter().enumerate() {
        let lower = arg.to_ascii_lowercase();
        if idx == 0 {
            continue;
        }
        if lower == "-m" {
            return cmd.get(idx + 1).cloned();
        }
        if lower.starts_with('-') {
            continue;
        }
        if matches!(
            lower.as_str(),
            "uvicorn"
                | "gunicorn"
                | "daphne"
                | "celery"
                | "flask"
                | "django"
                | "vite"
                | "next"
                | "nuxt"
                | "nest"
                | "tsx"
                | "ts-node"
        ) {
            return Some(arg.clone());
        }
        if looks_like_script_path(&lower) {
            let path = PathBuf::from(arg);
            let resolved = if path.is_absolute() {
                path
            } else if let Some(cwd) = cwd {
                PathBuf::from(cwd).join(path)
            } else {
                path
            };
            return Some(display_path(&resolved));
        }
    }
    None
}

fn looks_like_script_path(value: &str) -> bool {
    value.ends_with(".py")
        || value.ends_with(".js")
        || value.ends_with(".mjs")
        || value.ends_with(".cjs")
        || value.ends_with(".ts")
        || value.ends_with(".tsx")
        || value.ends_with(".jsx")
        || value.ends_with(".rb")
        || value.ends_with(".php")
        || value.ends_with(".jar")
        || value.ends_with(".dll")
        || value.ends_with(".exe")
        || value.ends_with("manage.py")
}

fn runtime_app_name(
    process_name: &str,
    script: Option<&str>,
    project_path: Option<&str>,
) -> String {
    if let Some(project_path) = project_path {
        if let Some(name) = project_manifest_name(Path::new(project_path)) {
            return name;
        }
        if let Some(name) = Path::new(project_path)
            .file_name()
            .and_then(|value| value.to_str())
        {
            return name.to_string();
        }
    }
    if let Some(script) = script {
        if !script.contains('.') && !script.contains(std::path::MAIN_SEPARATOR) {
            return script.to_string();
        }
        if let Some(stem) = Path::new(script)
            .file_stem()
            .and_then(|value| value.to_str())
        {
            return stem.to_string();
        }
    }
    process_name.to_string()
}

fn project_manifest_name(project_path: &Path) -> Option<String> {
    let package_json = project_path.join("package.json");
    if let Ok(text) = std::fs::read_to_string(&package_json) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(name) = value.get("name").and_then(serde_json::Value::as_str) {
                if !name.trim().is_empty() {
                    return Some(name.trim().to_string());
                }
            }
        }
    }
    let pyproject = project_path.join("pyproject.toml");
    if let Ok(text) = std::fs::read_to_string(&pyproject) {
        if let Ok(value) = text.parse::<TomlValue>() {
            if let Some(name) = value
                .get("project")
                .and_then(|project| project.get("name"))
                .and_then(TomlValue::as_str)
            {
                if !name.trim().is_empty() {
                    return Some(name.trim().to_string());
                }
            }
            if let Some(name) = value
                .get("tool")
                .and_then(|tool| tool.get("poetry"))
                .and_then(|poetry| poetry.get("name"))
                .and_then(TomlValue::as_str)
            {
                if !name.trim().is_empty() {
                    return Some(name.trim().to_string());
                }
            }
        }
    }
    let cargo = project_path.join("Cargo.toml");
    if let Ok(text) = std::fs::read_to_string(&cargo) {
        if let Ok(value) = text.parse::<TomlValue>() {
            if let Some(name) = value
                .get("package")
                .and_then(|package| package.get("name"))
                .and_then(TomlValue::as_str)
            {
                if !name.trim().is_empty() {
                    return Some(name.trim().to_string());
                }
            }
        }
    }
    let composer = project_path.join("composer.json");
    if let Ok(text) = std::fs::read_to_string(&composer) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(name) = value.get("name").and_then(serde_json::Value::as_str) {
                if !name.trim().is_empty() {
                    return Some(name.trim().to_string());
                }
            }
        }
    }
    None
}

fn tokenize_workspace_value(value: &str) -> std::collections::HashSet<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_ascii_lowercase();
            if token.len() >= 3 {
                Some(token)
            } else {
                None
            }
        })
        .collect()
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn disk_free_near(path: &Path) -> Option<u64> {
    let disks = Disks::new_with_refreshed_list();
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut best: Option<(usize, u64)> = None;
    for disk in disks.list() {
        let mp = disk.mount_point();
        if canonical.starts_with(mp) {
            let score = mp.as_os_str().len();
            let avail = disk.available_space();
            if best.map(|(s, _)| score > s).unwrap_or(true) {
                best = Some((score, avail));
            }
        }
    }
    best.map(|(_, b)| b)
}

#[cfg(test)]
mod unit_tests;
