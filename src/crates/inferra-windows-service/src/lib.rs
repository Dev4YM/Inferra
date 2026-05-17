use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use inferra_config::Paths;

pub const SERVICE_NAME: &str = "Inferra";
pub const SERVICE_DISPLAY_NAME: &str = "Inferra";
pub const SERVICE_DESCRIPTION: &str = "Local-first runtime failure explanation service";

#[derive(Debug, Clone, Copy)]
pub enum ServiceStartup {
    Auto,
    Manual,
    Disabled,
}

impl ServiceStartup {
    pub fn from_cli(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "automatic" => Ok(Self::Auto),
            "manual" | "demand" => Ok(Self::Manual),
            "disabled" => Ok(Self::Disabled),
            other => bail!("unsupported startup mode: {other}"),
        }
    }

    fn as_sc_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "demand",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServiceInstallOptions {
    pub config_path: PathBuf,
    pub ui_dist: PathBuf,
    pub startup: ServiceStartup,
}

#[derive(Debug, Clone)]
pub struct ServiceStatus {
    pub installed: bool,
    pub state: Option<String>,
    pub startup: Option<String>,
    pub binary_path: Option<String>,
    pub log_path: PathBuf,
}

/// Service-host shell for the Rust runtime.
pub async fn run_service_or_foreground(paths: Paths, ui_dist: PathBuf) -> Result<()> {
    tracing::info!("starting Rust runtime in service-compatible mode");
    inferra_api::serve(paths, ui_dist).await
}

pub fn service_log_path() -> PathBuf {
    let pd = std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".into());
    PathBuf::from(pd)
        .join("Inferra")
        .join("logs")
        .join("serve.log")
}

