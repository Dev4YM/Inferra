//! Native overview assembly for the Rust runtime.

mod resource_snapshot;

pub use resource_snapshot::{
    collect_host_resources_snapshot, collect_runtime_monitor_window, try_collect_gpu_summary,
};

use anyhow::Result;
use inferra_config::{experience_from_config, Paths};
use inferra_contracts::{
    AiStatusResponse, DashboardHealth, DashboardPayload, EventRow, IncidentRow, OverviewResponse,
    QuickAnalysis, RuntimeContext, RuntimeProcess, ServiceRow, SeverityValue, WorkspaceMapResponse,
    WorkspaceMapping, WorkspaceMappingSignal, WorkspaceProject,
};
use inferra_storage::{
    EventsStore, GovernanceSummary, IncidentRecord, IncidentsStore, ServiceStats, StoredHypothesis,
};
use std::path::Path;
use sysinfo::{Disks, System};
use toml::Value as TomlValue;
use walkdir::WalkDir;

const SEVERITY_INFO: i64 = 1;
const SEVERITY_WARN: i64 = 2;
const SEVERITY_ERROR: i64 = 3;

/// Build `/api/overview` from local SQLite + lightweight host snapshot.
pub fn build_overview(config: &TomlValue, paths: &Paths) -> Result<OverviewResponse> {
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

    let services = enrich_service_rows(&service_stats, &incidents);

    let storage_ok = paths.data_dir.exists();
    let mut degraded_reasons: Vec<String> = Vec::new();
    if !storage_ok {
        degraded_reasons.push("data directory does not exist yet".into());
    }

    let free_bytes = disk_free_near(&paths.data_dir);

    let mut sys = System::new_all();
    sys.refresh_all();
    let hostname = System::host_name();
    let mut processes: Vec<RuntimeProcess> = sys
        .processes()
        .values()
        .map(|p| {
            let mem = p.memory() as f64 / (1024.0 * 1024.0);
            RuntimeProcess {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_percent: f64::from(p.cpu_usage()),
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
        containers: None,
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
        summary_parts
            .push("AI enabled; availability is checked by the native provider probe.".into());
    }

    let risk = if degraded || incidents_n > 0 {
        "high"
    } else {
        "low"
    };

    let quick = QuickAnalysis {
        headline: summary_parts.join(" "),
        risk_level: risk.into(),
        containers_running: 0,
        process_sample_size: runtime.processes.as_ref().map(|p| p.len()).unwrap_or(0),
        code_projects_found: projects.len(),
        mode: experience.mode.clone(),
        ai_role: experience.ai_role.clone(),
    };

    let health = DashboardHealth {
        status: Some((if degraded { "degraded" } else { "ok" }).into()),
        active_incidents: Some(incidents_n),
        queue_depth: None,
        collector_errors: None,
        degraded: Some(degraded),
        degraded_reasons: Some(if degraded_reasons.is_empty() && !storage_ok {
            vec!["storage path missing".into()]
        } else {
            degraded_reasons
        }),
        storage_writes_ok: Some(storage_ok),
        data_dir_bytes_free: free_bytes,
        ai_enabled: Some(ai_enabled),
        ai_available: None,
        ai_reason: None,
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

pub fn build_workspace_map(config: &TomlValue, paths: &Paths) -> Result<WorkspaceMapResponse> {
    let enabled = workspace_enabled(config);
    let scan_root = paths.config_path.parent().unwrap_or(Path::new("."));
    let projects = if enabled {
        discover_projects(config, scan_root)
    } else {
        Vec::new()
    };
    let service_stats = if let Some(db) = EventsStore::open(&paths.events_db)? {
        db.service_aggregates(200)?
    } else {
        vec![]
    };
    let service_ids: Vec<String> = service_stats.iter().map(|s| s.service_id.clone()).collect();
    let config_mappings = config_workspace_mappings(config);
    let mapped_services: std::collections::HashSet<String> = config_mappings
        .iter()
        .map(|m| m.service_id.clone())
        .collect();
    let unmapped_services = service_ids
        .into_iter()
        .filter(|s| enabled && !mapped_services.contains(s))
        .collect();
    Ok(WorkspaceMapResponse {
        enabled,
        projects,
        service_mappings: config_mappings.clone(),
        unmapped_services,
        config_mappings,
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
    let inserted = events.get_events(event_ids)?;
    if inserted.is_empty() {
        return Ok(vec![]);
    }
    let active = incidents.active_incidents(500)?;
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
            stale_timeout_seconds.max(merge_window_seconds * 2),
            60,
        )?;
        if recent.is_empty() {
            continue;
        }
        let max_severity = recent
            .iter()
            .filter_map(event_severity)
            .max()
            .unwrap_or(SEVERITY_INFO);
        let warn_or_higher = recent
            .iter()
            .filter(|event| event_severity(event).unwrap_or(SEVERITY_INFO) >= SEVERITY_WARN)
            .count();
        let affected_services = services_in_events(&recent);
        let source_types = distinct_source_types(&recent);
        let should_open = max_severity >= SEVERITY_ERROR
            || warn_or_higher >= cluster_min_events
            || (warn_or_higher >= 1 && affected_services.len() > 1)
            || (warn_or_higher >= 1
                && source_types.len() > 1
                && recent.len() >= cluster_min_events);
        let existing = active
            .iter()
            .find(|incident| incident_matches_domain(incident, &domain_services));
        if should_open {
            let primary_service = dominant_service(&service_id, &recent);
            let incident_id = existing
                .map(|incident| incident.incident_id.clone())
                .unwrap_or_else(|| format!("inc-{}-{}", slug(&primary_service), unix_seconds()));
            let updated_at = recent
                .iter()
                .filter_map(|event| event.timestamp.clone())
                .max()
                .unwrap_or_else(now_iso);
            let created_at = existing
                .and_then(|incident| incident.created_at.clone())
                .unwrap_or_else(|| {
                    recent
                        .iter()
                        .filter_map(|event| event.timestamp.clone())
                        .min()
                        .unwrap_or_else(now_iso)
                });
            let first_seen = recent
                .iter()
                .filter_map(|event| event.timestamp.clone())
                .min()
                .unwrap_or_else(|| created_at.clone());
            let incident_event_ids = recent
                .iter()
                .filter_map(|event| event.event_id.clone())
                .collect::<Vec<_>>();
            let cluster_payloads = build_clusters(
                config,
                &primary_service,
                &recent,
            );
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
                    "top_messages": top_messages(&recent),
                })),
                resolution_info: None,
            };
            incidents.upsert_incident(&incident, &incident_event_ids)?;
            if existing.is_none() {
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
            incidents.replace_hypotheses(
                &incident_id,
                &build_hypotheses(config, &incident_id, &recent, &updated_at),
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
            touched.push(existing.incident_id.clone());
        }
    }
    Ok(touched)
}

fn build_hypotheses(
    config: &TomlValue,
    incident_id: &str,
    events: &[EventRow],
    updated_at: &str,
) -> Vec<StoredHypothesis> {
    #[derive(Clone)]
    struct Candidate {
        cause_type: String,
        description: String,
        score: f64,
        suggested_checks: Vec<String>,
        supporting_events: Vec<String>,
        affected_services: Vec<String>,
        contradicting_events: Vec<String>,
    }

    let all_services = services_in_events(events);
    let source_types = distinct_source_types(events);
    let max_severity_value = max_severity(events);
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
    if !dependency_support.is_empty() {
        let score = candidate_score(
            max_severity_value,
            dependency_support.len(),
            all_services.len(),
            source_types.len(),
            0.56,
        );
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
            score,
            suggested_checks: vec![
                "Check dependency connectivity and credentials".into(),
                "Inspect upstream health and latency around the incident window".into(),
            ],
            supporting_events: dependency_support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
        });
    }
    if !resource_support.is_empty() {
        let score = candidate_score(
            max_severity_value,
            resource_support.len(),
            all_services.len(),
            source_types.len(),
            0.52,
        );
        candidates.push(Candidate {
            cause_type: "resource_pressure".into(),
            description: "Host or process telemetry indicates resource pressure contributing to the incident.".into(),
            score,
            suggested_checks: vec![
                "Inspect CPU, memory, and disk saturation near the first warning".into(),
                "Review process-level spikes or OOM conditions".into(),
            ],
            supporting_events: resource_support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
        });
    }
    if !instability_support.is_empty() {
        let score = candidate_score(
            max_severity_value,
            instability_support.len(),
            all_services.len(),
            source_types.len(),
            0.49,
        );
        candidates.push(Candidate {
            cause_type: "service_instability".into(),
            description: "Lifecycle signals show the service or a nearby runtime repeatedly failing, restarting, or stopping.".into(),
            score,
            suggested_checks: vec![
                "Inspect restart loops, crash output, and supervisor state".into(),
                "Correlate deploy or restart timing with dependency and resource signals".into(),
            ],
            supporting_events: instability_support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
        });
    }
    if !orchestration_support.is_empty() {
        let score = candidate_score(
            max_severity_value,
            orchestration_support.len(),
            all_services.len(),
            source_types.len(),
            0.46,
        );
        candidates.push(Candidate {
            cause_type: "orchestration_change".into(),
            description: "Container or orchestration activity changed during the incident window and may have triggered downstream impact.".into(),
            score,
            suggested_checks: vec![
                "Review recent pod/container scheduling, restart, and rollout events".into(),
                "Compare runtime changes with the first user-visible failure".into(),
            ],
            supporting_events: orchestration_support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
        });
    }
    if all_services.len() > 1 && source_types.len() > 1 {
        let support = events
            .iter()
            .filter_map(|event| event.event_id.clone())
            .collect::<Vec<_>>();
        let score = candidate_score(
            max_severity_value,
            support.len(),
            all_services.len(),
            source_types.len(),
            0.44,
        );
        candidates.push(Candidate {
            cause_type: "shared_fate".into(),
            description: "Multiple related services degraded together, which suggests a shared dependency or infrastructure fault domain.".into(),
            score,
            suggested_checks: vec![
                "Trace common dependencies across the affected services".into(),
                "Review topology edges and shared infrastructure during the incident window".into(),
            ],
            supporting_events: support,
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
        });
    }
    if candidates.is_empty() {
        candidates.push(Candidate {
            cause_type: "unknown".into(),
            description: "Elevated signal was detected but did not match a stronger native hypothesis template.".into(),
            score: 0.48,
            suggested_checks: vec![
                "Inspect the grouped event timeline".into(),
                "Add topology and service mappings for stronger correlation".into(),
            ],
            supporting_events: events.iter().filter_map(|event| event.event_id.clone()).collect(),
            affected_services: all_services.clone(),
            contradicting_events: Vec::new(),
        });
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let max_hypotheses = config
        .get("hypothesis_engine")
        .and_then(|value| value.get("max_hypotheses_per_incident"))
        .and_then(TomlValue::as_integer)
        .unwrap_or(4)
        .max(1) as usize;
    candidates
        .into_iter()
        .take(max_hypotheses)
        .enumerate()
        .map(|(index, candidate)| StoredHypothesis {
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
                "heuristic_score": candidate.score,
            }),
            supporting_events: candidate.supporting_events,
            contradicting_events: candidate.contradicting_events,
            affected_services: candidate.affected_services,
            suggested_checks: candidate.suggested_checks,
            confidence_label: Some(confidence_label(candidate.score).into()),
            is_valid: true,
            invalidation_reasons: Vec::new(),
            created_at: updated_at.to_string(),
            updated_at: updated_at.to_string(),
        })
        .collect()
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
    severity: i64,
    supporting_events: usize,
    related_services: usize,
    source_types: usize,
    base: f64,
) -> f64 {
    let mut score = base;
    score += (supporting_events.min(4) as f64) * 0.06;
    score += ((related_services.saturating_sub(1)).min(2) as f64) * 0.05;
    score += ((source_types.saturating_sub(1)).min(2) as f64) * 0.03;
    if severity >= SEVERITY_ERROR {
        score += 0.06;
    } else if severity >= SEVERITY_WARN {
        score += 0.03;
    }
    score.clamp(0.0, 0.95)
}

