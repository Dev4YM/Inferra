pub mod ai;
pub mod data;
pub mod system;

use anyhow::Result;
use serde_json::{json, Value as JsonValue};
use toml::Value as TomlValue;

use crate::cli::{Cli, Command};
use crate::context::AppContext;

pub async fn dispatch(cli: Cli) -> Result<()> {
    let ctx = AppContext::new(cli.config.clone(), cli.ui_dist.clone(), cli.json);
    match cli.command {
        None => system::show_landing(&ctx).await,
        Some(Command::Status) => system::show_status(&ctx).await,
        Some(Command::Serve) => system::run_serve_command(&ctx).await,
        Some(Command::Runtime { action }) => system::run_runtime_command(&ctx, action).await,
        Some(Command::Service {
            action,
            service_run,
        }) => system::run_service_command(&ctx, action, service_run).await,
        Some(Command::Setup {
            yes,
            skip_connection_test,
            data_dir,
        }) => system::run_setup(&ctx, yes, skip_connection_test, data_dir),
        Some(Command::InitDb) => system::run_init_db(&ctx),
        Some(Command::Collectors { action }) => system::run_collector_command(&ctx, action).await,
        Some(Command::Config { action }) => system::run_config_command(&ctx, action),
        Some(Command::Incidents { action }) => data::run_incident_command(&ctx, action),
        Some(Command::Events { action }) => data::run_event_command(&ctx, action),
        Some(Command::Services { action }) => data::run_service_data_command(&ctx, action),
        Some(Command::Workspace { action }) => data::run_workspace_command(&ctx, action).await,
        Some(Command::Ai { action }) => ai::run_ai_command(&ctx, action).await,
    }
}

pub fn emit_command_result(ctx: &AppContext, payload: &JsonValue, lines: &[String]) {
    if ctx.ui.is_json() {
        ctx.ui.print_json(payload);
    } else {
        for line in lines {
            println!("{line}");
        }
    }
}

pub fn emit_empty_list(ctx: &AppContext, key: &str, message: &str) {
    emit_command_result(ctx, &json!({ key: [] }), &[message.to_string()]);
}

pub fn toml_value_at<'a>(value: &'a TomlValue, key: &str) -> Option<&'a TomlValue> {
    let mut current = value;
    for part in key.split('.').filter(|part| !part.is_empty()) {
        current = current.get(part)?;
    }
    Some(current)
}

pub fn toml_to_json(value: TomlValue) -> JsonValue {
    serde_json::to_value(value).unwrap_or(JsonValue::Null)
}

pub fn toml_to_display(value: Option<&TomlValue>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<missing>".to_string())
}

pub fn patch_from_path(key: &str, value: JsonValue) -> JsonValue {
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

pub fn parse_cli_value(raw: &str) -> JsonValue {
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

pub fn preset_patch(name: &str) -> Result<JsonValue> {
    Ok(match name {
        "web-only" => json!({
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
        }),
        "windows-server" => json!({
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
        }),
        "linux-node" => json!({
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
        }),
        "docker-host" => json!({
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
        }),
        "kubernetes" => json!({
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
        }),
        other => anyhow::bail!("unknown preset: {other}"),
    })
}

pub fn url_encode(value: &str) -> String {
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
