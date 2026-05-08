use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value as JsonValue;

fn unique_config_path() -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("inferra-cli-test-{stamp}.toml"))
}

#[test]
fn root_command_emits_landing_payload_in_json_mode() {
    let output = Command::new(env!("CARGO_BIN_EXE_inferra"))
        .args(["--json", "--config"])
        .arg(unique_config_path())
        .output()
        .expect("run inferra");

    assert!(output.status.success(), "stderr={}", String::from_utf8_lossy(&output.stderr));
    let payload: JsonValue =
        serde_json::from_slice(&output.stdout).expect("parse inferra root JSON output");
    assert_eq!(payload.get("name"), Some(&JsonValue::String("inferra".into())));
    assert_eq!(payload.get("version"), Some(&JsonValue::String(env!("CARGO_PKG_VERSION").into())));
    assert!(payload.get("runtime").is_some());
    assert!(payload.get("service").is_some());
}
