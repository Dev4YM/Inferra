//! Optional observability export: forward new `events` rows as OTLP/HTTP JSON.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Context;
use base64::{engine::general_purpose::STANDARD, Engine};
use inferra_config::{
    observability_export_backfill_on_start, observability_export_batch_size,
    observability_export_bearer_token, observability_export_enabled,
    observability_export_interval_seconds, observability_export_max_retries,
    observability_export_retry_initial_seconds, observability_export_retry_max_seconds,
    observability_export_timeout_seconds, observability_export_url,
};
use inferra_contracts::{EventRow, SeverityValue};
use inferra_storage::{initialize_databases, EventsStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::AppState;

const CURSOR_FILE: &str = "observability_export_cursor.json";

pub static EXPORT_BUSY: AtomicBool = AtomicBool::new(false);
pub static EXPORT_BATCHES_SUCCESS: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_BATCHES_FAILED: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_EVENTS_FORWARDED: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_EVENTS_DROPPED: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_RETRIES_TOTAL: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_BATCHES_SPLIT: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_PARTIAL_REJECTIONS: AtomicU64 = AtomicU64::new(0);
pub static EXPORT_TICKS_SKIPPED_BUSY: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportCursor {
    timestamp: String,
    #[serde(default)]
    event_id: String,
}

#[derive(Debug, Clone)]
struct ExportHttpConfig {
    url: String,
    bearer: String,
    max_retries: u64,
    retry_initial: Duration,
    retry_max: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExportProgress {
    forwarded: u64,
    dropped: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OtlpPartialSuccess {
    rejected_log_records: u64,
    error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BatchSendOutcome {
    Delivered,
    Split(String),
    Abort(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BatchAttemptOutcome {
    Delivered,
    Retryable(String),
    Split(String),
    Abort(String),
}

pub async fn run(state: AppState) {
    tracing::info!("observability export sink task started");
    loop {
        let cfg = state.config.read().await.clone();
        if !observability_export_enabled(&cfg) || observability_export_url(&cfg).is_none() {
            tracing::info!("observability export disabled; export sink exiting");
            break;
        }
        let sleep_secs = observability_export_interval_seconds(&cfg);
        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
        if let Err(error) = one_tick(&state).await {
            tracing::warn!(error = %error, "observability export tick failed");
        }
    }
}

async fn one_tick(state: &AppState) -> anyhow::Result<()> {
    if EXPORT_BUSY
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        EXPORT_TICKS_SKIPPED_BUSY.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }
    struct BusyGuard;
    impl Drop for BusyGuard {
        fn drop(&mut self) {
            EXPORT_BUSY.store(false, Ordering::Release);
        }
    }
    let _busy = BusyGuard;

    let paths = state.paths.as_ref().clone();
    let cfg = state.config.read().await.clone();
    let batch_size = observability_export_batch_size(&cfg);
    let timeout = Duration::from_secs(observability_export_timeout_seconds(&cfg));
    let backfill = observability_export_backfill_on_start(&cfg);
    let http_cfg = ExportHttpConfig {
        url: observability_export_url(&cfg).context("export url")?,
        bearer: observability_export_bearer_token(&cfg),
        max_retries: observability_export_max_retries(&cfg),
        retry_initial: Duration::from_secs_f64(observability_export_retry_initial_seconds(&cfg)),
        retry_max: Duration::from_secs_f64(observability_export_retry_max_seconds(&cfg)),
    };

    initialize_databases(&paths.events_db, &paths.incidents_db)?;

    let cursor_path = paths.data_dir.join(CURSOR_FILE);
    if !cursor_path.exists() {
        let store = match EventsStore::open(&paths.events_db)? {
            Some(s) => Some(s),
            None => None,
        };
        let cursor = if backfill {
            ExportCursor {
                timestamp: "1970-01-01T00:00:00Z".into(),
                event_id: String::new(),
            }
        } else if let Some(store) = store.as_ref() {
            if let Some((ts, id)) = store.max_event_cursor()? {
                ExportCursor {
                    timestamp: ts,
                    event_id: id,
                }
            } else {
                ExportCursor {
                    timestamp: "1970-01-01T00:00:00Z".into(),
                    event_id: String::new(),
                }
            }
        } else {
            ExportCursor {
                timestamp: "1970-01-01T00:00:00Z".into(),
                event_id: String::new(),
            }
        };
        write_cursor(&cursor_path, &cursor)?;
    }

    let Some(store) = EventsStore::open(&paths.events_db)? else {
        return Ok(());
    };

    let cursor: ExportCursor = read_cursor(&cursor_path)?;
    let events = store.events_after_cursor(&cursor.timestamp, &cursor.event_id, batch_size)?;
    if events.is_empty() {
        return Ok(());
    }

    let client = reqwest::Client::builder().timeout(timeout).build()?;
    let progress = forward_events(&client, &http_cfg, &cursor_path, &events).await?;
    if progress.forwarded > 0 || progress.dropped > 0 {
        tracing::info!(
            forwarded = progress.forwarded,
            dropped = progress.dropped,
            "observability export processed batch"
        );
    }
    Ok(())
}

async fn forward_events(
    client: &reqwest::Client,
    http_cfg: &ExportHttpConfig,
    cursor_path: &Path,
    events: &[EventRow],
) -> anyhow::Result<ExportProgress> {
    let mut queue = VecDeque::new();
    queue.push_back((0usize, events.len()));

    let mut progress = ExportProgress {
        forwarded: 0,
        dropped: 0,
    };

    while let Some((start, end)) = queue.pop_front() {
        let batch = &events[start..end];
        match send_batch_with_retries(client, http_cfg, batch).await {
            BatchSendOutcome::Delivered => {
                write_cursor(
                    cursor_path,
                    &cursor_for_event(batch.last().expect("non-empty batch")),
                )?;
                EXPORT_EVENTS_FORWARDED.fetch_add(batch.len() as u64, Ordering::Relaxed);
                progress.forwarded += batch.len() as u64;
            }
            BatchSendOutcome::Split(reason) if batch.len() > 1 => {
                EXPORT_BATCHES_SPLIT.fetch_add(1, Ordering::Relaxed);
                let mid = start + (batch.len() / 2);
                queue.push_front((mid, end));
                queue.push_front((start, mid));
                tracing::warn!(
                    batch_size = batch.len(),
                    reason = %reason,
                    "observability export splitting batch after sink rejection"
                );
            }
            BatchSendOutcome::Split(reason) => {
                let event = batch.first().expect("single-event batch");
                write_cursor(cursor_path, &cursor_for_event(event))?;
                EXPORT_EVENTS_DROPPED.fetch_add(1, Ordering::Relaxed);
                progress.dropped += 1;
                tracing::warn!(
                    event_id = event.event_id.as_deref().unwrap_or(""),
                    reason = %reason,
                    "observability export dropping poison event after repeated sink rejection"
                );
            }
            BatchSendOutcome::Abort(reason) => anyhow::bail!(reason),
        }
    }

    Ok(progress)
}

async fn send_batch_with_retries(
    client: &reqwest::Client,
    http_cfg: &ExportHttpConfig,
    events: &[EventRow],
) -> BatchSendOutcome {
    let mut delay = http_cfg.retry_initial;
    for attempt in 0..=http_cfg.max_retries {
        match send_batch_once(client, http_cfg, events).await {
            BatchAttemptOutcome::Delivered => {
                EXPORT_BATCHES_SUCCESS.fetch_add(1, Ordering::Relaxed);
                return BatchSendOutcome::Delivered;
            }
            BatchAttemptOutcome::Split(reason) => {
                EXPORT_BATCHES_FAILED.fetch_add(1, Ordering::Relaxed);
                return BatchSendOutcome::Split(reason);
            }
            BatchAttemptOutcome::Abort(reason) => {
                EXPORT_BATCHES_FAILED.fetch_add(1, Ordering::Relaxed);
                return BatchSendOutcome::Abort(reason);
            }
            BatchAttemptOutcome::Retryable(reason) => {
                EXPORT_BATCHES_FAILED.fetch_add(1, Ordering::Relaxed);
                if attempt >= http_cfg.max_retries {
                    return BatchSendOutcome::Abort(format!(
                        "export retry exhausted after {} attempts: {reason}",
                        attempt + 1
                    ));
                }
                EXPORT_RETRIES_TOTAL.fetch_add(1, Ordering::Relaxed);
                tokio::time::sleep(delay).await;
                delay = next_backoff(delay, http_cfg.retry_max);
            }
        }
    }
    BatchSendOutcome::Abort("export retry loop terminated unexpectedly".into())
}

async fn send_batch_once(
    client: &reqwest::Client,
    http_cfg: &ExportHttpConfig,
    events: &[EventRow],
) -> BatchAttemptOutcome {
    let body = build_export_logs_request(events);
    let mut req = client
        .post(&http_cfg.url)
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .json(&body);
    if !http_cfg.bearer.is_empty() {
        req = req.header("authorization", format!("Bearer {}", http_cfg.bearer));
    }

    let response = match req.send().await {
        Ok(response) => response,
        Err(error) => return BatchAttemptOutcome::Retryable(format!("transport error: {error}")),
    };

    let status = response.status();
    let body = response.bytes().await.unwrap_or_default();
    let partial = otlp_partial_success_from_body(&body);
    if let Some(partial) = partial.as_ref() {
        if partial.rejected_log_records > 0 {
            EXPORT_PARTIAL_REJECTIONS.fetch_add(partial.rejected_log_records, Ordering::Relaxed);
        }
    }

    if status.is_success() {
        if let Some(partial) = partial.filter(|item| item.rejected_log_records > 0) {
            return BatchAttemptOutcome::Split(partial_success_reason(&partial));
        }
        return BatchAttemptOutcome::Delivered;
    }

    let detail = response_detail(status, &body, partial.as_ref());
    if status.is_server_error()
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return BatchAttemptOutcome::Retryable(detail);
    }
    if matches!(
        status,
        reqwest::StatusCode::BAD_REQUEST
            | reqwest::StatusCode::PAYLOAD_TOO_LARGE
            | reqwest::StatusCode::UNPROCESSABLE_ENTITY
    ) {
        return BatchAttemptOutcome::Split(detail);
    }
    BatchAttemptOutcome::Abort(detail)
}

fn next_backoff(current: Duration, max_backoff: Duration) -> Duration {
    let doubled = current.checked_mul(2).unwrap_or(max_backoff);
    if doubled > max_backoff {
        max_backoff
    } else {
        doubled
    }
}

fn otlp_partial_success_from_body(body: &[u8]) -> Option<OtlpPartialSuccess> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let partial = value.get("partialSuccess")?;
    let rejected_log_records = partial
        .get("rejectedLogRecords")
        .and_then(value_u64)
        .unwrap_or(0);
    let error_message = partial
        .get("errorMessage")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    if rejected_log_records == 0 && error_message.is_none() {
        return None;
    }
    Some(OtlpPartialSuccess {
        rejected_log_records,
        error_message,
    })
}

fn value_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok())),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn partial_success_reason(partial: &OtlpPartialSuccess) -> String {
    match partial.error_message.as_deref() {
        Some(message) => format!(
            "sink partialSuccess rejected {} log records: {message}",
            partial.rejected_log_records
        ),
        None => format!(
            "sink partialSuccess rejected {} log records",
            partial.rejected_log_records
        ),
    }
}

fn response_detail(
    status: reqwest::StatusCode,
    body: &[u8],
    partial: Option<&OtlpPartialSuccess>,
) -> String {
    if let Some(partial) = partial {
        return format!("export HTTP {status}: {}", partial_success_reason(partial));
    }
    let text = String::from_utf8_lossy(body).trim().to_string();
    if text.is_empty() {
        format!("export HTTP {status}")
    } else {
        format!("export HTTP {status}: {text}")
    }
}

fn cursor_for_event(event: &EventRow) -> ExportCursor {
    ExportCursor {
        timestamp: event
            .timestamp
            .clone()
            .unwrap_or_else(|| "1970-01-01T00:00:00Z".into()),
        event_id: event.event_id.clone().unwrap_or_default(),
    }
}

fn read_cursor(path: &Path) -> anyhow::Result<ExportCursor> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let c: ExportCursor = serde_json::from_str(&raw).context("parse export cursor")?;
    Ok(c)
}

