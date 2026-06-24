use std::net::TcpListener;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use dialoguer::{Confirm, Input};
use inferra_collectors::configured_collectors;
use inferra_config::{apply_config_put, load_merged_config, server_listen, write_config};
use inferra_core::build_overview;
use serde_json::{json, Value as JsonValue};

use crate::cli::{CollectorAction, ConfigAction, RuntimeAction, ServiceAction};
use crate::commands::{
    emit_command_result, parse_cli_value, patch_from_path, preset_patch, toml_to_display,
    toml_to_json, toml_value_at,
};
use crate::context::{app_version, local_base_url, AppContext};
use crate::runtime_supervisor::{
    query_supervisor_status, restart_supervisor, runtime_unreachable_hint, service_install_hint,
    start_supervisor, stop_supervisor, SupervisorStatus,
};

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
    let spinner = ctx
        .ui
        .spinner("Gathering runtime, service, and dashboard status");
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
        Err(error) if is_addr_in_use_error(&error) => {
            handle_existing_runtime(host, port, error).await
        }
        Err(error) => Err(error),
    }
}

pub async fn run_runtime_command(ctx: &AppContext, action: Option<RuntimeAction>) -> Result<()> {
    match action {
        Some(RuntimeAction::Status) => emit_runtime_status(ctx).await,
        Some(RuntimeAction::Start) => runtime_start(ctx).await,
        Some(RuntimeAction::Stop) => runtime_stop(ctx).await,
        Some(RuntimeAction::Restart) => runtime_restart(ctx).await,
        Some(RuntimeAction::Open) => runtime_open(ctx).await,
        None => emit_runtime_status(ctx).await,
    }
}

