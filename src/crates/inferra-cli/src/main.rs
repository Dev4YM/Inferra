use std::net::TcpListener;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use inferra_collectors::configured_collectors;
use inferra_config::{apply_config_put, load_merged_config, server_listen, write_config, Paths};
use inferra_core::{build_overview, build_workspace_map};
use inferra_storage::{initialize_databases, EventsStore, IncidentsStore};
use serde_json::{json, Value as JsonValue};
use toml::Value as TomlValue;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "inferra", version, about = "Inferra Rust runtime CLI")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    ui_dist: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Serve,
    Service {
        #[command(subcommand)]
        action: Option<ServiceAction>,
        #[arg(long, hide = true)]
        service_run: bool,
    },
    Setup {
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        skip_connection_test: bool,
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    InitDb,
    Incidents {
        #[command(subcommand)]
        action: IncidentAction,
    },
    Events {
        #[command(subcommand)]
        action: EventAction,
    },
    Services {
        #[command(subcommand)]
        action: ServiceDataAction,
    },
    Collectors {
        #[command(subcommand)]
        action: CollectorAction,
    },
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    Ai {
        #[command(subcommand)]
        action: AiAction,
    },
}

#[derive(Subcommand, Debug)]
enum ServiceAction {
    Install {
        #[arg(long, default_value = "auto")]
        startup: String,
    },
    Remove,
    Start,
    Stop,
    Restart,
    Status,
    Repair,
}

#[derive(Subcommand, Debug)]
enum IncidentAction {
    List(ListArgs),
    Show { incident_id: String },
}

#[derive(Subcommand, Debug)]
enum EventAction {
    List(EventListArgs),
    Show { event_id: String },
}

#[derive(Subcommand, Debug)]
enum ServiceDataAction {
    List(ListArgs),
    Show {
        service_id: String,
    },
    Events {
        service_id: String,
        #[command(flatten)]
        args: ListArgs,
    },
}

#[derive(Subcommand, Debug)]
enum CollectorAction {
    Status,
    Start,
    Stop,
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    Show,
    Get { key: String },
    Set { key: String, value: String },
    Preset { name: String },
}

#[derive(Subcommand, Debug)]
enum WorkspaceAction {
    Map,
    Services,
    Inspect {
        path: String,
    },
    Projects {
        #[arg(long, default_value_t = 4)]
        max_depth: usize,
        #[arg(long, default_value_t = 100)]
        max_results: usize,
    },
}