fn write_cursor(path: &Path, cursor: &ExportCursor) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let tmp = path.with_extension("json.tmp");
    let raw = serde_json::to_string_pretty(cursor).context("serialize export cursor")?;
    std::fs::write(&tmp, raw).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename cursor {}", path.display()))?;
    Ok(())
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    let h: String = hex
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    if h.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(h.len() / 2);
    for i in (0..h.len()).step_by(2) {
        let byte = u8::from_str_radix(&h[i..i + 2], 16).ok()?;
        out.push(byte);
    }
    Some(out)
}

fn trace_id_otlp_field(trace_hex: Option<&str>) -> Option<String> {
    let h = trace_hex?.trim();
    if h.is_empty() {
        return None;
    }
    let bytes = hex_to_bytes(h)?;
    (bytes.len() == 16).then(|| STANDARD.encode(&bytes))
}

fn span_id_otlp_field(span_hex: Option<&str>) -> Option<String> {
    let h = span_hex?.trim();
    if h.is_empty() {
        return None;
    }
    let bytes = hex_to_bytes(h)?;
    (bytes.len() == 8).then(|| STANDARD.encode(&bytes))
}

fn rfc3339_to_unix_nanos(ts: &str) -> String {
    OffsetDateTime::parse(ts, &Rfc3339)
        .map(|t| t.unix_timestamp_nanos().max(0) as u64)
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "0".into())
}

