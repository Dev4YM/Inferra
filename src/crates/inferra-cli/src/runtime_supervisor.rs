use std::path::PathBuf;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::Command;
#[cfg(any(not(windows), test))]
use std::process::Output;

#[cfg(any(target_os = "linux", target_os = "macos"))]
use anyhow::Context;
use anyhow::Result;

#[cfg(any(target_os = "linux", test))]
pub const SYSTEMD_SERVICE_NAME: &str = "inferra.service";
#[cfg(any(target_os = "macos", test))]
pub const LAUNCHD_SERVICE_NAME: &str = "com.inferra.agent";

#[cfg(target_os = "linux")]
const SYSTEMD_UNIT_PATHS: [&str; 3] = [
    "/etc/systemd/system/inferra.service",
    "/usr/lib/systemd/system/inferra.service",
    "/lib/systemd/system/inferra.service",
];
#[cfg(target_os = "macos")]
const LAUNCHD_PLIST_PATH: &str = "/Library/LaunchDaemons/com.inferra.agent.plist";
#[cfg(target_os = "macos")]
const LAUNCHD_LOG_PATH: &str = "/usr/local/var/log/inferra.log";
#[cfg(target_os = "macos")]
const LAUNCHD_ERR_LOG_PATH: &str = "/usr/local/var/log/inferra.err.log";
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SERVICE_STATE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
#[cfg(any(target_os = "linux", target_os = "macos"))]
const SERVICE_CONTROL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupervisorStatus {
    pub kind: &'static str,
    pub supported: bool,
    pub service_name: String,
    pub installed: bool,
    pub state: Option<String>,
    pub startup: Option<String>,
    pub definition_path: Option<PathBuf>,
    pub binary_path: Option<String>,
    pub log_target: Option<String>,
}

impl SupervisorStatus {
    pub fn manager_label(&self) -> &'static str {
        match self.kind {
            "windows_service" => "Windows service",
            "systemd" => "systemd unit",
            "launchd" => "LaunchDaemon",
            _ => "service manager",
        }
    }

    pub fn normalized_state(&self) -> Option<&str> {
        self.state
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn is_running(&self) -> bool {
        match self.kind {
            "windows_service" => state_matches(self.normalized_state(), "running"),
            "systemd" => state_matches(self.normalized_state(), "active"),
            "launchd" => state_matches(self.normalized_state(), "running"),
            _ => false,
        }
    }

    pub fn is_start_pending(&self) -> bool {
        match self.kind {
            "windows_service" => state_matches(self.normalized_state(), "start_pending"),
            "systemd" => state_matches(self.normalized_state(), "activating"),
            "launchd" => state_matches(self.normalized_state(), "starting"),
            _ => false,
        }
    }

    #[cfg(any(not(windows), test))]
    pub fn is_stopped(&self) -> bool {
        match self.kind {
            "windows_service" => state_matches(self.normalized_state(), "stopped"),
            "systemd" => {
                state_matches(self.normalized_state(), "inactive")
                    || state_matches(self.normalized_state(), "failed")
            }
            "launchd" => {
                state_matches(self.normalized_state(), "not_loaded")
                    || state_matches(self.normalized_state(), "stopped")
            }
            _ => false,
        }
    }

    pub fn is_stop_pending(&self) -> bool {
        match self.kind {
            "windows_service" => state_matches(self.normalized_state(), "stop_pending"),
            "systemd" => state_matches(self.normalized_state(), "deactivating"),
            "launchd" => state_matches(self.normalized_state(), "stopping"),
            _ => false,
        }
    }
}

pub fn service_install_hint() -> &'static str {
    if cfg!(windows) {
        "deploy/windows/install-service.ps1 (Administrator)"
    } else if cfg!(target_os = "linux") {
        "sudo cp deploy/systemd/inferra.service /lib/systemd/system/inferra.service && sudo systemctl daemon-reload && sudo systemctl enable --now inferra"
    } else if cfg!(target_os = "macos") {
        "sudo ./deploy/macos/install.sh --full"
    } else {
        "inferra serve"
    }
}

