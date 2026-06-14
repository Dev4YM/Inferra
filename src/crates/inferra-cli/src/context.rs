use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use inferra_config::{load_merged_config, server_listen, Paths};
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;
use tracing_subscriber::EnvFilter;

use crate::ui::TerminalUi;

#[derive(Clone, Debug)]
pub struct AppContext {
    pub config_override: Option<PathBuf>,
    pub ui_dist_override: Option<PathBuf>,
    pub ui: TerminalUi,
}

impl AppContext {
    pub fn new(
        config_override: Option<PathBuf>,
        ui_dist_override: Option<PathBuf>,
        json: bool,
    ) -> Self {
        Self {
            config_override,
            ui_dist_override,
            ui: TerminalUi::new(json),
        }
    }

    pub fn paths(&self) -> Result<Paths> {
        Paths::discover(self.config_override.clone())
    }

    pub fn resolve_ui_dist(&self) -> Result<PathBuf> {
        resolve_ui_dist(self.ui_dist_override.clone())
    }

    pub async fn api_request(
        &self,
        method: reqwest::Method,
        path: &str,
        payload: Option<JsonValue>,
    ) -> Result<JsonValue> {
        let paths = self.paths()?;
        let config = load_merged_config(&paths.config_path)?;
        let base_url = {
            let (host, port) = server_listen(&config)?;
            local_base_url(&host, port)
        };
        let url = format!("{base_url}{path}");
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
        let response = request.send().await.with_context(|| {
            format!(
                "request local API at {url}; start `inferra runtime start` or install the Windows service if needed"
            )
        })?;
        let status = response.status();
        let text = response
            .text()
            .await
            .context("read local API response body")?;
        if !status.is_success() {
            anyhow::bail!("local API request failed ({status}): {text}");
        }
        serde_json::from_str(&text).with_context(|| format!("parse local API JSON from {url}"))
    }
}

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .try_init();
}

pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn local_base_url(host: &str, port: u16) -> String {
    let client_host = client_host(host);
    if client_host.contains(':') && !client_host.starts_with('[') {
        format!("http://[{client_host}]:{port}")
    } else {
        format!("http://{client_host}:{port}")
    }
}

pub fn client_host(host: &str) -> String {
    match host.trim() {
        "" | "0.0.0.0" => "127.0.0.1".to_string(),
        "::" | "[::]" => "::1".to_string(),
        other => other.to_string(),
    }
}

pub fn resolve_ui_dist(override_path: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        return Ok(path);
    }
    for key in ["INFERRA_UI_DIST", "INFERRA_UI_PATH"] {
        if let Ok(path) = std::env::var(key) {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return Ok(PathBuf::from(trimmed));
            }
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        for candidate in ui_dist_candidates_from_executable(&exe) {
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }
    for candidate in ui_dist_candidates_from_working_dir()? {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "UI bundle not found. Install runtime-assets/ui_dist beside inferra, set INFERRA_UI_DIST, or pass --ui-dist."
    )
}

pub fn ui_dist_candidates_from_executable(exe: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(parent) = exe.parent() {
        candidates.push(parent.join("runtime-assets").join("ui_dist"));
        if let Some(grandparent) = parent.parent() {
            candidates.push(grandparent.join("runtime-assets").join("ui_dist"));
        }
    }
    candidates
}

fn ui_dist_candidates_from_working_dir() -> Result<Vec<PathBuf>> {
    let cwd = std::env::current_dir().context("resolve current directory")?;
    Ok(vec![
        cwd.join("runtime-assets").join("ui_dist"),
        cwd.join("src").join("web").join("ui_dist"),
        cwd.join("web").join("ui_dist"),
    ])
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{client_host, local_base_url, ui_dist_candidates_from_executable};

    #[test]
    fn wildcard_bind_host_maps_to_loopback_for_clients() {
        assert_eq!(client_host("0.0.0.0"), "127.0.0.1");
        assert_eq!(local_base_url("0.0.0.0", 7433), "http://127.0.0.1:7433");
    }

    #[test]
    fn ui_dist_candidates_cover_bin_and_sibling_layouts() {
        let candidates = ui_dist_candidates_from_executable(Path::new(
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