#[derive(Subcommand, Debug)]
enum AiAction {
    Status,
    Doctor,
    Ask {
        question: String,
        #[arg(long, default_value = "overview")]
        scope: String,
        #[arg(long)]
        mode: Option<String>,
    },
    Report {
        incident_id: String,
        #[arg(long)]
        mode: Option<String>,
    },
    Investigate {
        #[command(subcommand)]
        target: Option<InvestigateTarget>,
        #[arg(long)]
        mode: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum InvestigateTarget {
    Latest,
    Incident { incident_id: String },
    Service { service_id: String },
}

#[derive(Args, Debug, Clone)]
struct ListArgs {
    #[arg(long, default_value_t = 25)]
    limit: usize,
}

#[derive(Args, Debug, Clone)]
struct EventListArgs {
    #[arg(long, default_value_t = 100)]
    limit: usize,
    #[arg(long)]
    service: Option<String>,
    #[arg(long)]
    severity: Option<i64>,
    #[arg(long)]
    search: Option<String>,
    #[arg(long)]
    source_type: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            run_serve_command(cli.config.clone(), cli.ui_dist.clone()).await?;
        }
        Command::Service {
            action,
            service_run,
        } => match action {
            Some(ServiceAction::Install { startup }) => {
                install_windows_service(cli.config.clone(), cli.ui_dist.clone(), &startup)?;
            }
            Some(ServiceAction::Remove) => {
                inferra_windows_service::remove_service()?;
                println!(
                    "Removed Windows service {}",
                    inferra_windows_service::SERVICE_NAME
                );
            }
            Some(ServiceAction::Start) => {
                inferra_windows_service::start_service()?;
                println!(
                    "Started Windows service {}",
                    inferra_windows_service::SERVICE_NAME
                );
            }
            Some(ServiceAction::Stop) => {
                inferra_windows_service::stop_service()?;
                println!(
                    "Stopped Windows service {}",
                    inferra_windows_service::SERVICE_NAME
                );
            }
            Some(ServiceAction::Restart) => {
                inferra_windows_service::restart_service()?;
                println!(
                    "Restarted Windows service {}",
                    inferra_windows_service::SERVICE_NAME
                );
            }
            Some(ServiceAction::Status) => emit_service_status(cli.json)?,
            Some(ServiceAction::Repair) => run_service_repair(cli.config.clone(), cli.json)?,
            None => {
                let paths = Paths::discover(cli.config.clone())?;
                let ui_dist = resolve_ui_dist(cli.ui_dist.clone())?;
                if service_run {
                    inferra_windows_service::dispatch_service(paths, ui_dist)?;
                } else {
                    inferra_windows_service::run_service_or_foreground(paths, ui_dist).await?;
                }
            }
        },
        Command::Setup { data_dir, .. } => run_setup(cli.config.clone(), data_dir)?,
        Command::InitDb => run_init_db(cli.config.clone())?,
        Command::Incidents { action } => {
            run_incident_command(cli.config.clone(), cli.json, action)?
        }
        Command::Events { action } => run_event_command(cli.config.clone(), cli.json, action)?,
        Command::Services { action } => {
            run_service_data_command(cli.config.clone(), cli.json, action)?
        }
        Command::Collectors { action } => {
            run_collector_command(cli.config.clone(), cli.json, action).await?
        }
        Command::Config { action } => run_config_command(cli.config.clone(), cli.json, action)?,
        Command::Workspace { action } => {
            run_workspace_command(cli.config.clone(), cli.json, action).await?
        }
        Command::Ai { action } => run_ai_command(cli.config.clone(), cli.json, action).await?,
    }
    Ok(())
}

