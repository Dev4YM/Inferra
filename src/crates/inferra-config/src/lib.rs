//! Load and merge `inferra.toml` with packaged defaults from [`defaults.toml`](../../../config/defaults.toml).

use anyhow::{Context, Result};
use inferra_contracts::ExperiencePayload;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;

const DEFAULTS_TOML: &str = include_str!("../../../config/defaults.toml");

/// Resolved filesystem paths for config and databases.
#[derive(Debug, Clone)]
pub struct Paths {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
    pub events_db: PathBuf,
    pub incidents_db: PathBuf,
}

impl Paths {
    pub fn discover(config_override: Option<PathBuf>) -> Result<Self> {
        let config_path = resolve_config_path(config_override)?;

        let merged = load_merged_config(&config_path)?;
        let data_dir = extract_data_dir(&merged)?;
        let data_dir = if data_dir.is_absolute() {
            data_dir
        } else {
            config_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(data_dir)
        };

        let events_name = merged
            .get("storage")
            .and_then(|s| s.get("events_db"))
            .and_then(|v| v.as_str())
            .unwrap_or("events.db");
        let incidents_name = merged
            .get("storage")
            .and_then(|s| s.get("incidents_db"))
            .and_then(|v| v.as_str())
            .unwrap_or("incidents.db");

        Ok(Self {
            config_path,
            data_dir: data_dir.clone(),
            events_db: data_dir.join(events_name),
            incidents_db: data_dir.join(incidents_name),
        })
    }
}