async fn emit_runtime_status(ctx: &AppContext) -> Result<()> {
    let payload = collect_runtime_payload(ctx).await?;
    if ctx.ui.is_json() {
        ctx.ui.print_json(&payload);
        return Ok(());
    }
    ctx.ui
        .banner("Inferra runtime", "API and web dashboard status");
    let mode = payload["mode"].as_str().unwrap_or("unknown");
    let dashboard = payload["dashboard_url"].as_str().unwrap_or_default();
    if payload["reachable"].as_bool().unwrap_or(false) {
        ctx.ui.success(&format!(
            "API and dashboard are running ({mode}) at {dashboard}"
        ));
    } else {
        ctx.ui
            .warning("API and dashboard are not responding on the configured address.");
    }
    ctx.ui.kv_table([
        ("Mode", mode.to_string()),
        ("Dashboard", dashboard.to_string()),
        (
            "API health",
            payload["health_status"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
        ),
        (
            "Service installed",
            payload["service_installed"]
                .as_bool()
                .unwrap_or(false)
                .to_string(),
        ),
        (
            "Service state",
            payload["service_state"]
                .as_str()
                .unwrap_or("not_installed")
                .to_string(),
        ),
    ]);
    if let Some(hint) = payload["hint"].as_str() {
        ctx.ui.paragraph("");
        ctx.ui.section("Next");
        ctx.ui.bullets(vec![hint.to_string()]);
    }
    Ok(())
}

async fn runtime_start(ctx: &AppContext) -> Result<()> {
    let status = query_supervisor_status()?;
    if !status.supported {
        bail!(
            "platform service control is not available on this host. Run `inferra serve` for a foreground API + dashboard."
        );
    }
    if !status.installed {
        let paths = ctx.paths()?;
        bail!(
            "{} {}",
            missing_service_message(&status),
            runtime_install_guidance(&status, &paths.config_path)
        );
    }
    if status.is_running() && runtime_health_ok(ctx).await? {
        return finish_runtime_action(ctx, "start", "Runtime already running (API + dashboard)")
            .await;
    }
    start_supervisor()?;
    wait_for_runtime_health(ctx, 45).await?;
    finish_runtime_action(ctx, "start", "Started Inferra runtime (API + dashboard)").await
}

async fn runtime_stop(ctx: &AppContext) -> Result<()> {
    let status = query_supervisor_status()?;
    if !status.supported {
        bail!("platform service control is not available on this host.");
    }
    if !status.installed {
        bail!("{}", missing_service_message(&status));
    }
    let post_status = stop_supervisor()?;
    let payload = collect_runtime_payload(ctx).await?;
    let reachable = payload["reachable"].as_bool().unwrap_or(false);
    let message = if reachable {
        format!(
            "Stopped {} {}. The API is still responding, which usually means a foreground runtime is still active on the configured address.",
            post_status.manager_label(),
            post_status.service_name
        )
    } else {
        format!(
            "Stopped {} {} (API + dashboard)",
            post_status.manager_label(),
            post_status.service_name
        )
    };
    let payload = json!({
        "action": "stop",
        "ok": true,
        "service": post_status.service_name,
        "manager": post_status.kind,
        "reachable_after_stop": reachable,
    });
    emit_command_result(ctx, &payload, &[message]);
    Ok(())
}

async fn runtime_restart(ctx: &AppContext) -> Result<()> {
    let status = query_supervisor_status()?;
    if !status.supported {
        bail!("platform service control is not available on this host.");
    }
    if !status.installed {
        bail!("{}", missing_service_message(&status));
    }
    restart_supervisor()?;
    wait_for_runtime_health(ctx, 45).await?;
    finish_runtime_action(
        ctx,
        "restart",
        "Restarted Inferra runtime (API + dashboard)",
    )
    .await
}

async fn runtime_open(ctx: &AppContext) -> Result<()> {
    let payload = collect_runtime_payload(ctx).await?;
    let url = payload["dashboard_url"]
        .as_str()
        .context("dashboard URL unavailable")?;
    if !payload["reachable"].as_bool().unwrap_or(false) {
        bail!("Runtime is not reachable at {url}. Run `inferra runtime start` first.");
    }
    open_dashboard_url(url)?;
    let message = format!("Opened dashboard at {url}");
    emit_command_result(
        ctx,
        &json!({ "action": "open", "ok": true, "dashboard_url": url }),
        &[message],
    );
    Ok(())
}

async fn finish_runtime_action(ctx: &AppContext, action: &str, message: &str) -> Result<()> {
    let payload = collect_runtime_payload(ctx).await?;
    let dashboard = payload["dashboard_url"].as_str().unwrap_or_default();
    let lines = vec![
        message.to_string(),
        format!("Dashboard: {dashboard}"),
        format!("API health: {dashboard}api/health"),
    ];
    emit_command_result(
        ctx,
        &json!({
            "action": action,
            "ok": payload["reachable"],
            "dashboard_url": dashboard,
            "mode": payload["mode"],
        }),
        &lines,
    );
    Ok(())
}

async fn collect_runtime_payload(ctx: &AppContext) -> Result<JsonValue> {
    let paths = ctx.paths()?;
    let config = load_merged_config(&paths.config_path)?;
    let (bind_host, port) = server_listen(&config)?;
    let dashboard_url = format!("{}/", local_base_url(&bind_host, port));
    let health = ctx
        .api_request(reqwest::Method::GET, "/api/health", None)
        .await
        .ok();
    let reachable = health.is_some();
    let service = query_supervisor_status().ok();
    let service_installed = service
        .as_ref()
        .map(|status| status.installed)
        .unwrap_or(false);
    let service_state = service
        .as_ref()
        .and_then(|status| status.state.clone())
        .unwrap_or_else(|| "not_installed".to_string());
    let mode = runtime_mode_for(reachable, service.as_ref());
    let hint = runtime_hint_for(reachable, service.as_ref());
    Ok(json!({
        "reachable": reachable,
        "dashboard_url": dashboard_url,
        "health_status": health.as_ref().and_then(|payload| payload.get("status")).cloned().unwrap_or(JsonValue::Null),
        "mode": mode,
        "service_installed": service_installed,
        "service_state": service_state,
        "hint": hint,
        "config_path": paths.config_path.display().to_string(),
    }))
}

async fn runtime_health_ok(ctx: &AppContext) -> Result<bool> {
    Ok(ctx
        .api_request(reqwest::Method::GET, "/api/health", None)
        .await
        .is_ok())
}

async fn wait_for_runtime_health(ctx: &AppContext, timeout_secs: u64) -> Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        if runtime_health_ok(ctx).await? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let paths = ctx.paths()?;
    let status = query_supervisor_status().ok();
    let service_state = status
        .as_ref()
        .and_then(|row| row.state.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let log_target = status
        .as_ref()
        .and_then(|row| row.log_target.clone())
        .unwrap_or_else(|| "service manager logs".to_string());
    bail!(
        "runtime did not become healthy within {timeout_secs}s (service state: {service_state}); check {log_target} and {}",
        paths.config_path.display()
    );
}

fn runtime_mode_for(reachable: bool, service: Option<&SupervisorStatus>) -> &'static str {
    if let Some(status) = service {
        if status.is_running() {
            return status.kind;
        }
        if status.is_start_pending() || status.is_stop_pending() {
            return "service_pending";
        }
    }
    if reachable {
        "foreground"
    } else {
        "stopped"
    }
}