fn confidence_label(score: f64) -> &'static str {
    if score >= 0.8 {
        "high"
    } else if score >= 0.65 {
        "medium"
    } else {
        "low"
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
    event.severity.as_ref().and_then(|value| {
        match value {
            SeverityValue::Level(level) => Some(*level),
            SeverityValue::Label(label) => severity_label_value(label),
        }
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

fn enrich_service_rows(stats: &[ServiceStats], incidents: &[IncidentRow]) -> Vec<ServiceRow> {
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
        .map(|s| {
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
            ServiceRow {
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
            }
        })
        .collect()
}

fn discover_projects(config: &TomlValue, root: &Path) -> Vec<WorkspaceProject> {
    let markers: &[(&str, &str)] = &[
        ("package.json", "node"),
        ("Cargo.toml", "rust"),
        ("go.mod", "go"),
        ("pyproject.toml", "python"),
        (".git", "git"),
    ];
    let mut out = Vec::new();
    let max_depth = workspace_max_depth(config);
    let max_results = workspace_max_results(config);

    for scan_root in workspace_roots(config, root) {
        for entry in WalkDir::new(&scan_root)
            .max_depth(max_depth)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if out.len() >= max_results {
                break;
            }
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            for (name, kind) in markers {
                if path.join(name).exists() {
                    let rel = path.to_string_lossy().to_string();
                    if out.iter().any(|p: &WorkspaceProject| p.path == rel) {
                        break;
                    }
                    out.push(WorkspaceProject {
                        path: rel,
                        kind: (*kind).into(),
                        marker: (*name).into(),
                    });
                    break;
                }
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
    roots.push(base_root.clone());
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
            if !roots.iter().any(|existing| existing == &resolved) {
                roots.push(resolved);
            }
        }
    }
    roots
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
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("inferra-core-{name}-{unique}"))
    }

    #[test]
    fn ai_status_from_config_surfaces_model_overrides() {
        let config: TomlValue = r#"
[ai]
enabled = true
provider = "ollama"
model = "gemma"
model_status = "gemma-status"
model_investigate = "gemma-investigate"
"#
        .parse()
        .expect("parse config");

        let status = ai_status_from_config(&config);
        assert_eq!(status.status_model.as_deref(), Some("gemma-status"));
        assert_eq!(
            status.investigate_model.as_deref(),
            Some("gemma-investigate")
        );
    }

    #[test]
    fn discover_projects_honors_workspace_roots_depth_and_limits() {
        let root = temp_dir("workspace-scan");
        let extra_root = root.join("extra-root");
        fs::create_dir_all(root.join("service-a")).expect("create service-a");
        fs::create_dir_all(root.join("service-b")).expect("create service-b");
        fs::create_dir_all(root.join("deep/one/two/three")).expect("create deep service");
        fs::create_dir_all(extra_root.join("service-c")).expect("create extra service");
        fs::write(root.join("service-a/Cargo.toml"), "[package]\nname='a'\n")
            .expect("write cargo marker");
        fs::write(root.join("service-b/package.json"), "{}").expect("write package marker");
        fs::write(root.join("deep/one/two/three/pyproject.toml"), "[project]\nname='deep'\n")
            .expect("write deep marker");
        fs::write(extra_root.join("service-c/go.mod"), "module example.com/servicec\n")
            .expect("write go marker");

        let config: TomlValue = r#"
[workspace]
max_depth = 2
max_results = 10
roots = ["extra-root"]
"#
        .parse()
        .expect("parse config");

        let projects = discover_projects(&config, &root);
        assert!(projects.iter().any(|project| project.path.contains("service-a")));
        assert!(projects.iter().any(|project| project.path.contains("service-b")));
        assert!(projects.iter().any(|project| project.path.contains("service-c")));
        assert!(!projects.iter().any(|project| project.path.contains("three")));

        let _ = fs::remove_dir_all(&root);
    }
}