pub fn runtime_unreachable_hint() -> String {
    if cfg!(windows) {
        "start `inferra runtime start` or install the Windows service if needed".to_string()
    } else if cfg!(target_os = "linux") {
        "start `inferra runtime start`, run `inferra serve`, or install/enable the inferra.service unit if needed".to_string()
    } else if cfg!(target_os = "macos") {
        "start `inferra runtime start`, run `inferra serve`, or install/load the com.inferra.agent LaunchDaemon if needed".to_string()
    } else {
        "start `inferra serve` if the local runtime is not already running".to_string()
    }
}

pub fn query_supervisor_status() -> Result<SupervisorStatus> {
    #[cfg(windows)]
    {
        query_windows_status()
    }
    #[cfg(target_os = "linux")]
    {
        query_systemd_status()
    }
    #[cfg(target_os = "macos")]
    {
        query_launchd_status()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        Ok(unsupported_status())
    }
}

pub fn start_supervisor() -> Result<SupervisorStatus> {
    #[cfg(windows)]
    {
        inferra_windows_service::start_service()?;
        query_windows_status()
    }
    #[cfg(target_os = "linux")]
    {
        systemd_start()
    }
    #[cfg(target_os = "macos")]
    {
        launchd_start()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("platform service control is not supported on this platform")
    }
}

pub fn stop_supervisor() -> Result<SupervisorStatus> {
    #[cfg(windows)]
    {
        inferra_windows_service::stop_service()?;
        query_windows_status()
    }
    #[cfg(target_os = "linux")]
    {
        systemd_stop()
    }
    #[cfg(target_os = "macos")]
    {
        launchd_stop()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("platform service control is not supported on this platform")
    }
}

pub fn restart_supervisor() -> Result<SupervisorStatus> {
    #[cfg(windows)]
    {
        inferra_windows_service::restart_service()?;
        query_windows_status()
    }
    #[cfg(target_os = "linux")]
    {
        systemd_restart()
    }
    #[cfg(target_os = "macos")]
    {
        launchd_restart()
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("platform service control is not supported on this platform")
    }
}

#[cfg(windows)]
fn query_windows_status() -> Result<SupervisorStatus> {
    let status = inferra_windows_service::query_service_status()?;
    Ok(SupervisorStatus {
        kind: "windows_service",
        supported: true,
        service_name: inferra_windows_service::SERVICE_NAME.to_string(),
        installed: status.installed,
        state: status.state,
        startup: status.startup,
        definition_path: None,
        binary_path: status.binary_path,
        log_target: Some(status.log_path.display().to_string()),
    })
}

#[cfg(target_os = "linux")]
fn query_systemd_status() -> Result<SupervisorStatus> {
    let supported = command_available("systemctl");
    let definition_path = systemd_definition_path();
    let installed = definition_path.is_some();
    let state = if supported && installed {
        Some(read_command_status(
            "systemctl",
            &["is-active", SYSTEMD_SERVICE_NAME],
        )?)
    } else {
        None
    };
    let startup = if supported && installed {
        Some(read_command_status(
            "systemctl",
            &["is-enabled", SYSTEMD_SERVICE_NAME],
        )?)
    } else {
        None
    };
    Ok(SupervisorStatus {
        kind: "systemd",
        supported,
        service_name: SYSTEMD_SERVICE_NAME.to_string(),
        installed,
        state,
        startup,
        definition_path,
        binary_path: None,
        log_target: Some(format!("journalctl -u {SYSTEMD_SERVICE_NAME}")),
    })
}

