use std::net::TcpListener;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use dialoguer::{Confirm, Input};
use inferra_collectors::configured_collectors;
use inferra_config::{apply_config_put, load_merged_config, server_listen, write_config};
use inferra_core::build_overview;
use serde_json::{json, Value as JsonValue};

use crate::cli::{CollectorAction, ConfigAction, ServiceAction};
use crate::commands::{
    emit_command_result, parse_cli_value, patch_from_path, preset_patch, toml_to_display,
    toml_to_json, toml_value_at,
};
use crate::context::{app_version, local_base_url, AppContext};

pub async fn show_landing(ctx: &AppContext) -> Result<()> {
    let spinner = ctx.ui.spinner("Inspecting local Inferra runtime");
    let payload = collect_status_payload(ctx).await?;
    spinner.finish("Status snapshot ready");
    if ctx.ui.is_json() {
        ctx.ui.print_json(&payload);
        return Ok(());
    }
    ctx.ui.banner(
        &format!("Inferra {}", app_version()),
        "Local runtime, service control, and incident investigation",
    );
    render_status_snapshot(ctx, &payload, true);
    Ok(())
}

pub async fn show_status(ctx: &AppContext) -> Result<()> {
    let spinner = ctx.ui.spinner("Gathering runtime, service, and dashboard status");
    let payload = collect_status_payload(ctx).await?;
    spinner.finish("Detailed status ready");
    if ctx.ui.is_json() {
        ctx.ui.print_json(&payload);
        return Ok(());
    }
    ctx.ui.banner(
        &format!("Inferra {}", app_version()),
        "Detailed local runtime status",
    );
    render_status_snapshot(ctx, &payload, false);
    Ok(())
}

pub async fn run_serve_command(ctx: &AppContext) -> Result<()> {
    let paths = ctx.paths()?;
    let config = load_merged_config(&paths.config_path)?;
    let (host, port) = server_listen(&config)?;
    let ui_dist = ctx.resolve_ui_dist()?;
    match inferra_api::serve(paths, ui_dist).await {
        Ok(()) => Ok(()),
        Err(error) if is_addr_in_use_error(&error) => handle_existing_runtime(host, port, error).await,
        Err(error) => Err(error),
    }
}

pub async fn run_service_command(
    ctx: &AppContext,
    action: Option<ServiceAction>,
    service_run: bool,
) -> Result<()> {
    match action {
        Some(ServiceAction::Install { startup }) => install_windows_service(ctx, &startup),
        Some(ServiceAction::Remove) => {
            inferra_windows_service::remove_service()?;
            let payload = json!({
                "service": inferra_windows_service::SERVICE_NAME,
                "action": "remove",
                "ok": true,
            });
            emit_command_result(
                ctx,
                &payload,
                &[format!(
                    "Removed Windows service {}",
                    inferra_windows_service::SERVICE_NAME
                )],
            );
            Ok(())
        }
        Some(ServiceAction::Start) => {
            inferra_windows_service::start_service()?;
            let payload = json!({
                "service": inferra_windows_service::SERVICE_NAME,
                "action": "start",
                "ok": true,
            });
            emit_command_result(
                ctx,
                &payload,
                &[format!(
                    "Started Windows service {}",
                    inferra_windows_service::SERVICE_NAME
                )],
            );
            Ok(())
        }
        Some(ServiceAction::Stop) => {
            inferra_windows_service::stop_service()?;
            let payload = json!({
                "service": inferra_windows_service::SERVICE_NAME,
                "action": "stop",
                "ok": true,
            });
            emit_command_result(
                ctx,
                &payload,
                &[format!(
                    "Stopped Windows service {}",
                    inferra_windows_service::SERVICE_NAME
                )],
            );
            Ok(())
        }
        Some(ServiceAction::Restart) => {
            inferra_windows_service::restart_service()?;
            let payload = json!({
                "service": inferra_windows_service::SERVICE_NAME,
                "action": "restart",
                "ok": true,
            });
            emit_command_result(
                ctx,
                &payload,
                &[format!(
                    "Restarted Windows service {}",
                    inferra_windows_service::SERVICE_NAME
                )],
            );
            Ok(())
        }
        Some(ServiceAction::Status) => emit_service_status(ctx),
        Some(ServiceAction::Repair) => run_service_repair(ctx),
        None => {
            let paths = ctx.paths()?;
            let ui_dist = ctx.resolve_ui_dist()?;
            if service_run {
                inferra_windows_service::dispatch_service(paths, ui_dist)?;
            } else {
                inferra_windows_service::run_service_or_foreground(paths, ui_dist).await?;
            }
            Ok(())
        }
    }
}

