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

#[test]
fn source_type_for_returns_expected_mapping() {
    assert_eq!(source_type_for("docker"), "container");
    assert_eq!(source_type_for("journald"), "journald");
    assert_eq!(source_type_for("file"), "file");
    assert_eq!(source_type_for("process"), "process_snapshot");
    assert_eq!(source_type_for("app"), "app_http");
    assert_eq!(source_type_for("windows_eventlog"), "eventlog");
    assert_eq!(source_type_for("host_metrics"), "host");
    assert_eq!(source_type_for("windows_service"), "service");
    assert_eq!(source_type_for("linux_syslog"), "syslog");
    assert_eq!(source_type_for("kubernetes"), "kubernetes");
    assert_eq!(source_type_for("unknown"), "runtime");
}

#[test]
fn collector_specs_omits_disabled_collectors() {
    let config: TomlValue = r#"
[collectors.host_metrics]
enabled = false

[collectors.docker]
enabled = false

[collectors.kubernetes]
enabled = false
"#
    .parse()
    .expect("parse config");

    let specs = collector_specs(&config);
    assert!(!specs
        .iter()
        .any(|spec| matches!(spec, CollectorSpec::HostMetrics { .. })));
    assert!(!specs
        .iter()
        .any(|spec| matches!(spec, CollectorSpec::Docker { .. })));
    assert!(!specs
        .iter()
        .any(|spec| matches!(spec, CollectorSpec::Kubernetes { .. })));
}

#[test]
fn configured_collectors_parses_enabled_field() {
    let config: TomlValue = r#"
[collectors.host_metrics]
enabled = true

[collectors.docker]
enabled = false
"#
    .parse()
    .expect("parse config");

    let rows = configured_collectors(&config);
    let host = rows.iter().find(|r| r.collector_id == "host_metrics");
    assert!(host.is_some());
    assert_eq!(host.unwrap().status, "configured");

    let docker = rows.iter().find(|r| r.collector_id == "docker");
    assert!(docker.is_some());
    assert_eq!(docker.unwrap().status, "disabled");
}

#[test]
fn string_array_extracts_values_from_toml_array() {
    let config: TomlValue = r#"
values = ["a", "b", "c"]
"#
    .parse()
    .expect("parse");
    let result = string_array(config.get("values"));
    assert_eq!(result, vec!["a", "b", "c"]);
}

#[test]
fn string_array_returns_empty_for_none() {
    let result = string_array(None);
    assert!(result.is_empty());
}

#[test]
fn poll_interval_clamps_to_minimum_half_second() {
    let config: TomlValue = r#"
poll_interval_seconds = 0.01
"#
    .parse()
    .expect("parse");
    let table = config.as_table().expect("table");
    let interval = poll_interval(table, 10.0);
    assert!(interval.as_secs_f64() >= 0.5);
}

#[test]
fn severity_from_level_maps_known_levels() {
    assert_eq!(severity_from_level("error"), 3);
    assert_eq!(severity_from_level("warn"), 2);
    assert_eq!(severity_from_level("info"), 1);
    assert_eq!(severity_from_level("debug"), 0);
    assert_eq!(severity_from_level("unknown"), 1);
}

#[test]
fn threshold_state_detects_entered_and_recovered() {
    let mut state = ThresholdState::default();
    let sample = HostSample {
        hostname: "local".into(),
        cpu_percent: 90.0,
        memory_percent: 50.0,
        disk_percent: 40.0,
        disk_free_bytes: 1024,
    };
    let event = state.update_and_build_event(&sample, 85.0, 85.0, 90.0);
    assert!(event.is_some());
    let ev = event.unwrap();
    assert_eq!(ev.severity, 2); // WARN for entering
    assert!(ev.message.contains("cpu"));

    // Now recover
    let sample2 = HostSample {
        hostname: "local".into(),
        cpu_percent: 30.0,
        memory_percent: 50.0,
        disk_percent: 40.0,
        disk_free_bytes: 1024,
    };
    let event2 = state.update_and_build_event(&sample2, 85.0, 85.0, 90.0);
    assert!(event2.is_some());
    assert_eq!(event2.unwrap().severity, 1); // INFO for recovery
}

