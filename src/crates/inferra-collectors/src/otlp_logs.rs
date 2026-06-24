//! OTLP/HTTP JSON log export → `NewEventRecord` (OpenTelemetry Phase 7 MVP).

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use opentelemetry_proto::tonic::{
    collector::logs::v1::ExportLogsServiceRequest,
    common::v1::{
        any_value::Value as ProtoAnyValue, AnyValue as ProtoAnyValueMessage, InstrumentationScope,
        KeyValue,
    },
    logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
    resource::v1::Resource,
};
use prost::Message;
use serde_json::{json, Map, Value};
use time::{Duration, OffsetDateTime};

use inferra_storage::NewEventRecord;

use super::{next_event_id, semantic_fingerprint};

#[derive(Debug, Clone, Default)]
pub struct OtlpLogsBuildResult {
    pub records: Vec<NewEventRecord>,
    pub rejected_log_records: u64,
}

/// Parse `ExportLogsServiceRequest` JSON into governed ingest rows.
pub fn build_new_event_records_from_otlp_logs_json(
    payload: &Value,
    max_records: usize,
) -> Result<OtlpLogsBuildResult> {
    let _ = payload
        .as_object()
        .context("OTLP body must be a JSON object")?;
    let resource_logs = payload
        .get("resourceLogs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = OtlpLogsBuildResult::default();
    let lim = max_records.max(1);

    for rl in resource_logs {
        let Some(rl) = rl.as_object() else {
            out.rejected_log_records += 1;
            continue;
        };
        let res_attrs = rl
            .get("resource")
            .and_then(|r| r.get("attributes"))
            .and_then(|a| a.as_array())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let base_flat = flatten_kv_attributes(res_attrs);

        let scope_logs = rl
            .get("scopeLogs")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        for sl in scope_logs {
            let Some(sl) = sl.as_object() else {
                out.rejected_log_records += 1;
                continue;
            };
            let scope = sl.get("scope").cloned().unwrap_or(Value::Null);
            let log_records = sl
                .get("logRecords")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            for lr in log_records {
                if out.records.len() >= lim {
                    out.rejected_log_records += 1;
                    continue;
                }
                let Some(rec) = log_record_to_new_event(&lr, &base_flat, &scope) else {
                    out.rejected_log_records += 1;
                    continue;
                };
                out.records.push(rec);
            }
        }
    }

    Ok(out)
}

/// Decode OTLP protobuf into the same JSON shape used by the existing OTLP/HTTP JSON ingest path.
pub fn normalize_otlp_logs_protobuf_request(payload: &[u8]) -> Result<Value> {
    let request = ExportLogsServiceRequest::decode(payload)
        .context("decode OTLP ExportLogsServiceRequest protobuf")?;
    Ok(export_logs_request_to_json(&request))
}

fn log_record_to_new_event(
    lr: &Value,
    resource_flat: &Map<String, Value>,
    scope: &Value,
) -> Option<NewEventRecord> {
    let obj = lr.as_object()?;
    let nanos = obj
        .get("timeUnixNano")
        .and_then(json_u64)
        .or_else(|| obj.get("observedTimeUnixNano").and_then(json_u64));
    let timestamp = nanos
        .map(iso_from_unix_nanos)
        .unwrap_or_else(super::now_iso);
    let collected_at = super::now_iso();

    let sev_num = obj.get("severityNumber").and_then(json_i64);
    let sev_text = obj
        .get("severityText")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let log_attrs = obj
        .get("attributes")
        .and_then(|a| a.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut merged = resource_flat.clone();
    merge_kv_into_map(&mut merged, log_attrs);

    let service_id = merged
        .get("service.name")
        .and_then(json_as_string)
        .unwrap_or_else(|| "unknown-service".into());

    let deployment_environment = merged
        .get("deployment.environment")
        .and_then(json_as_string)
        .or_else(|| {
            merged
                .get("deployment_environment")
                .and_then(json_as_string)
        });

    let trace_id = obj
        .get("traceId")
        .and_then(|v| v.as_str())
        .and_then(parse_trace_id);
    let span_id = obj
        .get("spanId")
        .and_then(|v| v.as_str())
        .and_then(parse_span_id);

    let body = obj
        .get("body")
        .map(any_value_to_json)
        .unwrap_or(Value::Null);
    let message = body_to_log_message(&body);

    let severity = severity_from_otel(sev_num, sev_text);

    let event_id = next_event_id("otlp");
    let fingerprint = semantic_fingerprint(
        "otlp",
        &service_id,
        "otlp_json",
        &message,
        trace_id.as_deref(),
    );

    let structured = json!({
        "attributes": merged,
        "otlp": {
            "scope": scope,
            "logRecord": lr,
        }
    });

    Some(NewEventRecord {
        event_id,
        timestamp: timestamp.clone(),
        service_id,
        severity,
        message,
        source_type: "otlp_json".into(),
        source_id: "otlp-http".into(),
        tags: Vec::new(),
        fingerprint,
        host_id: "local".into(),
        event_type: 0,
        timestamp_source: "otlp".into(),
        collected_at,
        quality: Some("normalized".into()),
        structured_data: Some(structured),
        raw_offset: None,
        trace_id,
        span_id,
        signal_kind: "log".into(),
        deployment_environment,
        severity_text: sev_text.map(str::to_string),
    })
}

fn json_u64(v: &Value) -> Option<u64> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_i64().and_then(|i| u64::try_from(i).ok())),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn json_i64(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_u64().map(|u| u as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn json_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn json_as_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn iso_from_unix_nanos(nanos: u64) -> String {
    let secs = (nanos / 1_000_000_000) as i64;
    let nsec = (nanos % 1_000_000_000) as i32;
    let dt = OffsetDateTime::UNIX_EPOCH + Duration::new(secs, nsec);
    dt.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| super::now_iso())
}

fn parse_trace_id(raw: &str) -> Option<String> {
    let compact: String = raw
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact.len() == 32 {
        return Some(compact);
    }
    let bytes = STANDARD.decode(raw.trim()).ok()?;
    (bytes.len() == 16).then(|| bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn parse_span_id(raw: &str) -> Option<String> {
    let compact: String = raw
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact.len() == 16 {
        return Some(compact);
    }
    let bytes = STANDARD.decode(raw.trim()).ok()?;
    (bytes.len() == 8).then(|| bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn severity_from_otel(severity_number: Option<i64>, severity_text: Option<&str>) -> i64 {
    if let Some(text) = severity_text {
        return super::severity_from_level(text);
    }
    let n = severity_number.unwrap_or(0);
    match n {
        1..=4 => 0,
        5..=8 => 0,
        9..=12 => 1,
        13..=16 => 2,
        17..=20 => 3,
        _ if n >= 21 => 4,
        _ => 1,
    }
}

fn flatten_kv_attributes(attrs: &[Value]) -> Map<String, Value> {
    let mut m = Map::new();
    merge_kv_into_map(&mut m, attrs);
    m
}

fn merge_kv_into_map(target: &mut Map<String, Value>, attrs: &[Value]) {
    for item in attrs {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some(key) = obj.get("key").and_then(|k| k.as_str()).map(str::to_string) else {
            continue;
        };
        if let Some(val) = obj.get("value") {
            if let Some(scalar) = any_value_to_scalar_json(val) {
                target.insert(key, scalar);
            }
        }
    }
}

fn any_value_to_json(v: &Value) -> Value {
    any_value_to_scalar_json(v).unwrap_or_else(|| v.clone())
}

/// OTLP AnyValue → JSON scalar when possible (for `attributes` map + indexing).
fn any_value_to_scalar_json(v: &Value) -> Option<Value> {
    if let Some(s) = v.get("stringValue") {
        return match s {
            Value::String(t) => Some(Value::String(t.clone())),
            other => Some(other.clone()),
        };
    }
    if let Some(b) = v.get("boolValue").and_then(|x| x.as_bool()) {
        return Some(Value::Bool(b));
    }
    if let Some(n) = v.get("intValue").and_then(json_i64) {
        return Some(Value::Number(n.into()));
    }
    if let Some(n) = v.get("doubleValue").and_then(json_f64) {
        return serde_json::Number::from_f64(n).map(Value::Number);
    }
    if let Some(arr) = v
        .get("arrayValue")
        .and_then(|a| a.get("values"))
        .and_then(|a| a.as_array())
    {
        let mapped: Vec<Value> = arr.iter().map(any_value_to_json).collect();
        return Some(Value::Array(mapped));
    }
    if let Some(entries) = v
        .get("kvlistValue")
        .and_then(|k| k.get("values"))
        .and_then(|a| a.as_array())
    {
        let mut inner = Map::new();
        merge_kv_into_map(&mut inner, entries);
        return Some(Value::Object(inner));
    }
    if v.get("bytesValue").and_then(|b| b.as_str()).is_some() {
        return v
            .get("bytesValue")
            .and_then(|b| b.as_str())
            .map(|s| Value::String(format!("bytes:{s}")));
    }
    None
}

fn body_to_log_message(body: &Value) -> String {
    match body {
        Value::String(s) => s.clone(),
        Value::Object(_) => body.to_string(),
        Value::Array(_) => body.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "(empty log body)".into(),
    }
}

fn export_logs_request_to_json(request: &ExportLogsServiceRequest) -> Value {
    json!({
        "resourceLogs": request
            .resource_logs
            .iter()
            .map(resource_logs_to_json)
            .collect::<Vec<_>>()
    })
}

fn resource_logs_to_json(resource_logs: &ResourceLogs) -> Value {
    let mut out = Map::new();
    if let Some(resource) = resource_logs.resource.as_ref() {
        out.insert("resource".into(), resource_to_json(resource));
    }
    out.insert(
        "scopeLogs".into(),
        Value::Array(
            resource_logs
                .scope_logs
                .iter()
                .map(scope_logs_to_json)
                .collect(),
        ),
    );
    if !resource_logs.schema_url.is_empty() {
        out.insert(
            "schemaUrl".into(),
            Value::String(resource_logs.schema_url.clone()),
        );
    }
    Value::Object(out)
}

fn resource_to_json(resource: &Resource) -> Value {
    let mut out = Map::new();
    out.insert(
        "attributes".into(),
        Value::Array(key_values_to_json(&resource.attributes)),
    );
    if resource.dropped_attributes_count > 0 {
        out.insert(
            "droppedAttributesCount".into(),
            Value::Number(resource.dropped_attributes_count.into()),
        );
    }
    Value::Object(out)
}

fn scope_logs_to_json(scope_logs: &ScopeLogs) -> Value {
    let mut out = Map::new();
    out.insert(
        "scope".into(),
        scope_logs
            .scope
            .as_ref()
            .map(instrumentation_scope_to_json)
            .unwrap_or(Value::Null),
    );
    out.insert(
        "logRecords".into(),
        Value::Array(
            scope_logs
                .log_records
                .iter()
                .map(proto_log_record_to_json)
                .collect(),
        ),
    );
    if !scope_logs.schema_url.is_empty() {
        out.insert(
            "schemaUrl".into(),
            Value::String(scope_logs.schema_url.clone()),
        );
    }
    Value::Object(out)
}

fn instrumentation_scope_to_json(scope: &InstrumentationScope) -> Value {
    let mut out = Map::new();
    if !scope.name.is_empty() {
        out.insert("name".into(), Value::String(scope.name.clone()));
    }
    if !scope.version.is_empty() {
        out.insert("version".into(), Value::String(scope.version.clone()));
    }
    if !scope.attributes.is_empty() {
        out.insert(
            "attributes".into(),
            Value::Array(key_values_to_json(&scope.attributes)),
        );
    }
    if scope.dropped_attributes_count > 0 {
        out.insert(
            "droppedAttributesCount".into(),
            Value::Number(scope.dropped_attributes_count.into()),
        );
    }
    Value::Object(out)
}

fn proto_log_record_to_json(log_record: &LogRecord) -> Value {
    let mut out = Map::new();
    if log_record.time_unix_nano > 0 {
        out.insert(
            "timeUnixNano".into(),
            Value::String(log_record.time_unix_nano.to_string()),
        );
    }
    if log_record.observed_time_unix_nano > 0 {
        out.insert(
            "observedTimeUnixNano".into(),
            Value::String(log_record.observed_time_unix_nano.to_string()),
        );
    }
    if log_record.severity_number != 0 {
        out.insert(
            "severityNumber".into(),
            Value::Number((log_record.severity_number as i64).into()),
        );
    }
    if !log_record.severity_text.is_empty() {
        out.insert(
            "severityText".into(),
            Value::String(log_record.severity_text.clone()),
        );
    }
    if let Some(body) = log_record.body.as_ref() {
        out.insert("body".into(), proto_any_value_to_otlp_json(body));
    }
    if !log_record.attributes.is_empty() {
        out.insert(
            "attributes".into(),
            Value::Array(key_values_to_json(&log_record.attributes)),
        );
    }
    if log_record.dropped_attributes_count > 0 {
        out.insert(
            "droppedAttributesCount".into(),
            Value::Number(log_record.dropped_attributes_count.into()),
        );
    }
    if log_record.flags != 0 {
        out.insert("flags".into(), Value::Number(log_record.flags.into()));
    }
    if !log_record.trace_id.is_empty() {
        out.insert(
            "traceId".into(),
            Value::String(bytes_to_hex(&log_record.trace_id)),
        );
    }
    if !log_record.span_id.is_empty() {
        out.insert(
            "spanId".into(),
            Value::String(bytes_to_hex(&log_record.span_id)),
        );
    }
    Value::Object(out)
}

fn key_values_to_json(values: &[KeyValue]) -> Vec<Value> {
    values
        .iter()
        .map(|kv| {
            let mut out = Map::new();
            out.insert("key".into(), Value::String(kv.key.clone()));
            out.insert(
                "value".into(),
                kv.value
                    .as_ref()
                    .map(proto_any_value_to_otlp_json)
                    .unwrap_or(Value::Null),
            );
            Value::Object(out)
        })
        .collect()
}

fn proto_any_value_to_otlp_json(value: &ProtoAnyValueMessage) -> Value {
    let Some(inner) = value.value.as_ref() else {
        return Value::Null;
    };
    let mut out = Map::new();
    match inner {
        ProtoAnyValue::StringValue(s) => {
            out.insert("stringValue".into(), Value::String(s.clone()));
        }
        ProtoAnyValue::BoolValue(b) => {
            out.insert("boolValue".into(), Value::Bool(*b));
        }
        ProtoAnyValue::IntValue(i) => {
            out.insert("intValue".into(), Value::Number((*i).into()));
        }
        ProtoAnyValue::DoubleValue(d) => {
            out.insert("doubleValue".into(), json_f64_value(*d));
        }
        ProtoAnyValue::ArrayValue(arr) => {
            out.insert(
                "arrayValue".into(),
                json!({
                    "values": arr.values.iter().map(proto_any_value_to_otlp_json).collect::<Vec<_>>()
                }),
            );
        }
        ProtoAnyValue::KvlistValue(kvlist) => {
            out.insert(
                "kvlistValue".into(),
                json!({
                    "values": key_values_to_json(&kvlist.values)
                }),
            );
        }
        ProtoAnyValue::BytesValue(bytes) => {
            out.insert("bytesValue".into(), Value::String(STANDARD.encode(bytes)));
        }
        ProtoAnyValue::StringValueStrindex(_) => return Value::Null,
    }
    Value::Object(out)
}

fn json_f64_value(value: f64) -> Value {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(value.to_string()))
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::{
        collector::logs::v1::ExportLogsServiceRequest,
        common::v1::{
            any_value::Value as ProtoAnyValue, AnyValue as ProtoAnyValueMessage,
            InstrumentationScope, KeyValue,
        },
        logs::v1::{LogRecord, ResourceLogs, ScopeLogs},
        resource::v1::Resource,
    };
    use prost::Message;

    #[test]
    fn otlp_json_maps_service_and_trace() {
        let tid = "aabbccdd0011223344556677889900aa";
        let sid = "0102030405060708";
        let payload = json!({
            "resourceLogs": [{
                "resource": {
                    "attributes": [
                        {"key": "service.name", "value": {"stringValue": "checkout"}},
                        {"key": "deployment.environment", "value": {"stringValue": "staging"}}
                    ]
                },
                "scopeLogs": [{
                    "scope": {"name": "test"},
                    "logRecords": [{
                        "timeUnixNano": "1715689200000000000",
                        "severityNumber": 17,
                        "severityText": "ERROR",
                        "body": {"stringValue": "payment failed"},
                        "traceId": tid,
                        "spanId": sid,
                        "attributes": [
                            {"key": "http.route", "value": {"stringValue": "/pay"}},
                            {"key": "http.status_code", "value": {"intValue": 502}}
                        ]
                    }]
                }]
            }]
        });
        let out = build_new_event_records_from_otlp_logs_json(&payload, 10).expect("parse");
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.rejected_log_records, 0);
        let r = &out.records[0];
        assert_eq!(r.service_id, "checkout");
        assert_eq!(r.message, "payment failed");
        assert_eq!(r.trace_id.as_deref(), Some(tid));
        assert_eq!(r.span_id.as_deref(), Some(sid));
        assert_eq!(r.severity, 3);
        assert_eq!(r.deployment_environment.as_deref(), Some("staging"));
    }

    #[test]
    fn otlp_protobuf_reuses_json_mapping_semantics() {
        let trace_id = [
            0xaa, 0xbb, 0xcc, 0xdd, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
            0x00, 0xaa,
        ];
        let span_id = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let request = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                resource: Some(Resource {
                    attributes: vec![
                        KeyValue {
                            key: "service.name".into(),
                            value: Some(ProtoAnyValueMessage {
                                value: Some(ProtoAnyValue::StringValue("checkout".into())),
                            }),
                            key_strindex: 0,
                        },
                        KeyValue {
                            key: "deployment.environment".into(),
                            value: Some(ProtoAnyValueMessage {
                                value: Some(ProtoAnyValue::StringValue("staging".into())),
                            }),
                            key_strindex: 0,
                        },
                    ],
                    dropped_attributes_count: 0,
                    entity_refs: Vec::new(),
                }),
                scope_logs: vec![ScopeLogs {
                    scope: Some(InstrumentationScope {
                        name: "protobuf-test".into(),
                        version: "1.0.0".into(),
                        attributes: Vec::new(),
                        dropped_attributes_count: 0,
                    }),
                    log_records: vec![LogRecord {
                        time_unix_nano: 1_715_689_200_000_000_000,
                        observed_time_unix_nano: 0,
                        severity_number: 17,
                        severity_text: "ERROR".into(),
                        body: Some(ProtoAnyValueMessage {
                            value: Some(ProtoAnyValue::StringValue("payment failed".into())),
                        }),
                        attributes: vec![
                            KeyValue {
                                key: "http.route".into(),
                                value: Some(ProtoAnyValueMessage {
                                    value: Some(ProtoAnyValue::StringValue("/pay".into())),
                                }),
                                key_strindex: 0,
                            },
                            KeyValue {
                                key: "http.status_code".into(),
                                value: Some(ProtoAnyValueMessage {
                                    value: Some(ProtoAnyValue::IntValue(502)),
                                }),
                                key_strindex: 0,
                            },
                        ],
                        dropped_attributes_count: 0,
                        flags: 0,
                        trace_id: trace_id.to_vec(),
                        span_id: span_id.to_vec(),
                        event_name: String::new(),
                    }],
                    schema_url: String::new(),
                }],
                schema_url: String::new(),
            }],
        };

        let normalized =
            normalize_otlp_logs_protobuf_request(&request.encode_to_vec()).expect("normalize");
        let out =
            build_new_event_records_from_otlp_logs_json(&normalized, 10).expect("parse protobuf");
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.rejected_log_records, 0);
        let r = &out.records[0];
        assert_eq!(r.service_id, "checkout");
        assert_eq!(r.message, "payment failed");
        assert_eq!(
            r.trace_id.as_deref(),
            Some("aabbccdd0011223344556677889900aa")
        );
        assert_eq!(r.span_id.as_deref(), Some("0102030405060708"));
        assert_eq!(r.severity, 3);
        assert_eq!(r.deployment_environment.as_deref(), Some("staging"));
    }
}