pub fn run_setup(
    ctx: &AppContext,
    yes: bool,
    skip_connection_test: bool,
    data_dir: Option<PathBuf>,
) -> Result<()> {
    let config_path = ctx.paths()?.config_path;
    let base = load_merged_config(&config_path)?;
    let default_data_dir = toml_value_at(&base, "storage.data_dir")
        .and_then(|value| value.as_str())
        .unwrap_or("./data")
        .to_string();
    let final_data_dir = match data_dir {
        Some(path) => Some(path),
        None if ctx.ui.is_interactive() && !yes => {
            let entered: String = Input::new()
                .with_prompt("Data directory")
                .default(default_data_dir)
                .interact_text()?;
            Some(PathBuf::from(entered))
        }
        None => None,
    };
    if ctx.ui.is_interactive() && !yes {
        let confirmed = Confirm::new()
            .with_prompt(format!("Write config to {}?", config_path.display()))
            .default(true)
            .interact()?;
        if !confirmed {
            bail!("setup cancelled by user");
        }
    }
    let final_cfg = if let Some(data_dir) = final_data_dir.clone() {
        apply_config_put(
            base,
            &json!({
                "storage": {
                    "data_dir": data_dir
                }
            }),
        )?
    } else {
        base
    };
    write_config(&config_path, &final_cfg)?;
    let payload = json!({
        "command": "setup",
        "config_path": config_path.display().to_string(),
        "data_dir": final_data_dir.as_ref().map(|path| path.display().to_string()),
        "skip_connection_test": skip_connection_test,
        "ok": true,
    });
    emit_command_result(
        ctx,
        &payload,
        &[format!("Wrote config {}", config_path.display())],
    );
    Ok(())
}

pub fn run_init_db(ctx: &AppContext) -> Result<()> {
    let paths = ctx.paths()?;
    inferra_storage::initialize_databases(&paths.events_db, &paths.incidents_db)?;
    let payload = json!({
        "command": "init-db",
        "ok": true,
        "data_dir": paths.data_dir.display().to_string(),
        "events_db": paths.events_db.display().to_string(),
        "incidents_db": paths.incidents_db.display().to_string(),
    });
    emit_command_result(
        ctx,
        &payload,
        &[format!(
            "Initialized Rust SQLite databases under {}",
            paths.data_dir.display()
        )],
    );
    Ok(())
}