#[test]
fn threshold_state_returns_none_when_stable() {
    let mut state = ThresholdState::default();
    let sample = HostSample {
        hostname: "local".into(),
        cpu_percent: 50.0,
        memory_percent: 50.0,
        disk_percent: 50.0,
        disk_free_bytes: 1024,
    };
    // First sample sets baseline, no transition
    let event = state.update_and_build_event(&sample, 85.0, 85.0, 90.0);
    assert!(event.is_none());
    // Second identical sample, still no transition
    let event2 = state.update_and_build_event(&sample, 85.0, 85.0, 90.0);
    assert!(event2.is_none());
}

#[test]
fn process_cpu_normalization_returns_host_share() {
    assert_eq!(normalize_process_cpu_to_host_percent(100.0, 32), 3.125);
    assert_eq!(normalize_process_cpu_to_host_percent(3200.0, 32), 100.0);
}

#[test]
fn wevtutil_xml_parser_uses_event_record_id_cursor() {
    let xml = r#"<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='MSSQLSERVER'/><EventID Qualifiers='49152'>18456</EventID><Level>2</Level><TimeCreated SystemTime='2026-05-14T10:24:15.5994746Z'/><EventRecordID>158828</EventRecordID><Channel>Application</Channel></System><EventData><Data>NT Service\Example</Data><Data> Reason: Could not find a login matching the name provided.</Data><Data> [CLIENT: &lt;local machine&gt;]</Data></EventData></Event>"#;
    let records = parse_wevtutil_xml_events(xml, "Application");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].record_id, 158828);
    assert_eq!(records[0].provider, "MSSQLSERVER");
    assert_eq!(records[0].level, "error");
    assert!(records[0].message.contains("<local machine>"));
}

#[test]
fn docker_missing_binary_is_unavailable_not_active_error() {
    let reason = collector_unavailable_reason("docker", "run docker events: program not found")
        .expect("docker unavailable reason");
    assert!(reason.contains("Docker is enabled"));
}

#[tokio::test]
async fn collector_runtime_stop_is_idempotent() {
    let runtime = CollectorRuntime::default();
    runtime.stop().await.expect("first stop");
    runtime.stop().await.expect("second stop");
}

#[tokio::test]
async fn collector_runtime_active_errors_clear_after_success() {
    let runtime = CollectorRuntime::default();
    runtime
        .record_error("docker", "container", "docker daemon unavailable")
        .await;
    assert_eq!(runtime.total_errors().await, 1);
    assert_eq!(runtime.active_error_count().await, 1);

    runtime
        .bump_success("docker", "container", 1, 0, Some(now_iso()))
        .await;

    assert_eq!(runtime.total_errors().await, 1);
    assert_eq!(runtime.active_error_count().await, 0);
    let config: TomlValue = "[collectors.docker]\nenabled = true"
        .parse()
        .expect("config");
    let rows = runtime.collector_rows(&config).await;
    let docker = rows
        .iter()
        .find(|row| row.collector_id == "docker")
        .expect("docker row");
    assert_eq!(docker.status, "running");
    assert_eq!(
        docker.last_error.as_deref(),
        Some("docker daemon unavailable")
    );
    assert!(docker.last_error_at.is_some());
}

#[tokio::test]
async fn collector_runtime_unavailable_does_not_increment_active_errors() {
    let runtime = CollectorRuntime::default();
    runtime
        .record_unavailable("docker", "container", "docker unavailable")
        .await;
    assert_eq!(runtime.total_errors().await, 0);
    assert_eq!(runtime.active_error_count().await, 0);

    let config: TomlValue = "[collectors.docker]\nenabled = true"
        .parse()
        .expect("config");
    let rows = runtime.collector_rows(&config).await;
    let docker = rows
        .iter()
        .find(|row| row.collector_id == "docker")
        .expect("docker row");
    assert_eq!(docker.status, "unavailable");
    assert_eq!(docker.last_error.as_deref(), Some("docker unavailable"));
}

#[test]
fn next_event_id_generates_unique_ids() {
    let id_a = next_event_id("test");
    let id_b = next_event_id("test");
    assert_ne!(id_a, id_b);
    assert!(id_a.starts_with("test-"));
}

#[test]
fn semantic_fingerprint_is_deterministic() {
    let fp_a = semantic_fingerprint("src", "svc", "file", "msg");
    let fp_b = semantic_fingerprint("src", "svc", "file", "msg");
    assert_eq!(fp_a, fp_b);
    let fp_c = semantic_fingerprint("src", "svc", "file", "other");
    assert_ne!(fp_a, fp_c);
}