fn severity_number(sev: Option<&SeverityValue>) -> i32 {
    let level = match sev {
        Some(SeverityValue::Level(n)) => *n,
        Some(SeverityValue::Label(s)) => match s.to_ascii_lowercase().as_str() {
            "critical" | "fatal" => 4,
            "error" => 3,
            "warn" | "warning" => 2,
            "debug" | "trace" => 0,
            _ => 1,
        },
        None => 1,
    };
    match level {
        0 => 5,
        1 => 9,
        2 => 13,
        3 => 17,
        _ => 21,
    }
}

fn log_record_json(ev: &EventRow) -> Value {
    let message = ev
        .message
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("(no message)");
    let ts = ev
        .timestamp
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(rfc3339_to_unix_nanos)
        .unwrap_or_else(|| "0".into());

    let mut lr = json!({
        "timeUnixNano": ts,
        "severityNumber": severity_number(ev.severity.as_ref()),
        "severityText": ev.severity_text.clone().unwrap_or_default(),
        "body": { "stringValue": message },
        "attributes": [
            {"key": "inferra.event_id", "value": {"stringValue": ev.event_id.clone().unwrap_or_default()}}
        ]
    });
    if let Some(obj) = lr.as_object_mut() {
        if let Some(t) = trace_id_otlp_field(ev.trace_id.as_deref()) {
            obj.insert("traceId".into(), json!(t));
        }
        if let Some(s) = span_id_otlp_field(ev.span_id.as_deref()) {
            obj.insert("spanId".into(), json!(s));
        }
        if let Some(env) = ev
            .deployment_environment
            .as_deref()
            .filter(|e| !e.is_empty())
        {
            if let Some(arr) = obj.get_mut("attributes").and_then(|a| a.as_array_mut()) {
                arr.push(json!({"key": "deployment.environment", "value": {"stringValue": env}}));
            }
        }
    }
    lr
}