async fn run_serve_command(
    config_override: Option<PathBuf>,
    ui_dist_override: Option<PathBuf>,
) -> Result<()> {
    let paths = Paths::discover(config_override)?;
    let config = load_merged_config(&paths.config_path)?;
    let (host, port) = server_listen(&config)?;
    let ui_dist = resolve_ui_dist(ui_dist_override)?;
    match inferra_api::serve(paths, ui_dist).await {
        Ok(()) => Ok(()),
        Err(error) if is_addr_in_use_error(&error) => {
            handle_existing_runtime(host, port, error).await
        }
        Err(error) => Err(error),
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .try_init();
}

async fn handle_existing_runtime(host: String, port: u16, error: anyhow::Error) -> Result<()> {
    let base_url = format!("http://{host}:{port}");
    let health_url = format!("{base_url}/api/health");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .context("build runtime probe client")?;
    match client.get(&health_url).send().await {
        Ok(response) if response.status().is_success() => {
            println!("Inferra is already running at {base_url}/");
            println!("Health endpoint: {health_url}");
            println!("Use `inferra service status`, `inferra collectors status`, or open the dashboard instead of starting a second listener.");
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

fn resolve_ui_dist(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        for candidate in ui_dist_candidates_from_executable(&exe) {
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    let repo_root = resolve_repo_root_from_manifest()?;
    Ok(repo_root.join("src").join("web").join("ui_dist"))
}

fn ui_dist_candidates_from_executable(exe: &std::path::Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(parent) = exe.parent() {
        candidates.push(parent.join("runtime-assets").join("ui_dist"));
        if let Some(grandparent) = parent.parent() {
            candidates.push(grandparent.join("runtime-assets").join("ui_dist"));
        }
    }
    candidates
}

fn resolve_repo_root_from_manifest() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .find(|path| path.join("pyproject.toml").exists() && path.join("README.md").exists())
        .map(|path| path.to_path_buf())
        .context("resolve repo root from manifest dir")
}

fn run_setup(config_override: Option<PathBuf>, data_dir: Option<PathBuf>) -> Result<()> {
    let config_path = config_override.unwrap_or_else(|| {
        let pd = std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".into());
        PathBuf::from(pd).join("Inferra").join("inferra.toml")
    });
    let base = load_merged_config(&config_path)?;
    let final_cfg = if let Some(data_dir) = data_dir {
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
    println!("Wrote config {}", config_path.display());
    Ok(())
}

fn run_init_db(config_override: Option<PathBuf>) -> Result<()> {
    let paths = Paths::discover(config_override)?;
    initialize_databases(&paths.events_db, &paths.incidents_db)?;
    println!(
        "Initialized Rust SQLite databases under {}",
        paths.data_dir.display()
    );
    Ok(())
}

fn install_windows_service(
    config_override: Option<PathBuf>,
    ui_dist_override: Option<PathBuf>,
    startup: &str,
) -> Result<()> {
    if !cfg!(windows) {
        bail!("Windows service install is only supported on Windows");
    }
    let paths = Paths::discover(config_override)?;
    let ui_dist = resolve_ui_dist(ui_dist_override)?;
    let startup = inferra_windows_service::ServiceStartup::from_cli(startup)?;
    let binary = std::env::current_exe().context("resolve current inferra executable")?;
    inferra_windows_service::install_service(
        &binary,
        &inferra_windows_service::ServiceInstallOptions {
            config_path: paths.config_path,
            ui_dist,
            startup,
        },
    )?;
    println!(
        "Installed Windows service {}",
        inferra_windows_service::SERVICE_NAME
    );
    Ok(())
}

fn emit_service_status(json_output: bool) -> Result<()> {
    let status = inferra_windows_service::query_service_status()?;
    let payload = json!({
        "service": inferra_windows_service::SERVICE_NAME,
        "installed": status.installed,
        "state": status.state,
        "startup": status.startup,
        "log_path": status.log_path.display().to_string(),
        "binary_path": status.binary_path,
    });
    emit_command_result(
        json_output,
        &payload,
        &[
            format!("service={}", inferra_windows_service::SERVICE_NAME),
            format!(
                "installed={}",
                payload["installed"].as_bool().unwrap_or(false)
            ),
            format!(
                "state={}",
                payload["state"].as_str().unwrap_or("not_installed")
            ),
            format!("startup={}", payload["startup"].as_str().unwrap_or("-")),
            format!(
                "log_path={}",
                payload["log_path"].as_str().unwrap_or_default()
            ),
        ],
    );
    Ok(())
}

fn run_service_repair(config_override: Option<PathBuf>, json_output: bool) -> Result<()> {
    let paths = Paths::discover(config_override)?;
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

    let ui_dist = resolve_ui_dist(None);
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

    let ok = findings.iter().all(|item| {
        item.get("ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    });
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
    emit_command_result(
        json_output,
        &payload,
        &[
            format!("service_repair_ok={ok}"),
            format!("config={}", paths.config_path.display()),
            format!("data_dir={}", paths.data_dir.display()),
            format!("bind={host}:{port} ok={bind_ok}"),
            format!("log_path={}", log_path.display()),
            if ok {
                "Next: inferra service status".to_string()
            } else {
                format!(
                    "Next: inferra --config \"{}\" service install --startup auto",
                    paths.config_path.display()
                )
            },
        ],
    );
    if ok {
        Ok(())
    } else {
        bail!("service repair found issues")
    }
}

fn emit_command_result(json_output: bool, payload: &JsonValue, lines: &[String]) {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
        );
    } else {
        for line in lines {
            println!("{line}");
        }
    }
}

fn run_incident_command(
    config_override: Option<PathBuf>,
    json_output: bool,
    action: IncidentAction,
) -> Result<()> {
    let paths = Paths::discover(config_override)?;
    let Some(store) = IncidentsStore::open(&paths.incidents_db)? else {
        emit_command_result(
            json_output,
            &json!({ "incidents": [] }),
            &["No incidents database found.".to_string()],
        );
        return Ok(());
    };
    match action {
        IncidentAction::List(args) => {
            let incidents = store.active_incidents(args.limit)?;
            let payload = json!({ "incidents": incidents });
            let lines = incidents
                .iter()
                .map(|incident| {
                    format!(
                        "{} state={} severity={} service={} events={}",
                        incident.incident_id,
                        incident.state,
                        incident.severity,
                        incident.primary_service,
                        incident.event_count.unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>();
            if lines.is_empty() {
                emit_command_result(json_output, &payload, &["No active incidents.".to_string()]);
            } else {
                emit_command_result(json_output, &payload, &lines);
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
            emit_command_result(
                json_output,
                &payload,
                &[
                    format!("incident={incident_id}"),
                    format!(
                        "state={}",
                        payload["incident"]["state"].as_str().unwrap_or("unknown")
                    ),
                    format!(
                        "severity={}",
                        payload["incident"]["severity"].as_i64().unwrap_or_default()
                    ),
                    format!(
                        "service={}",
                        payload["incident"]["primary_service"]
                            .as_str()
                            .unwrap_or_default()
                    ),
                ],
            );
        }
    }
    Ok(())
}

fn run_event_command(
    config_override: Option<PathBuf>,
    json_output: bool,
    action: EventAction,
) -> Result<()> {
    let paths = Paths::discover(config_override)?;
    let Some(store) = EventsStore::open(&paths.events_db)? else {
        emit_command_result(
            json_output,
            &json!({ "events": [] }),
            &["No events database found.".to_string()],
        );
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
            let lines = events
                .iter()
                .map(|event| {
                    format!(
                        "{} severity={} service={} {}",
                        event.event_id.as_deref().unwrap_or("-"),
                        event
                            .severity
                            .as_ref()
                            .and_then(JsonValue::as_i64)
                            .unwrap_or_default(),
                        event.service_id.as_deref().unwrap_or("-"),
                        event.message.as_deref().unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>();
            if lines.is_empty() {
                emit_command_result(json_output, &payload, &["No matching events.".to_string()]);
            } else {
                emit_command_result(json_output, &payload, &lines);
            }
        }
        EventAction::Show { event_id } => {
            let event = store
                .get_event(&event_id)?
                .with_context(|| format!("event not found: {event_id}"))?;
            let payload = json!({ "event": event });
            emit_command_result(
                json_output,
                &payload,
                &[
                    format!("event={event_id}"),
                    format!(
                        "service={}",
                        payload["event"]["service_id"].as_str().unwrap_or_default()
                    ),
                    format!(
                        "severity={}",
                        payload["event"]["severity"].as_i64().unwrap_or_default()
                    ),
                    payload["event"]["message"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                ],
            );
        }
    }
    Ok(())
}

fn run_service_data_command(
    config_override: Option<PathBuf>,
    json_output: bool,
    action: ServiceDataAction,
) -> Result<()> {
    let paths = Paths::discover(config_override)?;
    let config = load_merged_config(&paths.config_path)?;
    let overview = build_overview(&config, &paths)?;
    let services = overview.dashboard.services.unwrap_or_default();
    match action {
        ServiceDataAction::List(args) => {
            let rows = services.into_iter().take(args.limit).collect::<Vec<_>>();
            let payload = json!({ "services": rows });
            let lines = payload["services"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|service| {
                    format!(
                        "{} status={} events={} errors={}",
                        service
                            .get("service_id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default(),
                        service
                            .get("status")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("unknown"),
                        service
                            .get("event_count")
                            .and_then(JsonValue::as_i64)
                            .unwrap_or_default(),
                        service
                            .get("error_count")
                            .and_then(JsonValue::as_i64)
                            .unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>();
            emit_command_result(json_output, &payload, &lines);
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
            emit_command_result(
                json_output,
                &payload,
                &[
                    format!("service={service_id}"),
                    format!(
                        "status={}",
                        payload["service"]["status"].as_str().unwrap_or("unknown")
                    ),
                    format!(
                        "events={}",
                        payload["service"]["event_count"]
                            .as_i64()
                            .unwrap_or_default()
                    ),
                    format!(
                        "errors={}",
                        payload["service"]["error_count"]
                            .as_i64()
                            .unwrap_or_default()
                    ),
                ],
            );
        }
        ServiceDataAction::Events { service_id, args } => {
            let events = EventsStore::open(&paths.events_db)?
                .map(|store| store.events_for_service(&service_id, args.limit))
                .transpose()?
                .unwrap_or_default();
            let payload = json!({ "service_id": service_id, "events": events });
            let lines = payload["events"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|event| {
                    format!(
                        "{} severity={} {}",
                        event
                            .get("event_id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("-"),
                        event
                            .get("severity")
                            .and_then(JsonValue::as_i64)
                            .unwrap_or_default(),
                        event
                            .get("message")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default(),
                    )
                })
                .collect::<Vec<_>>();
            emit_command_result(json_output, &payload, &lines);
        }
    }
    Ok(())
}

async fn run_collector_command(
    config_override: Option<PathBuf>,
    json_output: bool,
    action: CollectorAction,
) -> Result<()> {
    match action {
        CollectorAction::Status => {
            let paths = Paths::discover(config_override.clone())?;
            let config = load_merged_config(&paths.config_path)?;
            let payload = match api_request(
                config_override,
                reqwest::Method::GET,
                "/api/collectors",
                None,
            )
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
                }),
            };
            let lines = payload["collectors"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|collector| {
                    format!(
                        "{} status={} running={}",
                        collector
                            .get("collector_id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default(),
                        collector
                            .get("status")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("unknown"),
                        collector
                            .get("is_running")
                            .and_then(JsonValue::as_bool)
                            .unwrap_or(false),
                    )
                })
                .collect::<Vec<_>>();
            emit_command_result(json_output, &payload, &lines);
        }
        CollectorAction::Start => {
            let payload = api_request(
                config_override,
                reqwest::Method::POST,
                "/api/collectors/start",
                None,
            )
            .await?;
            emit_command_result(
                json_output,
                &payload,
                &["Requested collector start via local API.".to_string()],
            );
        }
        CollectorAction::Stop => {
            let payload = api_request(
                config_override,
                reqwest::Method::POST,
                "/api/collectors/stop",
                None,
            )
            .await?;
            emit_command_result(
                json_output,
                &payload,
                &["Requested collector stop via local API.".to_string()],
            );
        }
    }
    Ok(())
}

fn run_config_command(
    config_override: Option<PathBuf>,
    json_output: bool,
    action: ConfigAction,
) -> Result<()> {
    let paths = Paths::discover(config_override)?;
    let config = load_merged_config(&paths.config_path)?;
    match action {
        ConfigAction::Show => emit_command_result(
            json_output,
            &json!({ "config": config }),
            &[format!("config_path={}", paths.config_path.display())],
        ),
        ConfigAction::Get { key } => {
            let value = toml_value_at(&config, &key);
            emit_command_result(
                json_output,
                &json!({ "key": key, "value": value.cloned().map(toml_to_json).unwrap_or(JsonValue::Null) }),
                &[format!("{key}={}", toml_to_display(value))],
            );
        }
        ConfigAction::Set { key, value } => {
            let patch = patch_from_path(&key, parse_cli_value(&value));
            let next = apply_config_put(config, &patch)?;
            write_config(&paths.config_path, &next)?;
            emit_command_result(
                json_output,
                &json!({ "updated": true, "key": key, "value": value }),
                &[format!("Updated {key} in {}", paths.config_path.display())],
            );
        }
        ConfigAction::Preset { name } => {
            let next = apply_config_put(config, &preset_patch(&name)?)?;
            write_config(&paths.config_path, &next)?;
            emit_command_result(
                json_output,
                &json!({ "preset": name, "applied": true }),
                &[format!(
                    "Applied preset {name} to {}",
                    paths.config_path.display()
                )],
            );
        }
    }
    Ok(())
}

async fn run_workspace_command(
    config_override: Option<PathBuf>,
    json_output: bool,
    action: WorkspaceAction,
) -> Result<()> {
    let paths = Paths::discover(config_override.clone())?;
    let config = load_merged_config(&paths.config_path)?;
    match action {
        WorkspaceAction::Map => {
            let payload = serde_json::to_value(build_workspace_map(&config, &paths)?)?;
            emit_command_result(
                json_output,
                &payload,
                &[format!(
                    "projects={} service_mappings={}",
                    payload["projects"]
                        .as_array()
                        .map(|items| items.len())
                        .unwrap_or(0),
                    payload["service_mappings"]
                        .as_array()
                        .map(|items| items.len())
                        .unwrap_or(0),
                )],
            );
        }
        WorkspaceAction::Services => {
            let payload = serde_json::to_value(build_workspace_map(&config, &paths)?)?;
            let subset = json!({
                "service_mappings": payload["service_mappings"].clone(),
                "unmapped_services": payload["unmapped_services"].clone(),
            });
            emit_command_result(
                json_output,
                &subset,
                &[format!(
                    "mapped={} unmapped={}",
                    subset["service_mappings"]
                        .as_array()
                        .map(|items| items.len())
                        .unwrap_or(0),
                    subset["unmapped_services"]
                        .as_array()
                        .map(|items| items.len())
                        .unwrap_or(0),
                )],
            );
        }
        WorkspaceAction::Inspect { path } => {
            let payload = api_request(
                config_override,
                reqwest::Method::GET,
                &format!("/api/workspace/inspect?path={}", url_encode(&path)),
                None,
            )
            .await?;
            emit_command_result(json_output, &payload, &[format!("path={path}")]);
        }
        WorkspaceAction::Projects {
            max_depth,
            max_results,
        } => {
            let payload = api_request(
                config_override,
                reqwest::Method::GET,
                &format!("/api/workspace/projects?max_depth={max_depth}&max_results={max_results}"),
                None,
            )
            .await?;
            emit_command_result(
                json_output,
                &payload,
                &[format!(
                    "projects={}",
                    payload["projects"]
                        .as_array()
                        .map(|items| items.len())
                        .unwrap_or(0),
                )],
            );
        }
    }
    Ok(())
}

async fn run_ai_command(
    config_override: Option<PathBuf>,
    json_output: bool,
    action: AiAction,
) -> Result<()> {
    let payload = match action {
        AiAction::Status => {
            api_request(
                config_override,
                reqwest::Method::GET,
                "/api/ai/status",
                None,
            )
            .await?
        }
        AiAction::Doctor => {
            api_request(
                config_override,
                reqwest::Method::GET,
                "/api/ai/doctor",
                None,
            )
            .await?
        }
        AiAction::Ask {
            question,
            scope,
            mode,
        } => {
            api_request(
                config_override,
                reqwest::Method::POST,
                "/api/ai/ask",
                Some(json!({ "question": question, "scope": scope, "mode": mode })),
            )
            .await?
        }
        AiAction::Report { incident_id, mode } => {
            let query = mode.map(|m| format!("?mode={m}")).unwrap_or_default();
            api_request(
                config_override,
                reqwest::Method::GET,
                &format!("/api/ai/report/{incident_id}{query}"),
                None,
            )
            .await?
        }
        AiAction::Investigate { target, mode } => {
            let query = mode.map(|m| format!("?mode={m}")).unwrap_or_default();
            match target.unwrap_or(InvestigateTarget::Latest) {
                InvestigateTarget::Latest => {
                    api_request(
                        config_override,
                        reqwest::Method::GET,
                        &format!("/api/investigate/now{query}"),
                        None,
                    )
                    .await?
                }
                InvestigateTarget::Incident { incident_id } => {
                    api_request(
                        config_override,
                        reqwest::Method::GET,
                        &format!("/api/investigate/incident/{incident_id}{query}"),
                        None,
                    )
                    .await?
                }
                InvestigateTarget::Service { service_id } => {
                    api_request(
                        config_override,
                        reqwest::Method::GET,
                        &format!("/api/investigate/service/{service_id}{query}"),
                        None,
                    )
                    .await?
                }
            }
        }
    };
    let lines = if let Some(headline) = payload
        .get("output")
        .and_then(|value| value.get("headline"))
        .and_then(JsonValue::as_str)
    {
        vec![headline.to_string()]
    } else if let Some(status) = payload.get("status").and_then(JsonValue::as_str) {
        vec![format!("status={status}")]
    } else {
        vec!["AI command completed.".to_string()]
    };
    emit_command_result(json_output, &payload, &lines);
    Ok(())
}

async fn api_request(
    config_override: Option<PathBuf>,
    method: reqwest::Method,
    path: &str,
    payload: Option<JsonValue>,
) -> Result<JsonValue> {
    let paths = Paths::discover(config_override)?;
    let config = load_merged_config(&paths.config_path)?;
    let (host, port) = server_listen(&config)?;
    let url = format!("http://{host}:{port}{path}");
    let server = config.get("server").and_then(TomlValue::as_table);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build local API client")?;
    let mut request = client.request(method, &url);
    if let Some(token_env) = server
        .and_then(|table| table.get("auth_token_env"))
        .and_then(TomlValue::as_str)
        .filter(|value| !value.is_empty())
    {
        if let Ok(token) = std::env::var(token_env) {
            request = request.bearer_auth(token);
        }
    }
    if let Some(payload) = payload {
        request = request.json(&payload);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("request local API at {url}; start `inferra serve` if needed"))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .context("read local API response body")?;
    if !status.is_success() {
        bail!("local API request failed ({status}): {text}");
    }
    serde_json::from_str(&text).with_context(|| format!("parse local API JSON from {url}"))
}

fn toml_value_at<'a>(value: &'a TomlValue, key: &str) -> Option<&'a TomlValue> {
    let mut current = value;
    for part in key.split('.').filter(|part| !part.is_empty()) {
        current = current.get(part)?;
    }
    Some(current)
}

fn toml_to_json(value: TomlValue) -> JsonValue {
    serde_json::to_value(value).unwrap_or(JsonValue::Null)
}

fn toml_to_display(value: Option<&TomlValue>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<missing>".to_string())
}

fn patch_from_path(key: &str, value: JsonValue) -> JsonValue {
    let mut parts = key
        .split('.')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    parts.reverse();
    let mut current = value;
    for part in parts {
        current = json!({ part: current });
    }
    current
}

fn parse_cli_value(raw: &str) -> JsonValue {
    if let Ok(value) = serde_json::from_str::<JsonValue>(raw) {
        value
    } else if let Ok(value) = raw.parse::<i64>() {
        JsonValue::from(value)
    } else if let Ok(value) = raw.parse::<f64>() {
        JsonValue::from(value)
    } else if let Ok(value) = raw.parse::<bool>() {
        JsonValue::from(value)
    } else {
        JsonValue::String(raw.to_string())
    }
}

fn preset_patch(name: &str) -> Result<JsonValue> {
    match name {
        "web-only" => Ok(json!({
            "collectors": {
                "auto_start": true,
                "app": {"enabled": true, "enable_main_api": true},
                "host_metrics": {"enabled": true},
                "process": {"enabled": true},
                "docker": {"enabled": false},
                "journald": {"enabled": false},
                "linux_syslog": {"enabled": false},
                "windows_eventlog": {"enabled": false},
                "windows_service": {"enabled": false},
                "kubernetes": {"enabled": false},
            }
        })),
        "windows-server" => Ok(json!({
            "collectors": {
                "auto_start": true,
                "host_metrics": {"enabled": true},
                "process": {"enabled": true},
                "windows_eventlog": {"enabled": true},
                "windows_service": {"enabled": true},
                "linux_syslog": {"enabled": false},
                "journald": {"enabled": false},
                "docker": {"enabled": false},
                "kubernetes": {"enabled": false},
            }
        })),
        "linux-node" => Ok(json!({
            "collectors": {
                "auto_start": true,
                "host_metrics": {"enabled": true},
                "process": {"enabled": true},
                "linux_syslog": {"enabled": true},
                "journald": {"enabled": true},
                "windows_eventlog": {"enabled": false},
                "windows_service": {"enabled": false},
                "docker": {"enabled": false},
                "kubernetes": {"enabled": false},
            }
        })),
        "docker-host" => Ok(json!({
            "collectors": {
                "auto_start": true,
                "host_metrics": {"enabled": true},
                "process": {"enabled": true},
                "docker": {"enabled": true},
                "linux_syslog": {"enabled": false},
                "journald": {"enabled": false},
                "windows_eventlog": {"enabled": false},
                "windows_service": {"enabled": false},
                "kubernetes": {"enabled": false},
            }
        })),
        "kubernetes" => Ok(json!({
            "collectors": {
                "auto_start": true,
                "host_metrics": {"enabled": true},
                "process": {"enabled": true},
                "kubernetes": {"enabled": true},
                "linux_syslog": {"enabled": false},
                "journald": {"enabled": false},
                "windows_eventlog": {"enabled": false},
                "windows_service": {"enabled": false},
                "docker": {"enabled": false},
            }
        })),
        other => bail!("unknown preset: {other}"),
    }
}

fn url_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{:02X}", byte),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_accepts_service_repair_as_native_command() {
        let cli = Cli::try_parse_from(["inferra", "service", "repair"]).expect("parse repair");
        match cli.command {
            Some(Command::Service {
                action: Some(ServiceAction::Repair),
                ..
            }) => {}
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn parser_rejects_unknown_command_groups() {
        let error = Cli::try_parse_from(["inferra", "--json", "guide", "--profile", "operator"])
            .expect_err("guide should be rejected");
        let rendered = error.to_string();
        assert!(rendered.contains("unrecognized subcommand"));
    }

    #[test]
    fn parser_accepts_incidents_and_ai_commands() {
        let cli = Cli::try_parse_from(["inferra", "incidents", "list"]).expect("parse incidents");
        assert!(matches!(
            cli.command,
            Some(Command::Incidents {
                action: IncidentAction::List(_)
            })
        ));

        let ai = Cli::try_parse_from(["inferra", "ai", "investigate", "service", "api"])
            .expect("parse ai investigate");
        assert!(matches!(
            ai.command,
            Some(Command::Ai {
                action: AiAction::Investigate {
                    target: Some(InvestigateTarget::Service { .. }),
                    ..
                }
            })
        ));
    }

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
    fn ui_dist_candidates_cover_bin_and_sibling_layouts() {
        let candidates = ui_dist_candidates_from_executable(std::path::Path::new(
            r"C:\Program Files\Inferra\bin\inferra.exe",
        ));
        assert_eq!(
            candidates[0],
            PathBuf::from(r"C:\Program Files\Inferra\bin\runtime-assets\ui_dist")
        );
        assert_eq!(
            candidates[1],
            PathBuf::from(r"C:\Program Files\Inferra\runtime-assets\ui_dist")
        );
    }
}
