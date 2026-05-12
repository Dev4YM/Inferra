use anyhow::{Context, Result};
use inferra_config::load_merged_config;
use inferra_core::{build_overview, build_workspace_map};
use inferra_storage::{EventsStore, IncidentsStore};
use serde_json::{json, Value as JsonValue};

use crate::cli::{EventAction, IncidentAction, ServiceDataAction, WorkspaceAction};
use crate::commands::{emit_command_result, emit_empty_list, url_encode};
use crate::context::AppContext;

pub fn run_incident_command(ctx: &AppContext, action: IncidentAction) -> Result<()> {
    let paths = ctx.paths()?;
    let Some(store) = IncidentsStore::open(&paths.incidents_db)? else {
        emit_empty_list(ctx, "incidents", "No incidents database found.");
        return Ok(());
    };
    match action {
        IncidentAction::List(args) => {
            let incidents = store.active_incidents(args.limit)?;
            let payload = json!({ "incidents": incidents });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui.banner(
                "Incidents",
                "Active incidents ordered by severity and recency",
            );
            if let Some(items) = payload["incidents"].as_array() {
                if items.is_empty() {
                    ctx.ui.info("No active incidents.");
                } else {
                    let rows = items.iter().map(|incident| {
                        vec![
                            incident["incident_id"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string(),
                            incident["state"].as_str().unwrap_or("unknown").to_string(),
                            incident["severity"]
                                .as_i64()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "-".to_string()),
                            incident["primary_service"]
                                .as_str()
                                .unwrap_or_default()
                                .to_string(),
                            incident["event_count"]
                                .as_i64()
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "-".to_string()),
                        ]
                    });
                    ctx.ui.table(
                        &["Incident", "State", "Severity", "Service", "Events"],
                        rows,
                    );
                }
            }
        }
        IncidentAction::Show { incident_id } => {
            let incident = store
                .get_incident(&incident_id)?
                .with_context(|| format!("incident not found: {incident_id}"))?;
            let payload = json!({
                "incident": incident,
                "event_ids": store.incident_event_ids(&incident_id)?,
                "hypotheses": store.hypotheses(&incident_id)?,
                "clusters": store.clusters(&incident_id)?,
                "state_log": store.list_state_log(&incident_id)?,
            });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui.banner(
                &format!("Incident {incident_id}"),
                "Incident detail, state, and top hypotheses",
            );
            ctx.ui.kv_table([
                (
                    "State",
                    payload["incident"]["state"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string(),
                ),
                (
                    "Severity",
                    payload["incident"]["severity"]
                        .as_i64()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ),
                (
                    "Primary service",
                    payload["incident"]["primary_service"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                ),
                (
                    "Event ids",
                    payload["event_ids"]
                        .as_array()
                        .map(|items| items.len().to_string())
                        .unwrap_or_else(|| "0".to_string()),
                ),
            ]);
            let hypotheses = payload["hypotheses"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !hypotheses.is_empty() {
                ctx.ui.paragraph("");
                ctx.ui.section("Hypotheses");
                let rows = hypotheses.iter().take(8).map(|item| {
                    vec![
                        item["hypothesis_id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        item["cause_type"].as_str().unwrap_or("-").to_string(),
                        item["rank"]
                            .as_i64()
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        item["confidence_label"].as_str().unwrap_or("-").to_string(),
                    ]
                });
                ctx.ui
                    .table(&["Hypothesis", "Type", "Rank", "Confidence"], rows);
            }
        }
    }
    Ok(())
}

pub fn run_event_command(ctx: &AppContext, action: EventAction) -> Result<()> {
    let paths = ctx.paths()?;
    let Some(store) = EventsStore::open(&paths.events_db)? else {
        emit_empty_list(ctx, "events", "No events database found.");
        return Ok(());
    };
    match action {
        EventAction::List(args) => {
            let events = store.query_logs(
                args.limit,
                args.service.as_deref(),
                args.severity,
                args.search.as_deref(),
                args.source_type.as_deref(),
            )?;
            let payload = json!({ "events": events });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui
                .banner("Events", "Raw event rows from the local store");
            let rows = payload["events"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|event| {
                    vec![
                        event["event_id"].as_str().unwrap_or("-").to_string(),
                        event["severity"]
                            .as_i64()
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        event["service_id"].as_str().unwrap_or("-").to_string(),
                        event["message"].as_str().unwrap_or_default().to_string(),
                    ]
                })
                .collect::<Vec<_>>();
            if rows.is_empty() {
                ctx.ui.info("No matching events.");
            } else {
                ctx.ui
                    .table(&["Event", "Severity", "Service", "Message"], rows);
            }
        }
        EventAction::Show { event_id } => {
            let event = store
                .get_event(&event_id)?
                .with_context(|| format!("event not found: {event_id}"))?;
            let payload = json!({ "event": event });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui
                .banner(&format!("Event {event_id}"), "Single event detail");
            ctx.ui.kv_table([
                (
                    "Service",
                    payload["event"]["service_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                ),
                (
                    "Severity",
                    payload["event"]["severity"]
                        .as_i64()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ),
                (
                    "Timestamp",
                    payload["event"]["timestamp"]
                        .as_str()
                        .unwrap_or("-")
                        .to_string(),
                ),
            ]);
            if let Some(message) = payload["event"]["message"].as_str() {
                ctx.ui.paragraph("");
                ctx.ui.section("Message");
                ctx.ui.paragraph(message);
            }
        }
    }
    Ok(())
}

pub fn run_service_data_command(ctx: &AppContext, action: ServiceDataAction) -> Result<()> {
    let paths = ctx.paths()?;
    let config = load_merged_config(&paths.config_path)?;
    let overview = build_overview(&config, &paths)?;
    let services = overview.dashboard.services.unwrap_or_default();
    match action {
        ServiceDataAction::List(args) => {
            let rows = services.into_iter().take(args.limit).collect::<Vec<_>>();
            let payload = json!({ "services": rows });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui
                .banner("Services", "Service health derived from the local overview");
            let table_rows = payload["services"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|service| {
                    vec![
                        service["service_id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        service["status"].as_str().unwrap_or("unknown").to_string(),
                        service["event_count"]
                            .as_i64()
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        service["error_count"]
                            .as_i64()
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                    ]
                })
                .collect::<Vec<_>>();
            ctx.ui
                .table(&["Service", "Status", "Events", "Errors"], table_rows);
        }
        ServiceDataAction::Show { service_id } => {
            let service = services
                .iter()
                .find(|row| row.service_id == service_id)
                .cloned()
                .with_context(|| format!("service not found: {service_id}"))?;
            let recent_events = EventsStore::open(&paths.events_db)?
                .map(|store| store.events_for_service(&service_id, 25))
                .transpose()?
                .unwrap_or_default();
            let payload = json!({ "service": service, "recent_events": recent_events });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui.banner(
                &format!("Service {service_id}"),
                "Service health and recent events",
            );
            ctx.ui.kv_table([
                (
                    "Status",
                    payload["service"]["status"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string(),
                ),
                (
                    "Events",
                    payload["service"]["event_count"]
                        .as_i64()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ),
                (
                    "Errors",
                    payload["service"]["error_count"]
                        .as_i64()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ),
            ]);
            let rows = payload["recent_events"]
                .as_array()
                .into_iter()
                .flatten()
                .take(10)
                .map(|event| {
                    vec![
                        event["event_id"].as_str().unwrap_or("-").to_string(),
                        event["severity"]
                            .as_i64()
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        event["message"].as_str().unwrap_or_default().to_string(),
                    ]
                })
                .collect::<Vec<_>>();
            if !rows.is_empty() {
                ctx.ui.paragraph("");
                ctx.ui.section("Recent events");
                ctx.ui.table(&["Event", "Severity", "Message"], rows);
            }
        }
        ServiceDataAction::Events { service_id, args } => {
            let events = EventsStore::open(&paths.events_db)?
                .map(|store| store.events_for_service(&service_id, args.limit))
                .transpose()?
                .unwrap_or_default();
            let payload = json!({ "service_id": service_id, "events": events });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui.banner(
                &format!(
                    "Service events {}",
                    payload["service_id"].as_str().unwrap_or_default()
                ),
                "Recent events for one service",
            );
            let rows = payload["events"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|event| {
                    vec![
                        event["event_id"].as_str().unwrap_or("-").to_string(),
                        event["severity"]
                            .as_i64()
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "-".to_string()),
                        event["message"].as_str().unwrap_or_default().to_string(),
                    ]
                })
                .collect::<Vec<_>>();
            ctx.ui.table(&["Event", "Severity", "Message"], rows);
        }
    }
    Ok(())
}

pub async fn run_workspace_command(ctx: &AppContext, action: WorkspaceAction) -> Result<()> {
    let paths = ctx.paths()?;
    let config = load_merged_config(&paths.config_path)?;
    match action {
        WorkspaceAction::Map => {
            let payload = serde_json::to_value(build_workspace_map(&config, &paths)?)?;
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui
                .banner("Workspace map", "Detected projects and service mappings");
            ctx.ui.kv_table([
                (
                    "Projects",
                    payload["projects"]
                        .as_array()
                        .map(|items| items.len().to_string())
                        .unwrap_or_else(|| "0".to_string()),
                ),
                (
                    "Service mappings",
                    payload["service_mappings"]
                        .as_array()
                        .map(|items| items.len().to_string())
                        .unwrap_or_else(|| "0".to_string()),
                ),
                (
                    "Unmapped services",
                    payload["unmapped_services"]
                        .as_array()
                        .map(|items| items.len().to_string())
                        .unwrap_or_else(|| "0".to_string()),
                ),
            ]);
            Ok(())
        }
        WorkspaceAction::Services => {
            let payload = serde_json::to_value(build_workspace_map(&config, &paths)?)?;
            let subset = json!({
                "service_mappings": payload["service_mappings"].clone(),
                "unmapped_services": payload["unmapped_services"].clone(),
            });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&subset);
                return Ok(());
            }
            ctx.ui
                .banner("Workspace services", "Mapped and unmapped services");
            let rows = subset["service_mappings"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|mapping| {
                    vec![
                        mapping["service_id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        mapping["project_path"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        mapping["confidence"]
                            .as_f64()
                            .map(|value| format!("{value:.2}"))
                            .unwrap_or_else(|| "-".to_string()),
                    ]
                })
                .collect::<Vec<_>>();
            if !rows.is_empty() {
                ctx.ui.table(&["Service", "Project", "Confidence"], rows);
            }
            let unmapped = subset["unmapped_services"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            if !unmapped.is_empty() {
                ctx.ui.paragraph("");
                ctx.ui.section("Unmapped");
                ctx.ui.bullets(unmapped);
            }
            Ok(())
        }
        WorkspaceAction::Inspect { path } => {
            let payload = ctx
                .api_request(
                    reqwest::Method::GET,
                    &format!("/api/workspace/inspect?path={}", url_encode(&path)),
                    None,
                )
                .await?;
            emit_command_result(ctx, &payload, &[format!("path={path}")]);
            Ok(())
        }
        WorkspaceAction::Projects {
            max_depth,
            max_results,
        } => {
            let payload = ctx
                .api_request(
                    reqwest::Method::GET,
                    &format!(
                        "/api/workspace/projects?max_depth={max_depth}&max_results={max_results}"
                    ),
                    None,
                )
                .await?;
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui
                .banner("Projects", "Projects discovered by the workspace API");
            let rows = payload["projects"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|project| {
                    vec![
                        project["path"].as_str().unwrap_or_default().to_string(),
                        project["kind"].as_str().unwrap_or_default().to_string(),
                        project["marker"].as_str().unwrap_or_default().to_string(),
                    ]
                })
                .collect::<Vec<_>>();
            ctx.ui.table(&["Path", "Kind", "Marker"], rows);
            Ok(())
        }
    }
}