fn runtime_hint_for(reachable: bool, service: Option<&SupervisorStatus>) -> String {
    if reachable {
        return "inferra runtime open".to_string();
    }
    if let Some(status) = service {
        if status.is_start_pending() || status.is_stop_pending() {
            return "inferra runtime status".to_string();
        }
        if status.supported && status.installed {
            return "inferra runtime start".to_string();
        }
    }
    service_install_hint().to_string()
}

fn missing_service_message(status: &SupervisorStatus) -> String {
    format!(
        "{} {} is not installed.",
        status.manager_label(),
        status.service_name
    )
}

fn runtime_install_guidance(status: &SupervisorStatus, config_path: &std::path::Path) -> String {
    if status.kind == "windows_service" {
        format!(
            "Run {} or:\n  inferra --config \"{}\" --ui-dist <ui_dist> service install --startup auto",
            service_install_hint(),
            config_path.display()
        )
    } else {
        format!("Install it with: {}", service_install_hint())
    }
}

fn platform_service_next_step(config_path: &std::path::Path) -> String {
    if cfg!(windows) {
        format!(
            "inferra --config \"{}\" service install --startup auto",
            config_path.display()
        )
    } else {
        service_install_hint().to_string()
    }
}

fn open_dashboard_url(url: &str) -> Result<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .context("open dashboard in browser")?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .context("open dashboard in browser")?;
        return Ok(());
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .context("open dashboard in browser")?;
        return Ok(());
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = url;
        bail!("opening URLs is not supported on this platform");
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
            if !cfg!(windows) {
                bail!("service removal is only implemented for the Windows service. Use the platform uninstall flow documented in docs/operations/install.md.");
            }
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
            let status = start_supervisor()?;
            let payload = json!({
                "service": status.service_name,
                "manager": status.kind,
                "action": "start",
                "ok": true,
            });
            emit_command_result(
                ctx,
                &payload,
                &[format!(
                    "Started {} {}",
                    status.manager_label(),
                    status.service_name
                )],
            );
            Ok(())
        }
        Some(ServiceAction::Stop) => {
            let status = stop_supervisor()?;
            let payload = json!({
                "service": status.service_name,
                "manager": status.kind,
                "action": "stop",
                "ok": true,
            });
            emit_command_result(
                ctx,
                &payload,
                &[format!(
                    "Stopped {} {}",
                    status.manager_label(),
                    status.service_name
                )],
            );
            Ok(())
        }
        Some(ServiceAction::Restart) => {
            let status = restart_supervisor()?;
            let payload = json!({
                "service": status.service_name,
                "manager": status.kind,
                "action": "restart",
                "ok": true,
            });
            emit_command_result(
                ctx,
                &payload,
                &[format!(
                    "Restarted {} {}",
                    status.manager_label(),
                    status.service_name
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
            ctx.ui.banner(
                "Collectors",
                "Configured collectors and local API runtime state",
            );
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
                        collector
                            .get("error_count")
                            .and_then(JsonValue::as_u64)
                            .unwrap_or(0)
                            .to_string(),
                        collector
                            .get("last_error")
                            .and_then(JsonValue::as_str)
                            .map(shorten_cli_cell)
                            .unwrap_or_else(|| "-".to_string()),
                    ]
                })
                .collect::<Vec<_>>();
            ctx.ui.table(
                &[
                    "Collector",
                    "Status",
                    "Running",
                    "Source",
                    "Errors",
                    "Last error",
                ],
                rows,
            );
            if payload.get("fallback") == Some(&JsonValue::Bool(true)) {
                ctx.ui
                    .warning("The runtime API was not reachable, so this view used static config.");
            } else {
                let problem_collectors = payload["collectors"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|collector| {
                        collector
                            .get("error_count")
                            .and_then(JsonValue::as_u64)
                            .unwrap_or(0)
                            > 0
                    })
                    .collect::<Vec<_>>();
                if !problem_collectors.is_empty() {
                    ctx.ui.warning(
                        "Collector diagnostics include last_error, last_error_at, error_hint, and log_query in `inferra collectors status --json`.",
                    );
                }
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

fn shorten_cli_cell(value: &str) -> String {
    const MAX_LEN: usize = 96;
    if value.chars().count() <= MAX_LEN {
        return value.to_string();
    }
    let mut shortened = value.chars().take(MAX_LEN - 3).collect::<String>();
    shortened.push_str("...");
    shortened
}

pub fn run_config_command(ctx: &AppContext, action: ConfigAction) -> Result<()> {
    let paths = ctx.paths()?;
    let config = load_merged_config(&paths.config_path)?;
    match action {
        ConfigAction::Show => {
            let payload =
                json!({ "config": config, "config_path": paths.config_path.display().to_string() });
            if ctx.ui.is_json() {
                ctx.ui.print_json(&payload);
            } else {
                ctx.ui
                    .banner("Config", "Merged config with defaults applied");
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
                &[format!(
                    "Applied preset {name} to {}",
                    paths.config_path.display()
                )],
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
    let status = query_supervisor_status()?;
    let payload = json!({
        "service": status.service_name,
        "manager": status.kind,
        "manager_label": status.manager_label(),
        "supported": status.supported,
        "installed": status.installed,
        "state": status.state,
        "startup": status.startup,
        "log_target": status.log_target,
        "definition_path": status.definition_path.as_ref().map(|path| path.display().to_string()),
        "binary_path": status.binary_path,
    });
    if ctx.ui.is_json() {
        ctx.ui.print_json(&payload);
    } else {
        ctx.ui.banner("Service", "Platform service status");
        ctx.ui.kv_table([
            (
                "Manager",
                payload["manager_label"]
                    .as_str()
                    .unwrap_or("service manager")
                    .to_string(),
            ),
            (
                "Service",
                payload["service"].as_str().unwrap_or_default().to_string(),
            ),
            (
                "Supported",
                payload["supported"].as_bool().unwrap_or(false).to_string(),
            ),
            (
                "Installed",
                payload["installed"].as_bool().unwrap_or(false).to_string(),
            ),
            (
                "State",
                payload["state"]
                    .as_str()
                    .unwrap_or("not_installed")
                    .to_string(),
            ),
            (
                "Startup",
                payload["startup"].as_str().unwrap_or("-").to_string(),
            ),
            (
                "Logs",
                payload["log_target"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            ),
        ]);
        if let Some(definition) = payload.get("definition_path").and_then(JsonValue::as_str) {
            ctx.ui.paragraph(&format!("Definition: {definition}"));
        }
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
    let service_status = query_supervisor_status().ok();

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

    if let Some(status) = service_status.as_ref() {
        findings.push(json!({
            "name": "service_manager",
            "ok": status.supported,
            "message": if status.supported {
                format!(
                    "{} {} installed={} state={} startup={}",
                    status.manager_label(),
                    status.service_name,
                    status.installed,
                    status.state.as_deref().unwrap_or("unknown"),
                    status.startup.as_deref().unwrap_or("unknown")
                )
            } else {
                "No supported platform service manager detected; use `inferra serve` on this host."
                    .to_string()
            },
        }));
        if let Some(log_target) = status.log_target.as_deref() {
            findings.push(json!({
                "name": "logs",
                "ok": true,
                "message": format!("Service logs: {log_target}"),
            }));
        }
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
        let mut steps = vec![format!(
            "inferra --config \"{}\" init-db",
            paths.config_path.display()
        )];
        steps.push(platform_service_next_step(&paths.config_path));
        steps
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
        ctx.ui.banner(
            "Service Repair",
            "Non-destructive checks for local service readiness",
        );
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
    let service = match query_supervisor_status() {
        Ok(status) => json!({
            "manager": status.kind,
            "manager_label": status.manager_label(),
            "supported": status.supported,
            "installed": status.installed,
            "state": status.state,
            "startup": status.startup,
            "binary_path": status.binary_path,
            "definition_path": status.definition_path.as_ref().map(|path| path.display().to_string()),
            "log_target": status.log_target,
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
        ctx.ui.success(&format!(
            "API and dashboard reachable at {}",
            runtime["dashboard_url"].as_str().unwrap_or_default()
        ));
    } else {
        ctx.ui.warning(&format!(
            "API and dashboard are not responding. {}.",
            runtime_unreachable_hint()
        ));
    }
    ctx.ui.kv_table([
        (
            "Version",
            payload["version"].as_str().unwrap_or_default().to_string(),
        ),
        (
            "Dashboard",
            runtime["dashboard_url"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        ),
        (
            "Config path",
            payload["config_path"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
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
                "Manager",
                payload["service"]["manager_label"]
                    .as_str()
                    .unwrap_or("service manager")
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
        if let Some(definition_path) = payload["service"]
            .get("definition_path")
            .and_then(JsonValue::as_str)
        {
            ctx.ui.paragraph(&format!("Definition: {definition_path}"));
        }
        if let Some(log_target) = payload["service"]
            .get("log_target")
            .and_then(JsonValue::as_str)
        {
            ctx.ui.paragraph(&format!("Logs: {log_target}"));
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
        "inferra runtime status".to_string(),
        "inferra runtime start".to_string(),
        "inferra runtime open".to_string(),
        "inferra serve".to_string(),
        "inferra collectors status".to_string(),
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

    #[test]
    fn runtime_mode_prefers_pending_service_states() {
        let status = SupervisorStatus {
            kind: "systemd",
            supported: true,
            service_name: "inferra.service".into(),
            installed: true,
            state: Some("activating".into()),
            startup: Some("enabled".into()),
            definition_path: None,
            binary_path: None,
            log_target: None,
        };
        assert_eq!(runtime_mode_for(false, Some(&status)), "service_pending");
    }

    #[test]
    fn runtime_hint_prefers_status_during_pending_transition() {
        let status = SupervisorStatus {
            kind: "launchd",
            supported: true,
            service_name: "com.inferra.agent".into(),
            installed: true,
            state: Some("stopping".into()),
            startup: Some("run_at_load".into()),
            definition_path: None,
            binary_path: None,
            log_target: None,
        };
        assert_eq!(
            runtime_hint_for(false, Some(&status)),
            "inferra runtime status"
        );
    }
}