pub async fn run_collector_command(ctx: &AppContext, action: CollectorAction) -> Result<()> {
    match action {
        CollectorAction::Status => {
            let spinner = ctx.ui.spinner("Checking collector status");
            let paths = ctx.paths()?;
            let config = load_merged_config(&paths.config_path)?;
            let payload = match ctx
                .api_request(reqwest::Method::GET, "/api/collectors", None)
                .await
            {
                Ok(payload) => payload,
                Err(_) => json!({
                    "collectors": configured_collectors(&config).into_iter().map(|row| json!({
                        "collector_id": row.collector_id,
                        "status": row.status,
                        "source_type": row.source_type,
                        "is_running": false,
                    })).collect::<Vec<_>>(),
                    "queue_depth": 0,
                    "fallback": true,
                }),
            };
            spinner.finish("Collector snapshot ready");
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
                return Ok(());
            }
            ctx.ui.banner("Collectors", "Configured collectors and local API runtime state");
            let rows = payload["collectors"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|collector| {
                    vec![
                        collector
                            .get("collector_id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        collector
                            .get("status")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("unknown")
                            .to_string(),
                        collector
                            .get("is_running")
                            .and_then(JsonValue::as_bool)
                            .unwrap_or(false)
                            .to_string(),
                        collector
                            .get("source_type")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("-")
                            .to_string(),
                    ]
                })
                .collect::<Vec<_>>();
            ctx.ui.table(&["Collector", "Status", "Running", "Source"], rows);
            if payload.get("fallback") == Some(&JsonValue::Bool(true)) {
                ctx.ui.warning("The runtime API was not reachable, so this view used static config.");
            }
            Ok(())
        }
        CollectorAction::Start => {
            let spinner = ctx.ui.spinner("Requesting collector start");
            let payload = ctx
                .api_request(reqwest::Method::POST, "/api/collectors/start", None)
                .await?;
            spinner.finish("Collector start requested");
            emit_command_result(
                ctx,
                &payload,
                &["Requested collector start via local API.".to_string()],
            );
            Ok(())
        }
        CollectorAction::Stop => {
            let spinner = ctx.ui.spinner("Requesting collector stop");
            let payload = ctx
                .api_request(reqwest::Method::POST, "/api/collectors/stop", None)
                .await?;
            spinner.finish("Collector stop requested");
            emit_command_result(
                ctx,
                &payload,
                &["Requested collector stop via local API.".to_string()],
            );
            Ok(())
        }
    }
}

pub fn run_config_command(ctx: &AppContext, action: ConfigAction) -> Result<()> {
    let paths = ctx.paths()?;
    let config = load_merged_config(&paths.config_path)?;
    match action {
        ConfigAction::Show => {
            let payload = json!({ "config": config, "config_path": paths.config_path.display().to_string() });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
            } else {
                ctx.ui.banner("Config", "Merged config with defaults applied");
                ctx.ui.kv_table([
                    ("Config path", paths.config_path.display().to_string()),
                    ("Data dir", paths.data_dir.display().to_string()),
                ]);
                ctx.ui.paragraph("");
                ctx.ui.print_json(&payload["config"]);
            }
            Ok(())
        }
        ConfigAction::Get { key } => {
            let value = toml_value_at(&config, &key);
            let payload = json!({
                "key": key,
                "value": value.cloned().map(toml_to_json).unwrap_or(JsonValue::Null),
            });
            emit_command_result(
                ctx,
                &payload,
                &[format!("{key}={}", toml_to_display(value))],
            );
            Ok(())
        }
        ConfigAction::Set { key, value } => {
            let patch = patch_from_path(&key, parse_cli_value(&value));
            let next = inferra_config::apply_config_put(config, &patch)?;
            write_config(&paths.config_path, &next)?;
            let payload = json!({ "updated": true, "key": key, "value": value });
            emit_command_result(
                ctx,
                &payload,
                &[format!("Updated {key} in {}", paths.config_path.display())],
            );
            Ok(())
        }
        ConfigAction::Preset { name } => {
            let next = inferra_config::apply_config_put(config, &preset_patch(&name)?)?;
            write_config(&paths.config_path, &next)?;
            let payload = json!({ "preset": name, "applied": true });
            emit_command_result(
                ctx,
                &payload,
                &[format!("Applied preset {name} to {}", paths.config_path.display())],
            );
            Ok(())
        }
    }
}

fn install_windows_service(ctx: &AppContext, startup: &str) -> Result<()> {
    if !cfg!(windows) {
        bail!("Windows service install is only supported on Windows");
    }
    let paths = ctx.paths()?;
    let ui_dist = ctx.resolve_ui_dist()?;
    let startup_mode = inferra_windows_service::ServiceStartup::from_cli(startup)?;
    let binary = std::env::current_exe().context("resolve current inferra executable")?;
    inferra_windows_service::install_service(
        &binary,
        &inferra_windows_service::ServiceInstallOptions {
            config_path: paths.config_path.clone(),
            ui_dist: ui_dist.clone(),
            startup: startup_mode,
        },
    )?;
    let payload = json!({
        "service": inferra_windows_service::SERVICE_NAME,
        "action": "install",
        "startup": startup,
        "config_path": paths.config_path.display().to_string(),
        "ui_dist": ui_dist.display().to_string(),
        "ok": true,
    });
    emit_command_result(
        ctx,
        &payload,
        &[format!(
            "Installed Windows service {}",
            inferra_windows_service::SERVICE_NAME
        )],
    );
    Ok(())
}