fn build_export_logs_request(events: &[EventRow]) -> Value {
    let resource_logs: Vec<Value> = events
        .iter()
        .map(|ev| {
            let service = ev
                .service_id
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("unknown-service");
            json!({
                "resource": {
                    "attributes": [{"key": "service.name", "value": {"stringValue": service}}]
                },
                "scopeLogs": [{
                    "scope": { "name": "inferra" },
                    "logRecords": [log_record_json(ev)]
                }]
            })
        })
        .collect();
    json!({ "resourceLogs": resource_logs })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::{Json, Router};
    use inferra_collectors::CollectorRuntime;
    use inferra_config::Paths;
    use inferra_storage::{initialize_databases, NewEventRecord};
    use tokio::sync::{Mutex as AsyncMutex, RwLock};
    use toml::Value as TomlValue;

    use crate::ScannerCache;

    #[test]
    fn export_payload_contains_log_records() {
        let ev = EventRow {
            event_id: Some("e-1".into()),
            timestamp: Some("2026-05-14T12:00:00Z".into()),
            severity: Some(SeverityValue::Level(3)),
            service_id: Some("svc".into()),
            message: Some("hello".into()),
            summary: None,
            source_ref: None,
            tags: None,
            trace_id: Some("aabbccdd0011223344556677889900aa".into()),
            span_id: Some("0102030405060708".into()),
            signal_kind: Some("log".into()),
            deployment_environment: Some("prod".into()),
            severity_text: Some("ERROR".into()),
        };
        let body = build_export_logs_request(std::slice::from_ref(&ev));
        let rls = body["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0].clone();
        assert_eq!(rls["body"]["stringValue"], json!("hello"));
        assert!(rls.get("traceId").is_some());
        assert!(rls.get("spanId").is_some());
    }

    #[test]
    fn parses_otlp_partial_success_body() {
        let body = br#"{
            "partialSuccess": {
                "rejectedLogRecords": 2,
                "errorMessage": "bad attributes"
            }
        }"#;
        let partial = otlp_partial_success_from_body(body).expect("partial success");
        assert_eq!(partial.rejected_log_records, 2);
        assert_eq!(partial.error_message.as_deref(), Some("bad attributes"));
    }

    #[tokio::test]
    async fn export_tick_retries_transient_sink_failures() {
        let _guard = export_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (url, state) = start_retry_then_success_sink().await;
        let (app_state, root) = export_test_state("retry-once", &url);
        insert_export_events(
            &app_state.paths.events_db,
            &[minimal_event(
                "evt-1",
                "2026-05-14T12:00:00Z",
                "hello retry",
            )],
        );

        one_tick(&app_state).await.expect("export tick");

        assert_eq!(state.attempts.lock().expect("attempts").len(), 2);
        let cursor = read_cursor(&app_state.paths.data_dir.join(CURSOR_FILE)).expect("cursor");
        assert_eq!(cursor.event_id, "evt-1");
        assert_eq!(
            state.accepted_batches.lock().expect("batches").as_slice(),
            &[vec!["evt-1".to_string()]]
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn export_tick_splits_partial_success_and_drops_only_poison_event() {
        let _guard = export_test_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (url, state) = start_partial_success_sink("poison").await;
        let (app_state, root) = export_test_state("split-poison", &url);
        insert_export_events(
            &app_state.paths.events_db,
            &[
                minimal_event("evt-1", "2026-05-14T12:00:00Z", "ok-one"),
                minimal_event("evt-2", "2026-05-14T12:00:01Z", "poison"),
                minimal_event("evt-3", "2026-05-14T12:00:02Z", "ok-two"),
            ],
        );

        one_tick(&app_state).await.expect("export tick");

        let batches = state.accepted_batches.lock().expect("batches").clone();
        assert!(batches.iter().any(|ids| ids == &vec!["evt-1".to_string()]));
        assert!(batches.iter().any(|ids| ids == &vec!["evt-2".to_string()]));
        assert!(batches.iter().any(|ids| ids == &vec!["evt-3".to_string()]));

        let cursor = read_cursor(&app_state.paths.data_dir.join(CURSOR_FILE)).expect("cursor");
        assert_eq!(cursor.event_id, "evt-3");

        let store = EventsStore::open(&app_state.paths.events_db)
            .expect("open store")
            .expect("events db");
        assert_eq!(store.count_events().expect("count events"), 3);
        assert!(store
            .events_after_cursor(&cursor.timestamp, &cursor.event_id, 10)
            .expect("remaining events")
            .is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[derive(Clone, Default)]
    struct MockSinkState {
        attempts: Arc<Mutex<Vec<Vec<String>>>>,
        accepted_batches: Arc<Mutex<Vec<Vec<String>>>>,
    }

    fn export_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    async fn start_retry_then_success_sink() -> (String, MockSinkState) {
        let state = MockSinkState::default();
        let app = Router::new()
            .route(
                "/v1/logs",
                post(
                    |State(state): State<MockSinkState>, Json(payload): Json<Value>| async move {
                        let ids = event_ids_from_payload(&payload);
                        let mut attempts = state.attempts.lock().expect("attempts");
                        attempts.push(ids.clone());
                        if attempts.len() == 1 {
                            return (
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(json!({ "error": "try again" })),
                            );
                        }
                        drop(attempts);
                        state.accepted_batches.lock().expect("accepted").push(ids);
                        (StatusCode::OK, Json(json!({})))
                    },
                ),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind sink");
        let addr = listener.local_addr().expect("sink addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve sink");
        });
        (format!("http://{addr}/v1/logs"), state)
    }

    async fn start_partial_success_sink(poison_message: &'static str) -> (String, MockSinkState) {
        let state = MockSinkState::default();
        let app = Router::new()
            .route(
                "/v1/logs",
                post(
                    move |State(state): State<MockSinkState>, Json(payload): Json<Value>| async move {
                        let ids = event_ids_from_payload(&payload);
                        state.attempts.lock().expect("attempts").push(ids.clone());
                        state
                            .accepted_batches
                            .lock()
                            .expect("accepted")
                            .push(ids);
                        let has_poison = messages_from_payload(&payload)
                            .iter()
                            .any(|message| message == poison_message);
                        if has_poison {
                            (
                                StatusCode::OK,
                                Json(json!({
                                    "partialSuccess": {
                                        "rejectedLogRecords": 1,
                                        "errorMessage": "poison event"
                                    }
                                })),
                            )
                        } else {
                            (StatusCode::OK, Json(json!({})))
                        }
                    },
                ),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind sink");
        let addr = listener.local_addr().expect("sink addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve sink");
        });
        (format!("http://{addr}/v1/logs"), state)
    }

    fn event_ids_from_payload(payload: &Value) -> Vec<String> {
        payload
            .get("resourceLogs")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .flat_map(|resource_log| {
                resource_log
                    .get("scopeLogs")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
            })
            .flat_map(|scope_log| {
                scope_log
                    .get("logRecords")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
            })
            .filter_map(|record| {
                record
                    .get("attributes")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                    .find_map(|attr| {
                        (attr.get("key").and_then(|v| v.as_str()) == Some("inferra.event_id"))
                            .then(|| {
                                attr.get("value")
                                    .and_then(|v| v.get("stringValue"))
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string)
                            })
                            .flatten()
                    })
            })
            .collect()
    }

    fn messages_from_payload(payload: &Value) -> Vec<String> {
        payload
            .get("resourceLogs")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .flat_map(|resource_log| {
                resource_log
                    .get("scopeLogs")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
            })
            .flat_map(|scope_log| {
                scope_log
                    .get("logRecords")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
            })
            .filter_map(|record| {
                record
                    .get("body")
                    .and_then(|v| v.get("stringValue"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect()
    }

    fn export_test_state(name: &str, url: &str) -> (AppState, PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inferra-export-sink-{name}-{unique}"));
        let data_dir = root.join("data");
        let ui_dist = root.join("ui");
        std::fs::create_dir_all(&data_dir).expect("create data dir");
        std::fs::create_dir_all(&ui_dist).expect("create ui dir");

        let events_db = data_dir.join("events.db");
        let incidents_db = data_dir.join("incidents.db");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");

        let config: TomlValue = format!(
            r#"
[observability.export]
enabled = true
url = "{url}"
interval_seconds = 1
batch_size = 10
timeout_seconds = 5
max_retries = 2
retry_initial_seconds = 0.01
retry_max_seconds = 0.01
backfill_on_start = true
bearer_token = ""
"#
        )
        .parse()
        .expect("parse config");

        let state = AppState {
            paths: Arc::new(Paths {
                config_path: root.join("inferra.toml"),
                data_dir,
                events_db,
                incidents_db,
            }),
            config: Arc::new(RwLock::new(config)),
            collectors: CollectorRuntime::default(),
            scanner_cache: Arc::new(RwLock::new(ScannerCache::default())),
            workspace_refresh: Arc::new(AsyncMutex::new(())),
            ui_dist,
            rate_limits: Arc::new(crate::middleware::RateLimitState::new(30.0, 15.0)),
        };
        (state, root)
    }

    fn minimal_event(event_id: &str, timestamp: &str, message: &str) -> NewEventRecord {
        let mut event = NewEventRecord::minimal(
            event_id, timestamp, "svc", 3, message, "app_http", timestamp,
        );
        event.severity_text = Some("ERROR".into());
        event.signal_kind = "log".into();
        event
    }

    fn insert_export_events(events_db: &Path, events: &[NewEventRecord]) {
        let mut store = EventsStore::open(events_db)
            .expect("open events store")
            .expect("events store exists");
        store.insert_batch(events).expect("insert events");
    }
}
