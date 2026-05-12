//! Unit tests for inferra-collectors (split from `lib.rs` for maintainability).

use super::*;
use inferra_storage::initialize_databases;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_db_paths(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("inferra-collectors-{name}-{unique}"));
    let events = root.join("events.db");
    let incidents = root.join("incidents.db");
    (root, events, incidents)
}

#[test]
fn wildcard_match_supports_star_and_question_mark() {
    assert!(wildcard_match("*.log", "app.log"));
    assert!(wildcard_match("app-?.txt", "app-1.txt"));
    assert!(!wildcard_match("app-?.txt", "app-10.txt"));
}

#[test]
fn collector_specs_include_file_and_app_standalone_when_enabled() {
    let config: TomlValue = r#"
[collectors.file]
enabled = true
paths = ["./logs/app.log"]
poll_interval_seconds = 1.0
start_at_end = false

[collectors.app]
enabled = true
enable_main_api = true
enable_standalone = true
listen = "127.0.0.1:9999"
mount_path = "/custom-ingest"
shared_token = "token"
max_payload_bytes = 2048
"#
    .parse()
    .expect("parse config");

    let specs = collector_specs(&config);
    assert!(specs
        .iter()
        .any(|spec| matches!(spec, CollectorSpec::File { .. })));
    assert!(specs
        .iter()
        .any(|spec| matches!(spec, CollectorSpec::AppIngest)));
    assert!(specs.iter().any(|spec| matches!(
        spec,
        CollectorSpec::AppStandalone {
            listen,
            mount_path,
            max_payload_bytes,
            ..
        } if listen == "127.0.0.1:9999" && mount_path == "/custom-ingest" && *max_payload_bytes == 2048
    )));
}

#[test]
fn normalized_mount_path_adds_leading_slash() {
    assert_eq!(normalized_mount_path("api/ingest"), "/api/ingest");
    assert_eq!(normalized_mount_path("/api/ingest"), "/api/ingest");
    assert_eq!(normalized_mount_path(""), "/api/ingest");
}

#[tokio::test]
async fn ingest_app_event_reports_governance_suppression() {
    let (root, events_db, incidents_db) = temp_db_paths("app-ingest");
    initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
    let runtime = CollectorRuntime::default();
    let config: TomlValue = r#"
[deduplication]
enabled = false

[noise_filter]
enabled = true
blocklist_enabled = true
allowlist_enabled = false
always_keep_severity = "ERROR"

[[noise_filter.blocklist]]
pattern = "health check passed"
severity_max = "INFO"
reason = "routine health signal"
"#
    .parse()
    .expect("parse config");

    let result = runtime
        .ingest_app_event(
            &events_db,
            &incidents_db,
            &config,
            &serde_json::json!({
                "service": "api",
                "level": "info",
                "message": "health check passed",
            }),
        )
        .await
        .expect("ingest app event");

    assert!(!result.accepted);
    assert_eq!(result.suppressed_noise, 1);
    let store = EventsStore::open(&events_db)
        .expect("open events store")
        .expect("events store exists");
    assert_eq!(store.count_events().expect("count events"), 0);

    let _ = fs::remove_dir_all(root);
}