fn resolve_config_path(config_override: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = config_override {
        return Ok(p);
    }
    for key in [
        "INFERRA_CONFIG",
        "INFERRA_CONFIG_PATH",
        "INFERA_CONFIG_PATH",
    ] {
        if let Ok(p) = std::env::var(key) {
            let trimmed = p.trim();
            if !trimmed.is_empty() {
                return Ok(PathBuf::from(trimmed));
            }
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        let candidates = config_path_candidates_from_executable(&exe);
        if executable_looks_installed(&exe) {
            if let Some(candidate) = candidates.first() {
                return Ok(candidate.clone());
            }
        }
        for candidate in candidates {
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    if cfg!(windows) {
        let candidate = windows_programdata_config_path();
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    let cwd_config = PathBuf::from("inferra.toml");
    if cwd_config.exists() {
        return Ok(cwd_config);
    }
    if cfg!(windows) {
        return Ok(windows_programdata_config_path());
    }
    Ok(cwd_config)
}

pub fn config_path_candidates_from_executable(exe: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(parent) = exe.parent() {
        candidates.push(parent.join("inferra.toml"));
        candidates.push(parent.join("config").join("inferra.toml"));
        if let Some(grandparent) = parent.parent() {
            candidates.push(grandparent.join("inferra.toml"));
            candidates.push(grandparent.join("etc").join("inferra.toml"));
            candidates.push(grandparent.join("etc").join("inferra").join("inferra.toml"));
        }
    }
    candidates
}

fn executable_looks_installed(exe: &Path) -> bool {
    let Some(parent) = exe.parent() else {
        return false;
    };
    parent.join("runtime-assets").exists()
        || parent
            .parent()
            .map(|grandparent| grandparent.join("runtime-assets").exists())
            .unwrap_or(false)
}

fn windows_programdata_config_path() -> PathBuf {
    let pd = std::env::var("PROGRAMDATA").unwrap_or_else(|_| "C:\\ProgramData".into());
    PathBuf::from(pd).join("Inferra").join("inferra.toml")
}

fn extract_data_dir(merged: &TomlValue) -> Result<PathBuf> {
    let dir = merged
        .get("storage")
        .and_then(|s| s.get("data_dir"))
        .and_then(|v| v.as_str())
        .context("storage.data_dir missing after merge")?;
    Ok(PathBuf::from(dir))
}

/// Resolved absolute `storage.data_dir` for validation during PUT /api/config.
pub fn resolve_data_dir(config_path: &Path, merged: &TomlValue) -> Result<PathBuf> {
    let mut data_dir = extract_data_dir(merged)?;
    if data_dir.is_relative() {
        data_dir = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(data_dir);
    }
    Ok(data_dir)
}

fn parse_toml_str(raw: &str) -> Result<TomlValue> {
    raw.parse::<TomlValue>()
        .map_err(|e| anyhow::anyhow!("toml parse: {e}"))
}

fn merge_toml(base: TomlValue, overlay: TomlValue) -> TomlValue {
    match (base, overlay) {
        (TomlValue::Table(mut b), TomlValue::Table(o)) => {
            for (k, v) in o {
                let merged = match (b.remove(&k), v) {
                    (Some(TomlValue::Table(bt)), TomlValue::Table(ot)) => {
                        merge_toml(TomlValue::Table(bt), TomlValue::Table(ot))
                    }
                    (_, new_v) => new_v,
                };
                b.insert(k, merged);
            }
            TomlValue::Table(b)
        }
        (_, overlay) => overlay,
    }
}

/// Merge defaults.toml + optional user file.
/// Minimum 1 hour. Used for log/event query windows and aligns with `[storage].retention_hours`.
pub fn storage_retention_hours(config: &TomlValue) -> i64 {
    config
        .get("storage")
        .and_then(|s| s.get("retention_hours"))
        .and_then(|v| v.as_integer())
        .unwrap_or(72)
        .max(1)
}

/// Keys under `attributes` in ingested JSON copied into `event_attributes` (see observability plan).
pub fn observability_indexed_attribute_keys(config: &TomlValue) -> Vec<String> {
    let mut out = Vec::new();
    let Some(obs) = config.get("observability").and_then(|v| v.as_table()) else {
        return out;
    };
    let Some(idx) = obs.get("indexed_attributes").and_then(|v| v.as_table()) else {
        return out;
    };
    let Some(keys) = idx.get("keys").and_then(|v| v.as_array()) else {
        return out;
    };
    const MAX_KEYS: usize = 64;
    for item in keys.iter().filter_map(TomlValue::as_str) {
        let trimmed = item.trim();
        if trimmed.is_empty() || out.len() >= MAX_KEYS {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

/// When true, `GET /api/v2/logs` `q=` uses SQLite FTS5 (`events_fts`); otherwise `q=` uses `LIKE` on `message`.
pub fn observability_logs_fts_enabled(config: &TomlValue) -> bool {
    config
        .get("observability")
        .and_then(|o| o.get("fts"))
        .and_then(|f| f.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// When true, `POST /v1/logs` accepts OTLP/HTTP log export batches over JSON or protobuf.
pub fn observability_otlp_logs_enabled(config: &TomlValue) -> bool {
    config
        .get("observability")
        .and_then(|o| o.get("otlp"))
        .and_then(|o| o.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Max log records processed per OTLP request (additional records are counted as rejected).
pub fn observability_otlp_max_logs_per_request(config: &TomlValue) -> usize {
    let n = config
        .get("observability")
        .and_then(|o| o.get("otlp"))
        .and_then(|o| o.get("max_logs_per_request"))
        .and_then(|v| v.as_integer())
        .unwrap_or(500);
    (n.max(1) as usize).min(10_000)
}

/// Max raw body size for `POST /v1/logs` across OTLP JSON and protobuf payloads.
pub fn observability_otlp_max_payload_bytes(config: &TomlValue) -> usize {
    let n = config
        .get("observability")
        .and_then(|o| o.get("otlp"))
        .and_then(|o| o.get("max_payload_bytes"))
        .and_then(|v| v.as_integer())
        .unwrap_or(4_194_304);
    (n.max(1024) as usize).min(32 * 1024 * 1024)
}

/// When true, a background task forwards new `events` rows as OTLP/HTTP JSON to `[observability.export].url`.
pub fn observability_export_enabled(config: &TomlValue) -> bool {
    config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// OTLP logs HTTP endpoint (must be `http://` or `https://`).
pub fn observability_export_url(config: &TomlValue) -> Option<String> {
    let url = config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("url"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url.to_string())
    } else {
        None
    }
}

pub fn observability_export_interval_seconds(config: &TomlValue) -> u64 {
    let n = config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("interval_seconds"))
        .and_then(|v| v.as_integer())
        .unwrap_or(30);
    (n.max(5) as u64).min(3600)
}

pub fn observability_export_batch_size(config: &TomlValue) -> usize {
    let n = config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("batch_size"))
        .and_then(|v| v.as_integer())
        .unwrap_or(100);
    (n.max(1) as usize).min(2000)
}

pub fn observability_export_timeout_seconds(config: &TomlValue) -> u64 {
    let n = config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("timeout_seconds"))
        .and_then(|v| v.as_integer())
        .unwrap_or(15);
    (n.max(1) as u64).min(120)
}

pub fn observability_export_max_retries(config: &TomlValue) -> u64 {
    let n = config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("max_retries"))
        .and_then(|v| v.as_integer())
        .unwrap_or(3);
    (n.max(0) as u64).min(10)
}

pub fn observability_export_retry_initial_seconds(config: &TomlValue) -> f64 {
    let n = config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("retry_initial_seconds"))
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .unwrap_or(1.0);
    n.clamp(0.1, 300.0)
}

pub fn observability_export_retry_max_seconds(config: &TomlValue) -> f64 {
    let initial = observability_export_retry_initial_seconds(config);
    let n = config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("retry_max_seconds"))
        .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
        .unwrap_or(30.0);
    n.clamp(initial, 3600.0)
}

/// When false (default), the first run seeds the export cursor to the newest row so historical rows are not re-sent.
pub fn observability_export_backfill_on_start(config: &TomlValue) -> bool {
    config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("backfill_on_start"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Optional `Authorization: Bearer …` for the export HTTP client (plain string; prefer env-based secrets in production).
pub fn observability_export_bearer_token(config: &TomlValue) -> String {
    config
        .get("observability")
        .and_then(|o| o.get("export"))
        .and_then(|e| e.get("bearer_token"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Merge defaults.toml + optional user file.
pub fn load_merged_config(config_path: &Path) -> Result<TomlValue> {
    let defaults = parse_toml_str(DEFAULTS_TOML).context("defaults.toml")?;
    if !config_path.exists() {
        return Ok(defaults);
    }
    let user_raw = std::fs::read_to_string(config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let user =
        parse_toml_str(&user_raw).with_context(|| format!("parse {}", config_path.display()))?;
    Ok(merge_toml(defaults, user))
}

pub fn config_to_json(config: &TomlValue) -> JsonValue {
    toml_to_json(config)
}

fn toml_to_json(v: &TomlValue) -> JsonValue {
    match v {
        TomlValue::String(s) => JsonValue::String(s.clone()),
        TomlValue::Integer(i) => JsonValue::Number((*i).into()),
        TomlValue::Float(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        TomlValue::Boolean(b) => JsonValue::Bool(*b),
        TomlValue::Datetime(d) => JsonValue::String(d.to_string()),
        TomlValue::Array(a) => JsonValue::Array(a.iter().map(toml_to_json).collect()),
        TomlValue::Table(t) => {
            let mut map = JsonMap::new();
            for (k, val) in t {
                map.insert(k.clone(), toml_to_json(val));
            }
            JsonValue::Object(map)
        }
    }
}

fn json_to_toml(v: &JsonValue) -> Result<TomlValue> {
    match v {
        JsonValue::Null => {
            anyhow::bail!(
                "null config values are not supported; omit the key to keep the current value"
            )
        }
        JsonValue::Bool(b) => Ok(TomlValue::Boolean(*b)),
        JsonValue::Number(n) => Ok(if let Some(i) = n.as_i64() {
            TomlValue::Integer(i)
        } else if let Some(f) = n.as_f64() {
            TomlValue::Float(f)
        } else {
            TomlValue::String(n.to_string())
        }),
        JsonValue::String(s) => Ok(TomlValue::String(s.clone())),
        JsonValue::Array(a) => Ok(TomlValue::Array(
            a.iter().map(json_to_toml).collect::<Result<Vec<_>, _>>()?,
        )),
        JsonValue::Object(o) => {
            let mut t = toml::map::Map::new();
            for (k, val) in o {
                t.insert(k.clone(), json_to_toml(val)?);
            }
            Ok(TomlValue::Table(t))
        }
    }
}

/// Deep-merge JSON `patch` into `base` (objects recurse; scalars and arrays replace).
pub fn merge_json(base: &mut JsonValue, patch: &JsonValue) {
    match (base, patch) {
        (JsonValue::Object(am), JsonValue::Object(pm)) => {
            for (k, v) in pm {
                match am.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => {
                        merge_json(existing, v);
                    }
                    _ => {
                        am.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (slot, patch) => *slot = patch.clone(),
    }
}

/// Apply UI config payload: expects `{ "config": { ... } }` merged onto current Toml, then re-base onto defaults.
pub fn apply_config_put(base_merged: TomlValue, body: &JsonValue) -> Result<TomlValue> {
    let patch = body.get("config").cloned().unwrap_or_else(|| body.clone());
    let defaults = parse_toml_str(DEFAULTS_TOML).context("defaults")?;
    let mut base_json = config_to_json(&base_merged);
    merge_json(&mut base_json, &patch);
    let merged_tables = json_to_toml(&base_json)?;
    Ok(merge_toml(defaults, merged_tables))
}

pub fn write_config(path: &Path, config: &TomlValue) -> Result<()> {
    let raw = toml::to_string_pretty(config).context("serialize config to TOML")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create_dir {}", parent.display()))?;
    }
    std::fs::write(path, raw).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn server_listen(config: &TomlValue) -> Result<(String, u16)> {
    let host = config
        .get("server")
        .and_then(|s| s.get("host"))
        .and_then(|v| v.as_str())
        .unwrap_or("127.0.0.1")
        .trim()
        .to_string();
    if host.is_empty() {
        anyhow::bail!("server.host cannot be empty");
    }
    let port = config
        .get("server")
        .and_then(|s| s.get("port"))
        .and_then(|v| v.as_integer())
        .unwrap_or(7433);
    if !(1..=u16::MAX as i64).contains(&port) {
        anyhow::bail!("server.port must be between 1 and {}", u16::MAX);
    }
    Ok((host, port as u16))
}

pub fn experience_from_config(config: &TomlValue) -> ExperiencePayload {
    let mode = config
        .get("experience")
        .and_then(|e| e.get("mode"))
        .and_then(|v| v.as_str())
        .unwrap_or("operator")
        .to_string();
    let ai_role = config
        .get("experience")
        .and_then(|e| e.get("ai_role"))
        .and_then(|v| v.as_str())
        .unwrap_or("investigator")
        .to_string();
    let suggest = config
        .get("experience")
        .and_then(|e| e.get("suggest_safe_actions"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let execute = config
        .get("experience")
        .and_then(|e| e.get("execute_actions"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let raw_evidence = config
        .get("experience")
        .and_then(|e| e.get("show_raw_evidence_by_default"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    ExperiencePayload {
        mode,
        ai_role,
        suggest_safe_actions: suggest,
        execute_actions: execute,
        show_raw_evidence_by_default: raw_evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_config_put, config_path_candidates_from_executable, load_merged_config,
        resolve_config_path, server_listen, write_config, TomlValue,
    };
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolve_config_path_prefers_explicit_override() {
        let override_path = PathBuf::from("custom/inferra.toml");
        let resolved = resolve_config_path(Some(override_path.clone())).expect("resolve override");
        assert_eq!(resolved, override_path);
    }

    #[test]
    fn resolve_config_path_accepts_inferra_config_env() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let env_path = PathBuf::from("env/inferra.toml");
        unsafe {
            std::env::set_var("INFERRA_CONFIG", &env_path);
            std::env::remove_var("INFERRA_CONFIG_PATH");
            std::env::remove_var("INFERA_CONFIG_PATH");
        }
        let resolved = resolve_config_path(None).expect("resolve env path");
        unsafe {
            std::env::remove_var("INFERRA_CONFIG");
        }
        assert_eq!(resolved, env_path);
    }

    #[test]
    fn resolve_config_path_accepts_inferra_config_path_env() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let env_path = PathBuf::from("env-path/inferra.toml");
        unsafe {
            std::env::remove_var("INFERRA_CONFIG");
            std::env::set_var("INFERRA_CONFIG_PATH", &env_path);
            std::env::remove_var("INFERA_CONFIG_PATH");
        }
        let resolved = resolve_config_path(None).expect("resolve env config path");
        unsafe {
            std::env::remove_var("INFERRA_CONFIG_PATH");
        }
        assert_eq!(resolved, env_path);
    }

    #[test]
    fn executable_config_candidates_cover_installed_layouts() {
        let candidates = config_path_candidates_from_executable(&PathBuf::from(
            r"C:\Program Files\Inferra\bin\inferra.exe",
        ));
        assert_eq!(
            candidates[0],
            PathBuf::from(r"C:\Program Files\Inferra\bin\inferra.toml")
        );
        assert!(candidates.contains(&PathBuf::from(
            r"C:\Program Files\Inferra\etc\inferra\inferra.toml"
        )));
    }

    #[test]
    fn write_config_produces_valid_toml_document() {
        let root = std::env::temp_dir().join(format!(
            "inferra-config-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        let config_path = root.join("inferra.toml");
        let config = load_merged_config(&config_path).expect("load merged defaults");
        write_config(&config_path, &config).expect("write config");
        let raw = std::fs::read_to_string(&config_path).expect("read config");
        assert!(raw.contains("[server]"));
        assert!(toml::from_str::<TomlValue>(&raw).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_config_put_rejects_null_values() {
        let base: TomlValue = r#"
[server]
host = "127.0.0.1"
port = 7433
"#
        .parse()
        .expect("parse base");
        let error = apply_config_put(base, &json!({ "config": { "server": { "host": null } } }))
            .expect_err("null values should be rejected");
        assert!(error
            .to_string()
            .contains("null config values are not supported"));
    }

    #[test]
    fn server_listen_rejects_invalid_port_numbers() {
        let config: TomlValue = r#"
[server]
host = "127.0.0.1"
port = 70000
"#
        .parse()
        .expect("parse config");
        let error = server_listen(&config).expect_err("invalid port should fail");
        assert!(error
            .to_string()
            .contains("server.port must be between 1 and"));
    }
}