#[cfg(target_os = "linux")]
fn systemd_start() -> Result<SupervisorStatus> {
    let status = query_systemd_status()?;
    if !status.supported {
        anyhow::bail!(
            "systemctl is not available on this host; run `inferra serve` for a foreground runtime"
        );
    }
    if !status.installed {
        anyhow::bail!(
            "Inferra systemd unit is not installed. {}",
            service_install_hint()
        );
    }
    if status.is_running() || status.is_start_pending() {
        return wait_for_supervisor_state(query_systemd_status, &["active"], "systemd start");
    }
    run_checked_command("systemctl", &["start", SYSTEMD_SERVICE_NAME])?;
    wait_for_supervisor_state(query_systemd_status, &["active"], "systemd start")
}

#[cfg(target_os = "linux")]
fn systemd_stop() -> Result<SupervisorStatus> {
    let status = query_systemd_status()?;
    if !status.supported {
        anyhow::bail!("systemctl is not available on this host");
    }
    if !status.installed {
        anyhow::bail!("Inferra systemd unit is not installed");
    }
    if status.is_stopped() || status.is_stop_pending() {
        return wait_for_supervisor_state(
            query_systemd_status,
            &["inactive", "failed"],
            "systemd stop",
        );
    }
    run_checked_command("systemctl", &["stop", SYSTEMD_SERVICE_NAME])?;
    wait_for_supervisor_state(
        query_systemd_status,
        &["inactive", "failed"],
        "systemd stop",
    )
}

#[cfg(target_os = "linux")]
fn systemd_restart() -> Result<SupervisorStatus> {
    let status = query_systemd_status()?;
    if !status.supported {
        anyhow::bail!("systemctl is not available on this host");
    }
    if !status.installed {
        anyhow::bail!(
            "Inferra systemd unit is not installed. {}",
            service_install_hint()
        );
    }
    run_checked_command("systemctl", &["restart", SYSTEMD_SERVICE_NAME])?;
    wait_for_supervisor_state(query_systemd_status, &["active"], "systemd restart")
}

#[cfg(target_os = "macos")]
fn query_launchd_status() -> Result<SupervisorStatus> {
    let supported = command_available("launchctl");
    let definition_path = PathBuf::from(LAUNCHD_PLIST_PATH);
    let installed = definition_path.exists();
    let state = if supported && installed {
        match run_command("launchctl", &["print", &launchd_domain_target()]) {
            Ok(output) if output.status.success() => {
                parse_launchctl_state(&stdout_text(&output)).or_else(|| Some("loaded".into()))
            }
            Ok(output) => {
                let text = output_text(&output);
                if is_launchctl_not_loaded(&text) {
                    Some("not_loaded".into())
                } else {
                    Some(normalize_status_text(&text))
                }
            }
            Err(error) => {
                let text = error.to_string();
                if is_launchctl_not_loaded(&text) {
                    Some("not_loaded".into())
                } else {
                    return Err(error);
                }
            }
        }
    } else {
        None
    };
    Ok(SupervisorStatus {
        kind: "launchd",
        supported,
        service_name: LAUNCHD_SERVICE_NAME.to_string(),
        installed,
        state,
        startup: installed.then_some("run_at_load".to_string()),
        definition_path: installed.then_some(definition_path),
        binary_path: None,
        log_target: Some(format!(
            "{LAUNCHD_LOG_PATH} (stderr: {LAUNCHD_ERR_LOG_PATH})"
        )),
    })
}

#[cfg(target_os = "macos")]
fn launchd_start() -> Result<SupervisorStatus> {
    let status = query_launchd_status()?;
    if !status.supported {
        anyhow::bail!(
            "launchctl is not available on this host; run `inferra serve` for a foreground runtime"
        );
    }
    if !status.installed {
        anyhow::bail!(
            "Inferra LaunchDaemon is not installed. {}",
            service_install_hint()
        );
    }
    if status.is_running() || status.is_start_pending() {
        return wait_for_supervisor_state(query_launchd_status, &["running"], "launchd start");
    }
    if status.is_stopped() {
        run_checked_command("launchctl", &["bootstrap", "system", LAUNCHD_PLIST_PATH])?;
    } else if let Err(error) =
        run_checked_command("launchctl", &["kickstart", "-k", &launchd_domain_target()])
    {
        if !is_launchctl_not_loaded(&error.to_string()) {
            return Err(error);
        }
        run_checked_command("launchctl", &["bootstrap", "system", LAUNCHD_PLIST_PATH])?;
    }
    wait_for_supervisor_state(query_launchd_status, &["running"], "launchd start")
}