fn emit_service_status(ctx: &AppContext) -> Result<()> {
    let status = inferra_windows_service::query_service_status()?;
    let payload = json!({
        "service": inferra_windows_service::SERVICE_NAME,
        "installed": status.installed,
        "state": status.state,
        "startup": status.startup,
        "log_path": status.log_path.display().to_string(),
        "binary_path": status.binary_path,
    });
    if ctx.ui.is_json() {
        ctx.ui.print_json(&payload);
    } else {
        ctx.ui.banner("Service", "Windows service status");
        ctx.ui.kv_table([
            ("Service", inferra_windows_service::SERVICE_NAME.to_string()),
            (
                "Installed",
                payload["installed"].as_bool().unwrap_or(false).to_string(),
            ),
            (
                "State",
                payload["state"].as_str().unwrap_or("not_installed").to_string(),
            ),
            (
                "Startup",
                payload["startup"].as_str().unwrap_or("-").to_string(),
            ),
            (
                "Log path",
                payload["log_path"].as_str().unwrap_or_default().to_string(),
            ),
        ]);
        if let Some(binary) = payload.get("binary_path").and_then(JsonValue::as_str) {
            ctx.ui.paragraph(&format!("Binary: {binary}"));
        }
    }
    Ok(())
}

fn run_service_repair(ctx: &AppContext) -> Result<()> {
    let spinner = ctx.ui.spinner("Validating service prerequisites");
    let paths = ctx.paths()?;
    let config = load_merged_config(&paths.config_path)?;
    let (host, port) = server_listen(&config)?;
    let log_path = inferra_windows_service::service_log_path();

    let mut findings = Vec::<JsonValue>::new();
    findings.push(json!({
        "name": "config_path",
        "ok": true,
        "message": format!("Config path: {}", paths.config_path.display()),
    }));

    let data_dir_ok = paths.data_dir.exists();
    findings.push(json!({
        "name": "data_dir",
        "ok": data_dir_ok,
        "message": if data_dir_ok {
            format!("Data dir exists: {}", paths.data_dir.display())
        } else {
            format!("Data dir missing: {}", paths.data_dir.display())
        },
    }));

    let ui_dist = ctx.resolve_ui_dist();
    findings.push(json!({
        "name": "ui_dist",
        "ok": ui_dist.as_ref().map(|path| path.exists()).unwrap_or(false),
        "message": match ui_dist {
            Ok(path) if path.exists() => format!("UI bundle: {}", path.display()),
            Ok(path) => format!("UI bundle missing: {}", path.display()),
            Err(error) => format!("UI bundle unavailable: {error}"),
        },
    }));

    let bind_ok = TcpListener::bind((host.as_str(), port)).is_ok();
    findings.push(json!({
        "name": "bind",
        "ok": bind_ok,
        "message": if bind_ok {
            format!("Can bind {host}:{port}")
        } else {
            format!("Cannot bind {host}:{port}")
        },
    }));

    findings.push(json!({
        "name": "log_path",
        "ok": true,
        "message": format!("Service log path: {}", log_path.display()),
    }));

    if cfg!(windows) {
        let status = inferra_windows_service::query_service_status()?;
        findings.push(json!({
            "name": "service",
            "ok": true,
            "message": if status.installed {
                format!(
                    "Service installed state={} startup={}",
                    status.state.as_deref().unwrap_or("unknown"),
                    status.startup.as_deref().unwrap_or("unknown")
                )
            } else {
                "Service not installed".to_string()
            },
        }));
    }

    let ok = findings
        .iter()
        .all(|item| item.get("ok").and_then(JsonValue::as_bool).unwrap_or(false));
    let next_steps = if ok {
        vec![
            "inferra service status".to_string(),
            "inferra service restart".to_string(),
        ]
    } else {
        vec![
            format!(
                "inferra --config \"{}\" init-db",
                paths.config_path.display()
            ),
            format!(
                "inferra --config \"{}\" service install --startup auto",
                paths.config_path.display()
            ),
        ]
    };
    let payload = json!({
        "command": "service repair",
        "config_path": paths.config_path.display().to_string(),
        "ok": ok,
        "findings": findings,
        "safe_next_steps": next_steps,
    });
    spinner.finish("Service checks complete");
    if ctx.ui.is_json() {
        ctx.ui.print_json(&payload);
    } else {
        ctx.ui.banner("Service Repair", "Non-destructive checks for local service readiness");
        let rows = payload["findings"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|finding| {
                vec![
                    finding
                        .get("name")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    if finding.get("ok") == Some(&JsonValue::Bool(true)) {
                        "ok".to_string()
                    } else {
                        "check".to_string()
                    },
                    finding
                        .get("message")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ]
            })
            .collect::<Vec<_>>();
        ctx.ui.table(&["Check", "Result", "Detail"], rows);
        ctx.ui.section("Next");
        ctx.ui.bullets(
            payload["safe_next_steps"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
    }
    if ok {
        Ok(())
    } else {
        bail!("service repair found issues")
    }
}

async fn collect_status_payload(ctx: &AppContext) -> Result<JsonValue> {
    let paths = ctx.paths()?;
    let config = load_merged_config(&paths.config_path)?;
    let (bind_host, port) = server_listen(&config)?;
    let dashboard_url = local_base_url(&bind_host, port);
    let health = ctx
        .api_request(reqwest::Method::GET, "/api/health", None)
        .await
        .ok();
    let overview = match ctx
        .api_request(reqwest::Method::GET, "/api/overview", None)
        .await
    {
        Ok(payload) => Some(payload),
        Err(_) => serde_json::to_value(build_overview(&config, &paths)?).ok(),
    };
    let service = match inferra_windows_service::query_service_status() {
        Ok(status) => json!({
            "supported": true,
            "installed": status.installed,
            "state": status.state,
            "startup": status.startup,
            "binary_path": status.binary_path,
            "log_path": status.log_path.display().to_string(),
        }),
        Err(error) => json!({
            "supported": false,
            "installed": false,
            "state": JsonValue::Null,
            "startup": JsonValue::Null,
            "error": error.to_string(),
        }),
    };
    let dashboard = overview
        .as_ref()
        .and_then(|payload| payload.get("dashboard"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    Ok(json!({
        "name": "inferra",
        "version": app_version(),
        "config_path": paths.config_path.display().to_string(),
        "data_dir": paths.data_dir.display().to_string(),
        "runtime": {
            "reachable": health.is_some(),
            "bind_host": bind_host,
            "port": port,
            "dashboard_url": format!("{dashboard_url}/"),
            "health": health,
        },
        "service": service,
        "overview": {
            "active_incidents": dashboard.get("incidents").and_then(JsonValue::as_array).map(|items| items.len()),
            "services": dashboard.get("services").and_then(JsonValue::as_array).map(|items| items.len()),
            "health": dashboard.get("health").cloned(),
        }
    }))
}

fn render_status_snapshot(ctx: &AppContext, payload: &JsonValue, welcome_only: bool) {
    let runtime = &payload["runtime"];
    if runtime.get("reachable") == Some(&JsonValue::Bool(true)) {
        ctx.ui.success(
            &format!(
                "Runtime reachable at {}",
                runtime["dashboard_url"].as_str().unwrap_or_default()
            ),
        );
    } else {
        ctx.ui.warning("Runtime is not responding on the configured local address.");
    }
    ctx.ui.kv_table([
        (
            "Version",
            payload["version"].as_str().unwrap_or_default().to_string(),
        ),
        (
            "Dashboard",
            runtime["dashboard_url"].as_str().unwrap_or_default().to_string(),
        ),
        (
            "Config path",
            payload["config_path"].as_str().unwrap_or_default().to_string(),
        ),
        (
            "Data dir",
            payload["data_dir"].as_str().unwrap_or_default().to_string(),
        ),
    ]);

    ctx.ui.paragraph("");
    ctx.ui.section("Runtime");
    ctx.ui.kv_table([
        (
            "Bind",
            format!(
                "{}:{}",
                runtime["bind_host"].as_str().unwrap_or_default(),
                runtime["port"].as_u64().unwrap_or_default()
            ),
        ),
        (
            "Reachable",
            runtime["reachable"].as_bool().unwrap_or(false).to_string(),
        ),
        (
            "Health status",
            runtime["health"]["status"]
                .as_str()
                .or_else(|| payload["overview"]["health"]["status"].as_str())
                .unwrap_or("unknown")
                .to_string(),
        ),
    ]);

    if !welcome_only {
        ctx.ui.paragraph("");
        ctx.ui.section("Service");
        ctx.ui.kv_table([
            (
                "Supported",
                payload["service"]["supported"]
                    .as_bool()
                    .unwrap_or(false)
                    .to_string(),
            ),
            (
                "Installed",
                payload["service"]["installed"]
                    .as_bool()
                    .unwrap_or(false)
                    .to_string(),
            ),
            (
                "State",
                payload["service"]["state"]
                    .as_str()
                    .unwrap_or("not_installed")
                    .to_string(),
            ),
            (
                "Startup",
                payload["service"]["startup"]
                    .as_str()
                    .unwrap_or("-")
                    .to_string(),
            ),
        ]);
        if let Some(log_path) = payload["service"].get("log_path").and_then(JsonValue::as_str) {
            ctx.ui.paragraph(&format!("Log path: {log_path}"));
        }

        ctx.ui.paragraph("");
        ctx.ui.section("Overview");
        ctx.ui.kv_table([
            (
                "Active incidents",
                payload["overview"]["active_incidents"]
                    .as_u64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
            (
                "Services",
                payload["overview"]["services"]
                    .as_u64()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
            (
                "AI available",
                payload["overview"]["health"]["ai_available"]
                    .as_bool()
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
        ]);
    }

    ctx.ui.paragraph("");
    ctx.ui.section("Next");
    ctx.ui.bullets(vec![
        "inferra status".to_string(),
        "inferra serve".to_string(),
        "inferra collectors status".to_string(),
        "inferra ai status".to_string(),
        "inferra service status".to_string(),
    ]);
}

async fn handle_existing_runtime(host: String, port: u16, error: anyhow::Error) -> Result<()> {
    let base_url = local_base_url(&host, port);
    let health_url = format!("{base_url}/api/health");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .context("build runtime probe client")?;
    match client.get(&health_url).send().await {
        Ok(response) if response.status().is_success() => {
            println!("Inferra is already running at {base_url}/");
            println!("Health endpoint: {health_url}");
            println!(
                "Use `inferra status`, `inferra service status`, or open the dashboard instead of starting a second listener."
            );
            Ok(())
        }
        Ok(response) => Err(error.context(format!(
            "port {port} on {host} is already in use; {health_url} returned {}",
            response.status()
        ))),
        Err(probe_error) => Err(error.context(format!(
            "port {port} on {host} is already in use and no Inferra runtime responded at {health_url}: {probe_error}"
        ))),
    }
}

fn is_addr_in_use_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_error| {
                io_error.kind() == std::io::ErrorKind::AddrInUse
                    || matches!(io_error.raw_os_error(), Some(10048 | 98))
            })
    }) || error
        .to_string()
        .to_ascii_lowercase()
        .contains("only one usage of each socket address")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_in_use_detection_matches_socket_conflicts() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "socket already bound",
        ));
        assert!(is_addr_in_use_error(&error));
    }

    #[test]
    fn addr_in_use_detection_ignores_other_io_errors() {
        let error = anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ));
        assert!(!is_addr_in_use_error(&error));
    }
}