pub fn install_service(binary_path: &Path, options: &ServiceInstallOptions) -> Result<()> {
    #[cfg(not(windows))]
    {
        let _ = binary_path;
        let _ = options;
        bail!("Windows service install is only supported on Windows")
    }
    #[cfg(windows)]
    {
        let create_args = build_sc_create_args(binary_path, options);
        let mut last_error = None;
        for attempt in 0..10 {
            match run_sc(&create_args) {
                Ok(()) => {
                    last_error = None;
                    break;
                }
                Err(error) => {
                    let text = error.to_string().to_ascii_lowercase();
                    if text.contains("marked for deletion") && attempt < 9 {
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        last_error = Some(error);
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        if let Some(error) = last_error {
            return Err(error);
        }
        run_sc(&[
            "description".into(),
            SERVICE_NAME.into(),
            SERVICE_DESCRIPTION.into(),
        ])?;
        Ok(())
    }
}

pub fn remove_service() -> Result<()> {
    #[cfg(not(windows))]
    {
        bail!("Windows service remove is only supported on Windows")
    }
    #[cfg(windows)]
    {
        let _ = run_sc(&["stop".into(), SERVICE_NAME.into()]);
        let output = run_sc_output(&["delete".into(), SERVICE_NAME.into()])?;
        if !output.status.success() {
            let text = String::from_utf8_lossy(&output.stderr);
            if !is_missing_service_error(&text) {
                bail!("failed to delete service: {}", text.trim());
            }
        }
        Ok(())
    }
}

pub fn start_service() -> Result<()> {
    #[cfg(not(windows))]
    {
        bail!("Windows service start is only supported on Windows")
    }
    #[cfg(windows)]
    {
        run_sc(&["start".into(), SERVICE_NAME.into()])
    }
}

pub fn stop_service() -> Result<()> {
    #[cfg(not(windows))]
    {
        bail!("Windows service stop is only supported on Windows")
    }
    #[cfg(windows)]
    {
        run_sc(&["stop".into(), SERVICE_NAME.into()])
    }
}

pub fn restart_service() -> Result<()> {
    if let Err(error) = stop_service() {
        let text = error.to_string();
        if !is_non_running_service_error(&text) && !is_missing_service_error(&text) {
            return Err(error);
        }
    }
    start_service()
}

pub fn query_service_status() -> Result<ServiceStatus> {
    #[cfg(not(windows))]
    {
        bail!("Windows service status is only supported on Windows")
    }
    #[cfg(windows)]
    {
        let query = run_sc_output(&["query".into(), SERVICE_NAME.into()])?;
        if !query.status.success() {
            return Ok(ServiceStatus {
                installed: false,
                state: None,
                startup: None,
                binary_path: None,
                log_path: service_log_path(),
            });
        }
        let query_text = String::from_utf8_lossy(&query.stdout);
        let qc = run_sc_output(&["qc".into(), SERVICE_NAME.into()])?;
        let qc_text = String::from_utf8_lossy(&qc.stdout);
        Ok(ServiceStatus {
            installed: true,
            state: parse_sc_field(&query_text, "STATE").and_then(parse_state_value),
            startup: parse_sc_field(&qc_text, "START_TYPE").and_then(parse_state_value),
            binary_path: parse_sc_field(&qc_text, "BINARY_PATH_NAME"),
            log_path: service_log_path(),
        })
    }
}

pub fn dispatch_service(paths: Paths, ui_dist: PathBuf) -> Result<()> {
    #[cfg(not(windows))]
    {
        let _ = paths;
        let _ = ui_dist;
        bail!("Windows service dispatch is only supported on Windows")
    }
    #[cfg(windows)]
    {
        imp::dispatch_service(paths, ui_dist)
    }
}

fn build_service_command_line(binary_path: &Path, options: &ServiceInstallOptions) -> String {
    let mut parts = vec![quote_windows_executable(&binary_path.display().to_string())];
    parts.push("--config".into());
    parts.push(quote_windows_arg(
        &options.config_path.display().to_string(),
    ));
    parts.push("--ui-dist".into());
    parts.push(quote_windows_arg(&options.ui_dist.display().to_string()));
    parts.push("service".into());
    parts.push("--service-run".into());
    parts.join(" ")
}

fn build_sc_create_args(binary_path: &Path, options: &ServiceInstallOptions) -> Vec<String> {
    vec![
        "create".into(),
        SERVICE_NAME.into(),
        "binPath=".into(),
        build_service_command_line(binary_path, options),
        "start=".into(),
        options.startup.as_sc_value().into(),
        "DisplayName=".into(),
        SERVICE_DISPLAY_NAME.into(),
    ]
}

fn quote_windows_executable(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

fn quote_windows_arg(value: &str) -> String {
    if value.contains([' ', '\t', '"']) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

fn run_sc(args: &[String]) -> Result<()> {
    let output = run_sc_output(args)?;
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "sc.exe {} failed: {}",
        args.join(" "),
        sc_output_text(&output)
    )
}

fn run_sc_output(args: &[String]) -> Result<std::process::Output> {
    Command::new("sc.exe")
        .args(args)
        .output()
        .with_context(|| format!("run sc.exe {}", args.join(" ")))
}

fn sc_output_text(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => "(no output)".to_string(),
    }
}

fn parse_sc_field(text: &str, field_name: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim_start();
        let (name, tail) = trimmed.split_once(':')?;
        if !name.trim().eq_ignore_ascii_case(field_name) {
            continue;
        }
        return Some(tail.trim().to_string());
    }
    None
}

fn parse_state_value(raw: String) -> Option<String> {
    let mut parts = raw.split_whitespace();
    let first = parts.next()?;
    let second = parts.next();
    let value = if first.chars().all(|c| c.is_ascii_digit()) {
        second.unwrap_or(first)
    } else {
        first
    };
    Some(value.to_ascii_lowercase())
}

fn append_service_log(message: &str) {
    let log_path = service_log_path();
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut fp) = OpenOptions::new().create(true).append(true).open(log_path) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(fp, "[{timestamp}] {message}");
    }
}

fn is_missing_service_error(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("does not exist") || lowered.contains("1060")
}

fn is_non_running_service_error(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("service has not been started")
        || lowered.contains("not started")
        || lowered.contains("1062")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;

    fn failing_status() -> std::process::ExitStatus {
        #[cfg(unix)]
        {
            std::process::ExitStatus::from_raw(1 << 8)
        }
        #[cfg(windows)]
        {
            std::process::ExitStatus::from_raw(1)
        }
    }

    #[test]
    fn build_service_command_line_quotes_executable_and_embedded_paths() {
        let command = build_service_command_line(
            Path::new(r"D:\Apps\inferra-rust.exe"),
            &ServiceInstallOptions {
                config_path: PathBuf::from(r"C:\ProgramData\Inferra\inferra.toml"),
                ui_dist: PathBuf::from(r"D:\Apps\runtime assets\ui_dist"),
                startup: ServiceStartup::Auto,
            },
        );
        assert!(command.starts_with("\"D:\\Apps\\inferra-rust.exe\" --config"));
        assert!(command.contains("--ui-dist \"D:\\Apps\\runtime assets\\ui_dist\""));
        assert!(command.ends_with("service --service-run"));
    }

    #[test]
    fn build_sc_create_args_splits_option_names_from_values() {
        let args = build_sc_create_args(
            Path::new(r"D:\Apps\inferra-rust.exe"),
            &ServiceInstallOptions {
                config_path: PathBuf::from(r"C:\ProgramData\Inferra\inferra.toml"),
                ui_dist: PathBuf::from(r"D:\Apps\runtime assets\ui_dist"),
                startup: ServiceStartup::Auto,
            },
        );
        assert_eq!(args[0], "create");
        assert_eq!(args[1], SERVICE_NAME);
        assert_eq!(args[2], "binPath=");
        assert!(args[3].starts_with("\"D:\\Apps\\inferra-rust.exe\" --config"));
        assert_eq!(args[4], "start=");
        assert_eq!(args[5], "auto");
        assert_eq!(args[6], "DisplayName=");
        assert_eq!(args[7], SERVICE_DISPLAY_NAME);
    }

    #[test]
    fn sc_output_text_prefers_stdout_when_present() {
        let output = std::process::Output {
            status: failing_status(),
            stdout: b"[SC] CreateService FAILED 1072: The specified service has been marked for deletion.".to_vec(),
            stderr: Vec::new(),
        };
        let text = sc_output_text(&output);
        assert!(text.contains("marked for deletion"));
    }

    #[test]
    fn parse_sc_field_requires_exact_field_name() {
        let text = "SERVICE_NAME: Inferra\n        STATE              : 4  RUNNING\n";
        assert_eq!(parse_sc_field(text, "STATE").as_deref(), Some("4  RUNNING"));
        assert!(parse_sc_field(text, "ATE").is_none());
    }

    #[test]
    fn service_startup_from_cli_accepts_aliases() {
        assert!(ServiceStartup::from_cli("auto").is_ok());
        assert!(ServiceStartup::from_cli("automatic").is_ok());
        assert!(ServiceStartup::from_cli("manual").is_ok());
        assert!(ServiceStartup::from_cli("demand").is_ok());
        assert!(ServiceStartup::from_cli("disabled").is_ok());
        assert!(ServiceStartup::from_cli("bogus").is_err());
    }

    #[test]
    fn service_startup_as_sc_value_matches_expectations() {
        assert_eq!(ServiceStartup::Auto.as_sc_value(), "auto");
        assert_eq!(ServiceStartup::Manual.as_sc_value(), "demand");
        assert_eq!(ServiceStartup::Disabled.as_sc_value(), "disabled");
    }

    #[test]
    fn quote_windows_executable_escapes_embedded_quotes() {
        assert_eq!(
            quote_windows_executable(r"C:\Apps\inferra.exe"),
            r#""C:\Apps\inferra.exe""#
        );
        assert_eq!(
            quote_windows_executable(r#"C:\Apps"inf"exe"#),
            r#""C:\Apps\"inf\"exe""#
        );
    }

    #[test]
    fn quote_windows_arg_quotes_only_when_spaces() {
        assert_eq!(quote_windows_arg("simple"), "simple");
        assert_eq!(quote_windows_arg("has space"), r#""has space""#);
    }

    #[test]
    fn is_missing_service_error_detects_known_patterns() {
        assert!(is_missing_service_error(
            "The specified service does not exist."
        ));
        assert!(is_missing_service_error("error 1060"));
        assert!(!is_missing_service_error("access denied"));
    }

    #[test]
    fn is_non_running_service_error_detects_known_patterns() {
        assert!(is_non_running_service_error(
            "The service has not been started."
        ));
        assert!(is_non_running_service_error("error 1062"));
        assert!(!is_non_running_service_error("already running"));
    }

    #[test]
    fn parse_state_value_extracts_text_after_numeric_code() {
        assert_eq!(
            parse_state_value("4  RUNNING".into()).as_deref(),
            Some("running")
        );
        assert_eq!(
            parse_state_value("RUNNING".into()).as_deref(),
            Some("running")
        );
    }

    #[test]
    fn service_log_path_uses_programdata() {
        let path = service_log_path();
        assert!(path.to_string_lossy().contains("Inferra"));
        assert!(path.to_string_lossy().contains("logs"));
        assert!(path.to_string_lossy().contains("serve.log"));
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::sync::OnceLock;
    use std::time::Duration;

    use anyhow::{Context, Result};
    use inferra_config::Paths;
    use windows_service::define_windows_service;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_dispatcher;

    use crate::{append_service_log, SERVICE_NAME};

    #[derive(Clone)]
    struct ServiceLaunchConfig {
        paths: Paths,
        ui_dist: PathBuf,
    }

    static SERVICE_CONFIG: OnceLock<ServiceLaunchConfig> = OnceLock::new();

    define_windows_service!(ffi_service_main, service_main);

    pub fn dispatch_service(paths: Paths, ui_dist: PathBuf) -> Result<()> {
        SERVICE_CONFIG
            .set(ServiceLaunchConfig { paths, ui_dist })
            .map_err(|_| anyhow::anyhow!("service configuration already initialized"))?;
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
            .context("start windows service dispatcher")?;
        Ok(())
    }

    pub fn service_main(_arguments: Vec<OsString>) {
        if let Err(error) = run_service() {
            append_service_log(&format!("service failed: {error:#}"));
        }
    }

    fn run_service() -> Result<()> {
        let config = SERVICE_CONFIG
            .get()
            .cloned()
            .context("service configuration missing before dispatch")?;
        append_service_log("service starting");

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let status_handle =
            service_control_handler::register(
                SERVICE_NAME,
                move |control_event| match control_event {
                    ServiceControl::Stop => {
                        let _ = stop_tx.send(());
                        ServiceControlHandlerResult::NoError
                    }
                    ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                    _ => ServiceControlHandlerResult::NotImplemented,
                },
            )
            .context("register service control handler")?;

        status_handle
            .set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Running,
                controls_accepted: ServiceControlAccept::STOP,
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            })
            .context("set running service status")?;

        let runtime = tokio::runtime::Runtime::new().context("create tokio runtime for service")?;
        let serve_result = runtime.block_on(async move {
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
            let watcher = std::thread::spawn(move || {
                let _ = stop_rx.recv();
                let _ = shutdown_tx.send(());
            });
            let result =
                inferra_api::serve_with_shutdown(config.paths, config.ui_dist, async move {
                    let _ = shutdown_rx.await;
                })
                .await;
            let _ = watcher.join();
            result
        });

        match &serve_result {
            Ok(()) => append_service_log(
                "http server finished without error (shutdown or listener ended unexpectedly)",
            ),
            Err(error) => append_service_log(&format!("http server error: {error:#}")),
        }

        status_handle
            .set_service_status(ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: ServiceState::Stopped,
                controls_accepted: ServiceControlAccept::empty(),
                exit_code: ServiceExitCode::Win32(0),
                checkpoint: 0,
                wait_hint: Duration::default(),
                process_id: None,
            })
            .context("set stopped service status")?;
        append_service_log("service stopped");
        serve_result
    }
}