#[cfg(target_os = "macos")]
fn launchd_stop() -> Result<SupervisorStatus> {
    let status = query_launchd_status()?;
    if !status.supported {
        anyhow::bail!("launchctl is not available on this host");
    }
    if !status.installed {
        anyhow::bail!("Inferra LaunchDaemon is not installed");
    }
    if status.is_stopped() || status.is_stop_pending() {
        return wait_for_supervisor_state(query_launchd_status, &["not_loaded"], "launchd stop");
    }
    run_checked_command(
        "launchctl",
        &["bootout", "system", &launchd_domain_target()],
    )?;
    wait_for_supervisor_state(query_launchd_status, &["not_loaded"], "launchd stop")
}

#[cfg(target_os = "macos")]
fn launchd_restart() -> Result<SupervisorStatus> {
    let status = query_launchd_status()?;
    if !status.supported {
        anyhow::bail!("launchctl is not available on this host");
    }
    if !status.installed {
        anyhow::bail!(
            "Inferra LaunchDaemon is not installed. {}",
            service_install_hint()
        );
    }
    if let Err(error) =
        run_checked_command("launchctl", &["kickstart", "-k", &launchd_domain_target()])
    {
        if !is_launchctl_not_loaded(&error.to_string()) {
            return Err(error);
        }
        run_checked_command("launchctl", &["bootstrap", "system", LAUNCHD_PLIST_PATH])?;
    }
    wait_for_supervisor_state(query_launchd_status, &["running"], "launchd restart")
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn unsupported_status() -> SupervisorStatus {
    SupervisorStatus {
        kind: "unsupported",
        supported: false,
        service_name: "inferra".to_string(),
        installed: false,
        state: None,
        startup: None,
        definition_path: None,
        binary_path: None,
        log_target: None,
    }
}

#[cfg(target_os = "linux")]
fn systemd_definition_path() -> Option<PathBuf> {
    SYSTEMD_UNIT_PATHS
        .iter()
        .map(PathBuf::from)
        .find(|candidate| candidate.exists())
}

#[cfg(target_os = "macos")]
fn launchd_domain_target() -> String {
    format!("system/{LAUNCHD_SERVICE_NAME}")
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_checked_command(command: &str, args: &[&str]) -> Result<Output> {
    let output = run_command(command, args)?;
    if output.status.success() {
        return Ok(output);
    }
    anyhow::bail!(
        "{command} {} failed: {}",
        args.join(" "),
        output_text(&output)
    )
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_command(command: &str, args: &[&str]) -> Result<Output> {
    Command::new(command)
        .args(args)
        .output()
        .with_context(|| format!("run {command} {}", args.join(" ")))
}

#[cfg(target_os = "linux")]
fn read_command_status(command: &str, args: &[&str]) -> Result<String> {
    let output = run_command(command, args)?;
    let text = normalize_status_text(&output_text(&output));
    if !text.is_empty() {
        return Ok(text);
    }
    if output.status.success() {
        Ok("unknown".to_string())
    } else {
        anyhow::bail!("{command} {} returned no status", args.join(" "))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn command_available(command: &str) -> bool {
    Command::new(command).arg("--version").output().is_ok()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn wait_for_supervisor_state<F>(
    mut query: F,
    expected: &[&str],
    action: &str,
) -> Result<SupervisorStatus>
where
    F: FnMut() -> Result<SupervisorStatus>,
{
    let deadline = std::time::Instant::now() + SERVICE_CONTROL_TIMEOUT;
    let mut last_status = None;
    let mut last_error = None;

    loop {
        match query() {
            Ok(status) => {
                if !status.installed {
                    anyhow::bail!("{} is not installed", status.manager_label());
                }
                if expected
                    .iter()
                    .any(|candidate| state_matches(status.normalized_state(), candidate))
                {
                    return Ok(status);
                }
                last_status = Some(status);
            }
            Err(error) => last_error = Some(error),
        }

        if std::time::Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(SERVICE_STATE_POLL_INTERVAL);
    }

    if let Some(status) = last_status {
        anyhow::bail!(
            "{action} did not reach [{}] within {}s (last state: {})",
            expected.join(", "),
            SERVICE_CONTROL_TIMEOUT.as_secs(),
            status.normalized_state().unwrap_or("unknown")
        );
    }

    if let Some(error) = last_error {
        return Err(error.context(format!(
            "{action} did not reach [{}] within {}s",
            expected.join(", "),
            SERVICE_CONTROL_TIMEOUT.as_secs()
        )));
    }

    anyhow::bail!(
        "{action} did not reach [{}] within {}s",
        expected.join(", "),
        SERVICE_CONTROL_TIMEOUT.as_secs()
    )
}

fn state_matches(state: Option<&str>, expected: &str) -> bool {
    state.is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

#[cfg(any(not(windows), test))]
fn output_text(output: &Output) -> String {
    let stdout = stdout_text(output);
    let stderr = stderr_text(output);
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n{stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => "(no output)".to_string(),
    }
}

#[cfg(any(not(windows), test))]
fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[cfg(any(not(windows), test))]
fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

#[cfg(any(not(windows), test))]
fn normalize_status_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
}

#[cfg(target_os = "macos")]
fn parse_launchctl_state(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let (name, value) = trimmed.split_once('=')?;
        if name.trim() == "state" {
            return Some(normalize_status_text(value));
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn is_launchctl_not_loaded(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("could not find service")
        || lowered.contains("service is disabled")
        || lowered.contains("not loaded")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;
    #[cfg(windows)]
    use std::os::windows::process::ExitStatusExt;

    fn success_status() -> std::process::ExitStatus {
        #[cfg(unix)]
        {
            std::process::ExitStatus::from_raw(0)
        }
        #[cfg(windows)]
        {
            std::process::ExitStatus::from_raw(0)
        }
    }

    #[test]
    fn normalize_status_text_uses_first_non_empty_line() {
        assert_eq!(normalize_status_text("\n Active \nfailed"), "active");
        assert_eq!(normalize_status_text(""), "");
    }

    #[test]
    fn supervisor_status_helpers_match_manager_specific_states() {
        let linux = SupervisorStatus {
            kind: "systemd",
            supported: true,
            service_name: SYSTEMD_SERVICE_NAME.into(),
            installed: true,
            state: Some("active".into()),
            startup: Some("enabled".into()),
            definition_path: None,
            binary_path: None,
            log_target: None,
        };
        assert!(linux.is_running());
        assert!(!linux.is_stopped());

        let launchd = SupervisorStatus {
            kind: "launchd",
            supported: true,
            service_name: LAUNCHD_SERVICE_NAME.into(),
            installed: true,
            state: Some("not_loaded".into()),
            startup: Some("run_at_load".into()),
            definition_path: None,
            binary_path: None,
            log_target: None,
        };
        assert!(launchd.is_stopped());
        assert!(!launchd.is_running());
    }

    #[test]
    fn output_text_prefers_stdout_then_stderr() {
        let output = Output {
            status: success_status(),
            stdout: b"active\n".to_vec(),
            stderr: Vec::new(),
        };
        assert_eq!(output_text(&output), "active");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parse_launchctl_state_extracts_state_line() {
        let text = "system/com.inferra.agent = {\n\tstate = running\n}\n";
        assert_eq!(parse_launchctl_state(text).as_deref(), Some("running"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn launchctl_not_loaded_detection_matches_known_errors() {
        assert!(is_launchctl_not_loaded(
            "Could not find service \"system/com.inferra.agent\""
        ));
        assert!(!is_launchctl_not_loaded("permission denied"));
    }
}
