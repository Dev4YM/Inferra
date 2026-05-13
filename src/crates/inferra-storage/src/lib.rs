//! SQLite access compatible with the historical Python `events.db` and `incidents.db`.

use anyhow::{Context, Result};
use inferra_contracts::{EventRow, EventSourceRef, HypothesisRow, IncidentRow, SeverityValue};
use rusqlite::{Connection, OptionalExtension, Row, Transaction};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use time::{Duration, OffsetDateTime};

#[derive(Debug, Clone, Default)]
pub struct ServiceStats {
    pub service_id: String,
    pub event_count: i64,
    pub error_count: i64,
    pub last_event_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewEventRecord {
    pub event_id: String,
    pub timestamp: String,
    pub service_id: String,
    pub severity: i64,
    pub message: String,
    pub source_type: String,
    pub source_id: String,
    pub tags: Vec<String>,
    pub fingerprint: String,
    pub host_id: String,
    pub event_type: i64,
    pub timestamp_source: String,
    pub collected_at: String,
    pub quality: Option<String>,
    pub structured_data: Option<Value>,
    pub raw_offset: Option<i64>,
}

impl NewEventRecord {
    pub fn minimal(
        event_id: impl Into<String>,
        timestamp: impl Into<String>,
        service_id: impl Into<String>,
        severity: i64,
        message: impl Into<String>,
        source_type: impl Into<String>,
        collected_at: impl Into<String>,
    ) -> Self {
        let event_id = event_id.into();
        let service_id = service_id.into();
        Self {
            fingerprint: event_id.clone(),
            source_id: service_id.clone(),
            event_id,
            timestamp: timestamp.into(),
            service_id,
            severity,
            message: message.into(),
            source_type: source_type.into(),
            tags: Vec::new(),
            host_id: "local".into(),
            event_type: 0,
            timestamp_source: "collector".into(),
            collected_at: collected_at.into(),
            quality: None,
            structured_data: None,
            raw_offset: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IncidentRecord {
    pub incident_id: String,
    pub state: String,
    pub severity: i64,
    pub primary_service: String,
    pub affected_services: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub time_range_start: String,
    pub time_range_end: String,
    pub event_count: i64,
    pub cluster_ids: Vec<String>,
    pub runtime_context: Option<Value>,
    pub resolution_info: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct StoredHypothesis {
    pub hypothesis_id: String,
    pub rank: Option<i64>,
    pub cause_type: String,
    pub description: String,
    pub total_score: Option<f64>,
    pub score_breakdown: Value,
    pub supporting_events: Vec<String>,
    pub contradicting_events: Vec<String>,
    pub affected_services: Vec<String>,
    pub suggested_checks: Vec<String>,
    pub confidence_label: Option<String>,
    pub is_valid: bool,
    pub invalidation_reasons: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct StoredExplanation {
    pub explanation_id: String,
    pub incident_id: String,
    pub summary: String,
    pub primary_text: String,
    pub evidence_text: Option<String>,
    pub timeline_text: Option<String>,
    pub alternatives: Vec<String>,
    pub actions: Vec<String>,
    pub uncertainty: Vec<String>,
    pub model_used: String,
    pub guardrail_flags: Vec<String>,
    pub created_at: String,
    pub explanation_schema_version: i64,
    pub hypotheses_hash: String,
    pub events_hash_head: String,
    pub quality: String,
}

#[derive(Debug, Clone)]
pub struct StoredFeedback {
    pub feedback_id: String,
    pub incident_id: String,
    pub correct_hypothesis_id: Option<String>,
    pub feedback_type: String,
    pub operator_notes: String,
    pub resolved_at: String,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredAiTrace {
    pub trace_id: String,
    pub incident_id: String,
    pub trace_kind: String,
    pub sanitized_system_prompt: String,
    pub sanitized_user_prompt: String,
    pub allowed_fields: Vec<String>,
    pub blocked_fields: Vec<String>,
    pub raw_logs_sent: bool,
    pub trace_schema_version: i64,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct StoredAiGeneration {
    pub generation_id: String,
    pub scope_key: String,
    pub focus: String,
    pub mode: String,
    pub question: String,
    pub response: Value,
    pub bundle_hash: String,
    pub used_ai: bool,
    pub provider: Value,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct StoredInferenceGraphSnapshot {
    pub incident_id: String,
    pub graph_data: Value,
    pub created_at: String,
    pub event_count: i64,
}

#[derive(Debug, Clone)]
pub struct StoredUiSnapshot {
    pub data_type: String,
    pub payload: Value,
    pub source: String,
    pub updated_at: String,
    pub schema_version: i64,
    pub interval_seconds: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct StoredAdaptiveLearningAuditEntry {
    pub audit_id: String,
    pub artifact_kind: String,
    pub artifact_id: String,
    pub action: String,
    pub reason: Option<String>,
    pub previous_status: String,
    pub new_status: String,
    pub review_status_before: Option<String>,
    pub review_status_after: Option<String>,
    pub runtime_effect: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct AdaptiveLearningAuditQuery {
    pub artifact_kind: Option<String>,
    pub artifact_id: Option<String>,
    pub action: Option<String>,
    pub review_status_after: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone)]
pub struct StoredAdaptiveLearningHistoryEntry {
    pub entry_id: String,
    pub artifact_kind: String,
    pub artifact_id: String,
    pub artifact_label: String,
    pub incident_id: String,
    pub cause_type: String,
    pub hypothesis_id: String,
    pub observed_at: String,
    pub score: Option<f64>,
    pub rank: Option<i64>,
    pub estimated_impact: f64,
    pub impact_metric: Option<String>,
    pub score_delta: Option<f64>,
    pub rank_delta: Option<i64>,
    pub edge_delta: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct AdaptiveLearningHistoryQuery {
    pub artifact_kind: Option<String>,
    pub artifact_id: Option<String>,
    pub incident_id: Option<String>,
    pub cause_type: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Default)]
pub struct StoredAdaptiveLearningModel {
    pub schema_version: i64,
    pub last_updated: Option<String>,
    pub processed_feedback_ids: Vec<String>,
    pub learned_detectors: Vec<StoredLearnedDetector>,
    pub learned_templates: Vec<StoredLearnedTemplate>,
    pub learned_compositions: Vec<StoredLearnedComposition>,
    pub learned_edge_profiles: Vec<StoredLearnedEdgeProfile>,
}

#[derive(Debug, Clone)]
pub struct StoredLearnedDetector {
    pub detector_id: String,
    pub requirement_name: String,
    pub cause_type: String,
    pub positive_terms: Vec<String>,
    pub tags: Vec<String>,
    pub source_types: Vec<String>,
    pub min_severity: Option<i64>,
    pub confirmations: i64,
    pub false_positives: i64,
    pub created_from_feedback_id: String,
    pub updated_at: String,
    pub manually_disabled: bool,
    pub status_reason: Option<String>,
    pub review_status: String,
    pub review_reason: Option<String>,
    pub last_reviewed_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredLearnedTemplate {
    pub template_id: String,
    pub template_name: String,
    pub cause_type: String,
    pub cause_subtype: Option<String>,
    pub title_template: String,
    pub confidence: f64,
    pub requires: Vec<String>,
    pub requires_same_service: bool,
    pub requires_temporal_order: bool,
    pub confirmations: i64,
    pub false_positives: i64,
    pub created_from_feedback_id: String,
    pub updated_at: String,
    pub manually_disabled: bool,
    pub status_reason: Option<String>,
    pub review_status: String,
    pub review_reason: Option<String>,
    pub last_reviewed_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredLearnedComposition {
    pub composition_id: String,
    pub composition_name: String,
    pub cause_type: String,
    pub cause_subtype: Option<String>,
    pub title_template: String,
    pub confidence: f64,
    pub requires: Vec<String>,
    pub requires_same_service: bool,
    pub requires_temporal_order: bool,
    pub preferred_edge_types: Vec<String>,
    pub confirmations: i64,
    pub false_positives: i64,
    pub created_from_feedback_id: String,
    pub updated_at: String,
    pub manually_disabled: bool,
    pub status_reason: Option<String>,
    pub review_status: String,
    pub review_reason: Option<String>,
    pub last_reviewed_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredLearnedEdgeProfile {
    pub profile_id: String,
    pub edge_type: String,
    pub source_service: Option<String>,
    pub target_service: Option<String>,
    pub cause_type: Option<String>,
    pub confirmations: i64,
    pub false_positives: i64,
    pub average_plausibility: f64,
    pub average_latency_ms: f64,
    pub created_from_feedback_id: String,
    pub updated_at: String,
    pub manually_disabled: bool,
    pub status_reason: Option<String>,
    pub review_status: String,
    pub review_reason: Option<String>,
    pub last_reviewed_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredAdaptiveReviewViewSelection {
    pub artifact_kind: String,
    pub artifact_id: String,
}

#[derive(Debug, Clone)]
pub struct StoredAdaptiveReviewView {
    pub view_id: String,
    pub name: String,
    pub description: Option<String>,
    pub search_text: Option<String>,
    pub assigned_reviewer: Option<String>,
    pub artifact_selections: Vec<StoredAdaptiveReviewViewSelection>,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StoredChatMessage {
    pub message_id: String,
    pub incident_id: String,
    pub role: String,
    pub content: String,
    pub message_schema_version: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct GovernanceRule {
    pub pattern: String,
    pub service_id: Option<String>,
    pub severity_min: Option<i64>,
    pub severity_max: Option<i64>,
    pub tags: Vec<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IngestGovernance {
    pub dedup_enabled: bool,
    pub dedup_window_seconds: i64,
    pub max_tracked_fingerprints: usize,
    pub severity_escalation_splits: bool,
    pub noise_enabled: bool,
    pub blocklist_enabled: bool,
    pub allowlist_enabled: bool,
    pub registry_enabled: bool,
    pub high_rate_threshold_per_minute: i64,
    pub always_keep_severity: i64,
    pub blocklist: Vec<GovernanceRule>,
    pub allowlist: Vec<GovernanceRule>,
}

impl Default for IngestGovernance {
    fn default() -> Self {
        Self {
            dedup_enabled: false,
            dedup_window_seconds: 60,
            max_tracked_fingerprints: 10_000,
            severity_escalation_splits: false,
            noise_enabled: false,
            blocklist_enabled: false,
            allowlist_enabled: false,
            registry_enabled: false,
            high_rate_threshold_per_minute: 100,
            always_keep_severity: 3,
            blocklist: Vec::new(),
            allowlist: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct IngestBatchResult {
    pub inserted: usize,
    pub inserted_event_ids: Vec<String>,
    pub suppressed_duplicates: usize,
    pub suppressed_noise: usize,
    pub allowlisted: usize,
    pub retained_due_to_severity: usize,
}

#[derive(Debug, Clone, Default)]
pub struct GovernanceSummary {
    pub dedup_suppressed_total: i64,
    pub noise_suppressed_total: i64,
    pub allowlisted_total: i64,
    pub retained_due_to_severity_total: i64,
    pub inserted_total: i64,
    pub tracked_fingerprints: i64,
    pub active_dedup_windows: i64,
    pub active_window_suppressed: i64,
    pub last_noise_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct DedupWindowState {
    count: i64,
    suppressed_count: i64,
    window_end: String,
}

pub struct EventsStore {
    conn: Connection,
}

impl EventsStore {
    pub fn open(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let conn =
            Connection::open(path).with_context(|| format!("open events db {}", path.display()))?;
        Ok(Some(Self { conn }))
    }

    pub fn service_aggregates(&self, limit: usize) -> Result<Vec<ServiceStats>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT service_id, COUNT(*) as c, \
                 SUM(CASE WHEN severity >= 3 THEN 1 ELSE 0 END) as err, \
                 MAX(timestamp) as last_ts \
                 FROM events GROUP BY service_id \
                 ORDER BY last_ts DESC LIMIT ?1",
            )
            .context("prepare service stats")?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |r| {
                Ok(ServiceStats {
                    service_id: r.get(0)?,
                    event_count: r.get(1)?,
                    error_count: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    last_event_at: r.get(3)?,
                })
            })
            .context("query_map services")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn count_events(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .context("count events")?;
        Ok(count as usize)
    }

    pub fn latest_events(&self, limit: usize) -> Result<Vec<EventRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT event_id, timestamp, severity, service_id, message, source_type, tags \
                 FROM events ORDER BY timestamp DESC LIMIT ?1",
            )
            .context("prepare latest events")?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], event_row_from_row)
            .context("query latest events")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out.reverse();
        Ok(out)
    }

    pub fn get_event(&self, event_id: &str) -> Result<Option<EventRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT event_id, timestamp, severity, service_id, message, source_type, tags \
                 FROM events WHERE event_id = ?1",
            )
            .context("prepare event detail")?;
        let mut rows = stmt.query(rusqlite::params![event_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(event_row_from_row(row)?))
    }

    pub fn get_events(&self, event_ids: &[String]) -> Result<Vec<EventRow>> {
        if event_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (0..event_ids.len())
            .map(|idx| format!("?{}", idx + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT event_id, timestamp, severity, service_id, message, source_type, tags \
             FROM events WHERE event_id IN ({placeholders})"
        );
        let mut stmt = self
            .conn
            .prepare(&sql)
            .context("prepare batch event detail")?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(event_ids.iter()),
                event_row_from_row,
            )
            .context("query batch event detail")?;
        let mut by_id = HashMap::new();
        for row in rows {
            let event = row?;
            if let Some(event_id) = event.event_id.clone() {
                by_id.insert(event_id, event);
            }
        }
        let mut out = Vec::with_capacity(event_ids.len());
        for event_id in event_ids {
            if let Some(event) = by_id.remove(event_id) {
                out.push(event);
            }
        }
        Ok(out)
    }

    pub fn events_for_service(&self, service_id: &str, limit: usize) -> Result<Vec<EventRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT event_id, timestamp, severity, service_id, message, source_type, tags \
                 FROM events WHERE service_id = ?1 ORDER BY timestamp DESC LIMIT ?2",
            )
            .context("prepare service events")?;
        let rows = stmt
            .query_map(
                rusqlite::params![service_id, limit as i64],
                event_row_from_row,
            )
            .context("query service events")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out.reverse();
        Ok(out)
    }

    pub fn query_logs(
        &self,
        limit: usize,
        service: Option<&str>,
        min_severity: Option<i64>,
        search: Option<&str>,
        source_type: Option<&str>,
    ) -> Result<Vec<EventRow>> {
        let mut sql = String::from(
            "SELECT event_id, timestamp, severity, service_id, message, source_type, tags \
             FROM events WHERE 1=1 AND timestamp >= datetime('now', '-24 hours')",
        );
        let mut params: Vec<rusqlite::types::Value> = Vec::new();

        if let Some(service) = service.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND service_id = ?");
            sql.push_str(&(params.len() + 1).to_string());
            params.push(service.to_string().into());
        }
        if let Some(min_severity) = min_severity {
            sql.push_str(" AND severity >= ?");
            sql.push_str(&(params.len() + 1).to_string());
            params.push(min_severity.into());
        }
        if let Some(search) = search.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND message LIKE '%' || ?");
            sql.push_str(&(params.len() + 1).to_string());
            sql.push_str(" || '%'");
            params.push(search.trim().to_string().into());
        }
        if let Some(source_type) = source_type.filter(|value| !value.trim().is_empty()) {
            sql.push_str(" AND source_type = ?");
            sql.push_str(&(params.len() + 1).to_string());
            params.push(source_type.to_string().into());
        }

        sql.push_str(" ORDER BY timestamp DESC LIMIT ?");
        sql.push_str(&(params.len() + 1).to_string());
        params.push((limit as i64).into());

        let mut stmt = self.conn.prepare(&sql).context("prepare logs query")?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter()),
                event_row_from_row,
            )
            .context("query logs")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn insert_batch(&mut self, events: &[NewEventRecord]) -> Result<usize> {
        Ok(self
            .insert_batch_governed(events, &IngestGovernance::default())?
            .inserted)
    }

    pub fn insert_batch_governed(
        &mut self,
        events: &[NewEventRecord],
        governance: &IngestGovernance,
    ) -> Result<IngestBatchResult> {
        if events.is_empty() {
            return Ok(IngestBatchResult::default());
        }
        let tx = self
            .conn
            .transaction()
            .context("begin event insert transaction")?;
        let mut result = IngestBatchResult::default();
        let watermark = events
            .iter()
            .map(|event| event.collected_at.as_str())
            .max()
            .map(str::to_string)
            .unwrap_or_else(now_iso);
        cleanup_expired_dedup_windows(&tx, &watermark)?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT OR IGNORE INTO events (
                        event_id, timestamp, timestamp_source, service_id, host_id,
                        severity, event_type, message, structured_data, tags,
                        fingerprint, quality, source_type, source_id, raw_offset,
                        collected_at, schema_version
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5,
                        ?6, ?7, ?8, ?9, ?10,
                        ?11, ?12, ?13, ?14, ?15,
                        ?16, 1
                    )",
                )
                .context("prepare event insert")?;
            for original in events {
                let mut event = original.clone();
                let fingerprint = governed_fingerprint(&event, governance);
                track_fingerprint(&tx, &fingerprint, &event.collected_at)?;

                let allowlist_rule = if governance.noise_enabled && governance.allowlist_enabled {
                    governance
                        .allowlist
                        .iter()
                        .find(|rule| match_governance_rule(rule, &event))
                } else {
                    None
                };
                if let Some(rule) = allowlist_rule {
                    merge_tags(&mut event.tags, &rule.tags);
                    result.allowlisted += 1;
                    increment_governance_counter(&tx, "allowlisted_total", 1, &event.collected_at)?;
                }

                let retained_due_to_severity = event.severity >= governance.always_keep_severity;
                if retained_due_to_severity {
                    result.retained_due_to_severity += 1;
                    increment_governance_counter(
                        &tx,
                        "retained_due_to_severity_total",
                        1,
                        &event.collected_at,
                    )?;
                }

                if governance.noise_enabled
                    && governance.blocklist_enabled
                    && allowlist_rule.is_none()
                    && !retained_due_to_severity
                {
                    if let Some(rule) = governance
                        .blocklist
                        .iter()
                        .find(|rule| match_governance_rule(rule, &event))
                    {
                        result.suppressed_noise += 1;
                        increment_governance_counter(
                            &tx,
                            "noise_suppressed_total",
                            1,
                            &event.collected_at,
                        )?;
                        if let Some(reason) =
                            rule.reason.as_deref().filter(|value| !value.is_empty())
                        {
                            set_governance_value(
                                &tx,
                                "last_noise_reason",
                                reason,
                                &event.collected_at,
                            )?;
                        }
                        continue;
                    }
                }

                let existing_window = if governance.dedup_enabled
                    && allowlist_rule.is_none()
                    && !retained_due_to_severity
                {
                    load_dedup_window(&tx, &fingerprint)?
                } else {
                    None
                };
                if let Some(window) = existing_window {
                    if within_dedup_window(&event.timestamp, &window.window_end) {
                        let next_window_end = max_timestamp(
                            &window.window_end,
                            &dedup_window_end(&event.timestamp, governance.dedup_window_seconds),
                        );
                        tx.execute(
                            "UPDATE dedup_window
                             SET last_event_id = ?2,
                                 count = ?3,
                                 window_end = ?4,
                                 suppressed_count = ?5
                             WHERE fingerprint = ?1",
                            rusqlite::params![
                                fingerprint,
                                event.event_id,
                                window.count + 1,
                                next_window_end,
                                window.suppressed_count + 1,
                            ],
                        )
                        .context("update dedup window")?;
                        result.suppressed_duplicates += 1;
                        increment_governance_counter(
                            &tx,
                            "dedup_suppressed_total",
                            1,
                            &event.collected_at,
                        )?;
                        continue;
                    }
                }

                event.fingerprint = fingerprint.clone();
                let inserted = stmt
                    .execute(rusqlite::params![
                        event.event_id,
                        event.timestamp,
                        event.timestamp_source,
                        event.service_id,
                        event.host_id,
                        event.severity,
                        event.event_type,
                        event.message,
                        event.structured_data.as_ref().map(Value::to_string),
                        serde_json::to_string(&event.tags).unwrap_or_else(|_| "[]".into()),
                        event.fingerprint,
                        event.quality,
                        event.source_type,
                        event.source_id,
                        event.raw_offset,
                        event.collected_at,
                    ])
                    .context("insert event row")?;
                if inserted > 0 {
                    result.inserted += inserted;
                    result.inserted_event_ids.push(event.event_id.clone());
                    increment_governance_counter(&tx, "inserted_total", 1, &event.collected_at)?;
                    let window_end =
                        dedup_window_end(&event.timestamp, governance.dedup_window_seconds);
                    tx.execute(
                        "INSERT INTO dedup_window (
                            fingerprint, first_event_id, last_event_id, count, window_start, window_end, suppressed_count
                         ) VALUES (?1, ?2, ?3, 1, ?4, ?5, 0)
                         ON CONFLICT(fingerprint) DO UPDATE SET
                            first_event_id = excluded.first_event_id,
                            last_event_id = excluded.last_event_id,
                            count = excluded.count,
                            window_start = excluded.window_start,
                            window_end = excluded.window_end,
                            suppressed_count = excluded.suppressed_count",
                        rusqlite::params![
                            fingerprint,
                            event.event_id,
                            event.event_id,
                            event.timestamp,
                            window_end,
                        ],
                    )
                    .context("upsert dedup window")?;
                }
            }
        }
        trim_governance_tracking(&tx, governance.max_tracked_fingerprints)?;
        tx.commit().context("commit event insert transaction")?;
        Ok(result)
    }

    pub fn fingerprint_exists(&self, fingerprint: &str) -> Result<bool> {
        let exists = self
            .conn
            .query_row(
                "SELECT 1 FROM fingerprint_seen WHERE fingerprint = ?1
                 UNION
                 SELECT 1 FROM events WHERE fingerprint = ?1
                 LIMIT 1",
                rusqlite::params![fingerprint],
                |_| Ok(()),
            )
            .optional()
            .context("query fingerprint existence")?
            .is_some();
        Ok(exists)
    }

    pub fn get_collector_state(
        &self,
        collector_id: &str,
        state_key: &str,
    ) -> Result<Option<String>> {
        match self.conn.query_row(
                "SELECT state_value FROM collector_state WHERE collector_id = ?1 AND state_key = ?2",
                rusqlite::params![collector_id, state_key],
                |row| row.get(0),
            )
            .optional()
        {
            Ok(value) => Ok(value),
            Err(error) if is_missing_table_error(&error) => Ok(None),
            Err(error) => Err(error).context("query collector state"),
        }
    }

    pub fn set_collector_state(
        &self,
        collector_id: &str,
        state_key: &str,
        state_value: &str,
        updated_at: &str,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO collector_state (collector_id, state_key, state_value, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(collector_id, state_key) DO UPDATE SET
                    state_value = excluded.state_value,
                    updated_at = excluded.updated_at",
                rusqlite::params![collector_id, state_key, state_value, updated_at],
            )
            .context("upsert collector state")?;
        Ok(())
    }

    pub fn governance_summary(&self) -> Result<GovernanceSummary> {
        let tracked_fingerprints = self
            .conn
            .query_row("SELECT COUNT(*) FROM fingerprint_seen", [], |row| {
                row.get(0)
            })
            .or_else(|error| {
                if is_missing_table_error(&error) {
                    Ok(0)
                } else {
                    Err(error)
                }
            })
            .context("count tracked fingerprints")?;
        let (active_dedup_windows, active_window_suppressed) = self
            .conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(suppressed_count), 0) FROM dedup_window",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .or_else(|error| {
                if is_missing_table_error(&error) {
                    Ok((0, 0))
                } else {
                    Err(error)
                }
            })
            .context("summarize dedup windows")?;
        Ok(GovernanceSummary {
            dedup_suppressed_total: governance_counter(self, "dedup_suppressed_total")?,
            noise_suppressed_total: governance_counter(self, "noise_suppressed_total")?,
            allowlisted_total: governance_counter(self, "allowlisted_total")?,
            retained_due_to_severity_total: governance_counter(
                self,
                "retained_due_to_severity_total",
            )?,
            inserted_total: governance_counter(self, "inserted_total")?,
            tracked_fingerprints,
            active_dedup_windows,
            active_window_suppressed,
            last_noise_reason: match self.get_collector_state("governance", "last_noise_reason") {
                Ok(value) => value,
                Err(error) if is_missing_table_error(&error) => None,
                Err(error) => return Err(error),
            },
        })
    }

    pub fn prune_expired(&self, retention_hours: i64) -> Result<usize> {
        let deleted = self
            .conn
            .execute(
                "DELETE FROM events WHERE inserted_at < datetime('now', ?1)",
                rusqlite::params![format!("-{} hours", retention_hours.max(1))],
            )
            .context("prune expired events")?;
        Ok(deleted)
    }
}

fn governed_fingerprint(event: &NewEventRecord, governance: &IngestGovernance) -> String {
    let base = if event.fingerprint.trim().is_empty() {
        format!(
            "{}:{}:{}",
            event.service_id.trim().to_ascii_lowercase(),
            event.source_type.trim().to_ascii_lowercase(),
            normalized_message(&event.message)
        )
    } else {
        event.fingerprint.trim().to_ascii_lowercase()
    };
    if governance.severity_escalation_splits {
        format!("{base}::sev{}", event.severity)
    } else {
        base
    }
}

fn normalized_message(message: &str) -> String {
    let normalized = message
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == ' ' {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>();
    normalized
        .split_whitespace()
        .take(18)
        .collect::<Vec<_>>()
        .join(" ")
}

fn match_governance_rule(rule: &GovernanceRule, event: &NewEventRecord) -> bool {
    if !rule.pattern.is_empty()
        && !event
            .message
            .to_ascii_lowercase()
            .contains(&rule.pattern.to_ascii_lowercase())
    {
        return false;
    }
    if let Some(service_id) = rule.service_id.as_deref().filter(|value| !value.is_empty()) {
        if service_id != event.service_id {
            return false;
        }
    }
    if let Some(severity_min) = rule.severity_min {
        if event.severity < severity_min {
            return false;
        }
    }
    if let Some(severity_max) = rule.severity_max {
        if event.severity > severity_max {
            return false;
        }
    }
    true
}

fn merge_tags(target: &mut Vec<String>, extra: &[String]) {
    for tag in extra {
        if !target.iter().any(|existing| existing == tag) {
            target.push(tag.clone());
        }
    }
}

fn track_fingerprint(tx: &Transaction<'_>, fingerprint: &str, observed_at: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO fingerprint_seen (fingerprint, first_seen_at, last_seen_at, hit_count)
         VALUES (?1, ?2, ?3, 1)
         ON CONFLICT(fingerprint) DO UPDATE SET
            last_seen_at = excluded.last_seen_at,
            hit_count = fingerprint_seen.hit_count + 1",
        rusqlite::params![fingerprint, observed_at, observed_at],
    )
    .context("track fingerprint")?;
    Ok(())
}

fn load_dedup_window(tx: &Transaction<'_>, fingerprint: &str) -> Result<Option<DedupWindowState>> {
    tx.query_row(
        "SELECT count, suppressed_count, window_end FROM dedup_window WHERE fingerprint = ?1",
        rusqlite::params![fingerprint],
        |row| {
            Ok(DedupWindowState {
                count: row.get(0)?,
                suppressed_count: row.get(1)?,
                window_end: row.get(2)?,
            })
        },
    )
    .optional()
    .context("load dedup window")
}

fn within_dedup_window(timestamp: &str, window_end: &str) -> bool {
    match (parse_rfc3339(timestamp), parse_rfc3339(window_end)) {
        (Some(ts), Some(end)) => ts <= end,
        _ => timestamp <= window_end,
    }
}

fn dedup_window_end(timestamp: &str, seconds: i64) -> String {
    parse_rfc3339(timestamp)
        .map(|value| value + Duration::seconds(seconds.max(1)))
        .and_then(|value| {
            value
                .format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| timestamp.to_string())
}

fn max_timestamp(left: &str, right: &str) -> String {
    match (parse_rfc3339(left), parse_rfc3339(right)) {
        (Some(a), Some(b)) => {
            if a >= b {
                left.to_string()
            } else {
                right.to_string()
            }
        }
        _ => {
            if left >= right {
                left.to_string()
            } else {
                right.to_string()
            }
        }
    }
}

fn cleanup_expired_dedup_windows(tx: &Transaction<'_>, now: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM dedup_window WHERE window_end < ?1",
        rusqlite::params![now],
    )
    .context("cleanup expired dedup windows")?;
    Ok(())
}

fn trim_governance_tracking(tx: &Transaction<'_>, max_tracked_fingerprints: usize) -> Result<()> {
    if max_tracked_fingerprints == 0 {
        return Ok(());
    }
    let tracked: i64 = tx
        .query_row("SELECT COUNT(*) FROM fingerprint_seen", [], |row| {
            row.get(0)
        })
        .context("count tracked fingerprints for trim")?;
    let excess = tracked - max_tracked_fingerprints as i64;
    if excess <= 0 {
        return Ok(());
    }
    let mut stmt = tx
        .prepare("SELECT fingerprint FROM fingerprint_seen ORDER BY last_seen_at ASC LIMIT ?1")
        .context("prepare tracked fingerprint trim")?;
    let rows = stmt
        .query_map(rusqlite::params![excess], |row| row.get::<_, String>(0))
        .context("query tracked fingerprint trim")?;
    let mut fingerprints = Vec::new();
    for item in rows {
        fingerprints.push(item?);
    }
    drop(stmt);
    for fingerprint in fingerprints {
        tx.execute(
            "DELETE FROM dedup_window WHERE fingerprint = ?1",
            rusqlite::params![fingerprint],
        )
        .context("trim dedup window fingerprint")?;
        tx.execute(
            "DELETE FROM fingerprint_seen WHERE fingerprint = ?1",
            rusqlite::params![fingerprint],
        )
        .context("trim fingerprint seen")?;
    }
    Ok(())
}

fn increment_governance_counter(
    tx: &Transaction<'_>,
    state_key: &str,
    amount: i64,
    updated_at: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO collector_state (collector_id, state_key, state_value, updated_at)
         VALUES ('governance', ?1, ?2, ?3)
         ON CONFLICT(collector_id, state_key) DO UPDATE SET
            state_value = CAST(
                COALESCE(CAST(collector_state.state_value AS INTEGER), 0)
                + CAST(excluded.state_value AS INTEGER)
                AS TEXT
            ),
            updated_at = excluded.updated_at",
        rusqlite::params![state_key, amount.to_string(), updated_at],
    )
    .with_context(|| format!("increment governance counter {state_key}"))?;
    Ok(())
}

fn set_governance_value(
    tx: &Transaction<'_>,
    state_key: &str,
    state_value: &str,
    updated_at: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO collector_state (collector_id, state_key, state_value, updated_at)
         VALUES ('governance', ?1, ?2, ?3)
         ON CONFLICT(collector_id, state_key) DO UPDATE SET
            state_value = excluded.state_value,
            updated_at = excluded.updated_at",
        rusqlite::params![state_key, state_value, updated_at],
    )
    .with_context(|| format!("set governance value {state_key}"))?;
    Ok(())
}

fn governance_counter(store: &EventsStore, state_key: &str) -> Result<i64> {
    match store.get_collector_state("governance", state_key) {
        Ok(value) => Ok(value
            .and_then(|item| item.parse::<i64>().ok())
            .unwrap_or_default()),
        Err(error) if is_missing_table_error(&error) => Ok(0),
        Err(error) => Err(error),
    }
}

pub fn initialize_databases(events_db: &Path, incidents_db: &Path) -> Result<()> {
    if let Some(parent) = events_db.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create events db parent {}", parent.display()))?;
    }
    if let Some(parent) = incidents_db.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create incidents db parent {}", parent.display()))?;
    }

    initialize_events_db(events_db)?;
    initialize_incidents_db(incidents_db)?;
    Ok(())
}

pub struct IncidentsStore {
    conn: Connection,
}

impl IncidentsStore {
    pub fn open(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open incidents db {}", path.display()))?;
        Ok(Some(Self { conn }))
    }

    pub fn upsert_ui_snapshot(
        &self,
        data_type: &str,
        payload: &Value,
        source: &str,
        interval_seconds: Option<i64>,
    ) -> Result<StoredUiSnapshot> {
        let updated_at = now_iso();
        let payload_json = serde_json::to_string(payload).context("serialize ui snapshot")?;
        self.conn
            .execute(
                "INSERT INTO ui_snapshots (
                    data_type, payload_json, source, updated_at, schema_version, interval_seconds
                 ) VALUES (?1, ?2, ?3, ?4, 1, ?5)
                 ON CONFLICT(data_type) DO UPDATE SET
                    payload_json = excluded.payload_json,
                    source = excluded.source,
                    updated_at = excluded.updated_at,
                    schema_version = excluded.schema_version,
                    interval_seconds = excluded.interval_seconds",
                rusqlite::params![
                    data_type,
                    payload_json,
                    source,
                    updated_at,
                    interval_seconds
                ],
            )
            .with_context(|| format!("upsert ui snapshot {data_type}"))?;
        Ok(StoredUiSnapshot {
            data_type: data_type.to_string(),
            payload: payload.clone(),
            source: source.to_string(),
            updated_at,
            schema_version: 1,
            interval_seconds,
        })
    }

    pub fn ui_snapshot(&self, data_type: &str) -> Result<Option<StoredUiSnapshot>> {
        self.conn
            .query_row(
                "SELECT data_type, payload_json, source, updated_at, schema_version, interval_seconds
                 FROM ui_snapshots WHERE data_type = ?1",
                [data_type],
                ui_snapshot_from_row,
            )
            .optional()
            .with_context(|| format!("query ui snapshot {data_type}"))
    }

    pub fn ui_snapshots(&self) -> Result<Vec<StoredUiSnapshot>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT data_type, payload_json, source, updated_at, schema_version, interval_seconds
                 FROM ui_snapshots ORDER BY data_type ASC",
            )
            .context("prepare ui snapshots")?;
        let rows = stmt
            .query_map([], ui_snapshot_from_row)
            .context("query ui snapshots")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn active_incidents(&self, limit: usize) -> Result<Vec<IncidentRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT incident_id, state, severity, primary_service, affected_services, \
                 created_at, updated_at, event_count FROM incidents \
                 WHERE state IN ('open','investigating','explained') \
                 ORDER BY severity DESC, updated_at DESC LIMIT ?1",
            )
            .context("prepare incidents")?;
        let rows = stmt
            .query_map([limit as i64], |r| {
                let affected_raw: String = r.get(4)?;
                let affected: Option<Vec<String>> = serde_json::from_str(&affected_raw).ok();
                Ok(IncidentRow {
                    incident_id: r.get(0)?,
                    state: r.get(1)?,
                    severity: r.get(2)?,
                    primary_service: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    affected_services: affected,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                    event_count: r.get(7)?,
                })
            })
            .context("incidents query")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn active_incident_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM incidents WHERE state IN ('open','investigating','explained')",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn latest_active_incident_id(&self) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT incident_id FROM incidents \
                 WHERE state IN ('open','investigating','explained') \
                 ORDER BY updated_at DESC, created_at DESC LIMIT 1",
            )
            .context("prepare latest active incident")?;
        let mut rows = stmt.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(row.get(0)?))
    }

    pub fn latest_incident_id(&self) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT incident_id FROM incidents \
                 ORDER BY updated_at DESC, created_at DESC LIMIT 1",
            )
            .context("prepare latest incident")?;
        let mut rows = stmt.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(row.get(0)?))
    }

    pub fn recent_incidents_excluding(
        &self,
        exclude_id: &str,
        limit: usize,
    ) -> Result<Vec<IncidentRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT incident_id, state, severity, primary_service, affected_services, \
                 created_at, updated_at, event_count FROM incidents \
                 WHERE incident_id != ?1 \
                 ORDER BY updated_at DESC LIMIT ?2",
            )
            .context("prepare recent incidents")?;
        let rows = stmt
            .query_map(rusqlite::params![exclude_id, limit as i64], |r| {
                let affected_raw: String = r.get(4)?;
                let affected: Option<Vec<String>> = serde_json::from_str(&affected_raw).ok();
                Ok(IncidentRow {
                    incident_id: r.get(0)?,
                    state: r.get(1)?,
                    severity: r.get(2)?,
                    primary_service: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                    affected_services: affected,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                    event_count: r.get(7)?,
                })
            })
            .context("recent incidents query")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn get_operator_context(&self, scope_key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT body FROM ai_operator_context WHERE scope_key = ?1")
            .context("prepare get_operator_context")?;
        let mut rows = stmt.query(rusqlite::params![scope_key])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(row.get(0)?))
    }

    pub fn set_operator_context(&self, scope_key: &str, body: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO ai_operator_context (scope_key, body, updated_at) \
                 VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')) \
                 ON CONFLICT(scope_key) DO UPDATE SET \
                    body = excluded.body, \
                    updated_at = excluded.updated_at",
                rusqlite::params![scope_key, body],
            )
            .context("upsert operator context")?;
        Ok(())
    }

    pub fn get_incident(&self, incident_id: &str) -> Result<Option<IncidentRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT incident_id, state, severity, primary_service, affected_services, \
                 created_at, updated_at, event_count FROM incidents WHERE incident_id = ?1",
            )
            .context("prepare incident detail")?;
        let mut rows = stmt.query(rusqlite::params![incident_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(incident_row_from_row(row)?))
    }

    pub fn incident_event_ids(&self, incident_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT event_id FROM incident_events WHERE incident_id = ?1 ORDER BY added_at ASC",
            )
            .context("prepare incident event ids")?;
        let rows = stmt
            .query_map(rusqlite::params![incident_id], |row| {
                row.get::<_, String>(0)
            })
            .context("query incident event ids")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn hypotheses(&self, incident_id: &str) -> Result<Vec<HypothesisRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT hypothesis_id, cause_type, rank, description, total_score, confidence_label, suggested_checks, score_breakdown \
                 FROM hypotheses WHERE incident_id = ?1 ORDER BY rank ASC, total_score DESC",
            )
            .context("prepare hypotheses")?;
        let rows = stmt
            .query_map(rusqlite::params![incident_id], |row| {
                let suggested_raw: String = row.get(6)?;
                let score_breakdown_raw: String = row.get(7)?;
                let suggested_checks = serde_json::from_str::<Vec<String>>(&suggested_raw).ok();
                let provenance = serde_json::from_str::<Value>(&score_breakdown_raw)
                    .ok()
                    .and_then(|value| value.get("provenance").cloned());
                Ok(HypothesisRow {
                    hypothesis_id: row.get(0)?,
                    cause_type: row.get(1)?,
                    rank: row.get(2)?,
                    description: row.get(3)?,
                    total_score: row.get(4)?,
                    confidence_label: row.get(5)?,
                    suggested_checks,
                    provenance,
                })
            })
            .context("query hypotheses")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn hypothesis_records(&self, incident_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT hypothesis_id, cause_type, rank, description, total_score, score_breakdown,
                        supporting_events, contradicting_events, affected_services, suggested_checks,
                        confidence_label, is_valid, invalidation_reasons, created_at, updated_at
                 FROM hypotheses WHERE incident_id = ?1 ORDER BY rank ASC, total_score DESC",
            )
            .context("prepare full hypotheses query")?;
        let rows = stmt
            .query_map(rusqlite::params![incident_id], |row| {
                let score_breakdown_raw: String = row.get(5)?;
                let supporting_raw: String = row.get(6)?;
                let contradicting_raw: String = row.get(7)?;
                let affected_raw: String = row.get(8)?;
                let suggested_raw: String = row.get(9)?;
                let invalidation_raw: String = row.get(12)?;
                Ok(serde_json::json!({
                    "hypothesis_id": row.get::<_, String>(0)?,
                    "cause_type": row.get::<_, String>(1)?,
                    "rank": row.get::<_, Option<i64>>(2)?,
                    "description": row.get::<_, String>(3)?,
                    "total_score": row.get::<_, Option<f64>>(4)?,
                    "score_breakdown": serde_json::from_str::<Value>(&score_breakdown_raw).unwrap_or(Value::Null),
                    "supporting_events": parse_json_array(supporting_raw),
                    "contradicting_events": parse_json_array(contradicting_raw),
                    "affected_services": parse_json_array(affected_raw),
                    "suggested_checks": parse_json_array(suggested_raw),
                    "confidence_label": row.get::<_, Option<String>>(10)?,
                    "is_valid": row.get::<_, i64>(11)? != 0,
                    "invalidation_reasons": parse_json_array(invalidation_raw),
                    "created_at": row.get::<_, String>(13)?,
                    "updated_at": row.get::<_, String>(14)?,
                }))
            })
            .context("query full hypotheses")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn clusters(&self, incident_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT cluster_data FROM incident_clusters WHERE incident_id = ?1 ORDER BY cluster_id ASC",
            )
            .context("prepare incident clusters")?;
        let rows = stmt
            .query_map(rusqlite::params![incident_id], |row| {
                row.get::<_, String>(0)
            })
            .context("query incident clusters")?;
        let mut out = Vec::new();
        for row in rows {
            let raw = row?;
            if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                out.push(parsed);
            }
        }
        Ok(out)
    }

    pub fn upsert_inference_graph_snapshot(
        &self,
        snapshot: &StoredInferenceGraphSnapshot,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO inference_graph_snapshots (
                    incident_id, graph_data, created_at, event_count
                ) VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(incident_id) DO UPDATE SET
                    graph_data = excluded.graph_data,
                    created_at = excluded.created_at,
                    event_count = excluded.event_count",
                rusqlite::params![
                    snapshot.incident_id,
                    snapshot.graph_data.to_string(),
                    snapshot.created_at,
                    snapshot.event_count,
                ],
            )
            .context("upsert inference graph snapshot")?;
        Ok(())
    }

    pub fn inference_graph_snapshot(&self, incident_id: &str) -> Result<Option<Value>> {
        self.conn
            .query_row(
                "SELECT graph_data, created_at, event_count
                 FROM inference_graph_snapshots WHERE incident_id = ?1",
                rusqlite::params![incident_id],
                |row| {
                    let graph_raw: String = row.get(0)?;
                    let graph_data = serde_json::from_str::<Value>(&graph_raw)
                        .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
                    Ok(serde_json::json!({
                        "incident_id": incident_id,
                        "graph_data": graph_data,
                        "created_at": row.get::<_, String>(1)?,
                        "event_count": row.get::<_, i64>(2)?,
                    }))
                },
            )
            .optional()
            .context("query inference graph snapshot")
    }

    pub fn upsert_incident(
        &mut self,
        incident: &IncidentRecord,
        event_ids: &[String],
    ) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .context("begin incident upsert transaction")?;
        tx.execute(
            "INSERT INTO incidents (
                incident_id, state, created_at, updated_at, severity, primary_service,
                affected_services, time_range_start, time_range_end, event_count,
                schema_version, cluster_ids, runtime_context, resolution_info
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6,
                ?7, ?8, ?9, ?10,
                1, ?11, ?12, ?13
            )
            ON CONFLICT(incident_id) DO UPDATE SET
                state = excluded.state,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                severity = excluded.severity,
                primary_service = excluded.primary_service,
                affected_services = excluded.affected_services,
                time_range_start = excluded.time_range_start,
                time_range_end = excluded.time_range_end,
                event_count = excluded.event_count,
                cluster_ids = excluded.cluster_ids,
                runtime_context = excluded.runtime_context,
                resolution_info = excluded.resolution_info",
            rusqlite::params![
                incident.incident_id,
                incident.state,
                incident.created_at,
                incident.updated_at,
                incident.severity,
                incident.primary_service,
                serde_json::to_string(&incident.affected_services).unwrap_or_else(|_| "[]".into()),
                incident.time_range_start,
                incident.time_range_end,
                incident.event_count,
                serde_json::to_string(&incident.cluster_ids).unwrap_or_else(|_| "[]".into()),
                incident.runtime_context.as_ref().map(Value::to_string),
                incident.resolution_info.as_ref().map(Value::to_string),
            ],
        )
        .context("upsert incident row")?;
        tx.execute(
            "DELETE FROM incident_events WHERE incident_id = ?1",
            rusqlite::params![incident.incident_id],
        )
        .context("clear incident events")?;
        let mut stmt = tx
            .prepare(
                "INSERT OR IGNORE INTO incident_events (incident_id, event_id, added_at)
                 VALUES (?1, ?2, ?3)",
            )
            .context("prepare incident event insert")?;
        for event_id in event_ids {
            stmt.execute(rusqlite::params![
                incident.incident_id,
                event_id,
                incident.updated_at
            ])
            .context("insert incident event")?;
        }
        drop(stmt);
        tx.commit().context("commit incident upsert transaction")?;
        Ok(())
    }

    pub fn add_events_to_incident(
        &mut self,
        incident_id: &str,
        event_ids: &[String],
        updated_at: &str,
    ) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .context("begin incident event transaction")?;
        let mut stmt = tx
            .prepare(
                "INSERT OR IGNORE INTO incident_events (incident_id, event_id, added_at)
                 VALUES (?1, ?2, ?3)",
            )
            .context("prepare incident events append")?;
        for event_id in event_ids {
            stmt.execute(rusqlite::params![incident_id, event_id, updated_at])
                .context("append incident event")?;
        }
        drop(stmt);
        let event_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM incident_events WHERE incident_id = ?1",
                rusqlite::params![incident_id],
                |row| row.get(0),
            )
            .context("count incident events")?;
        tx.execute(
            "UPDATE incidents SET event_count = ?2, updated_at = ?3 WHERE incident_id = ?1",
            rusqlite::params![incident_id, event_count, updated_at],
        )
        .context("update incident counters")?;
        tx.commit().context("commit incident event transaction")?;
        Ok(())
    }

    pub fn replace_hypotheses(
        &mut self,
        incident_id: &str,
        hypotheses: &[StoredHypothesis],
    ) -> Result<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .context("begin hypothesis transaction")?;
        tx.execute(
            "DELETE FROM hypotheses WHERE incident_id = ?1",
            rusqlite::params![incident_id],
        )
        .context("delete existing hypotheses")?;
        let mut stmt = tx
            .prepare(
                "INSERT INTO hypotheses (
                    hypothesis_id, incident_id, rank, cause_type, description, total_score,
                    score_breakdown, supporting_events, contradicting_events, affected_services,
                    suggested_checks, confidence_label, is_valid, invalidation_reasons,
                    created_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, ?10,
                    ?11, ?12, ?13, ?14,
                    ?15, ?16
                )",
            )
            .context("prepare hypothesis insert")?;
        for item in hypotheses {
            stmt.execute(rusqlite::params![
                item.hypothesis_id,
                incident_id,
                item.rank,
                item.cause_type,
                item.description,
                item.total_score,
                item.score_breakdown.to_string(),
                serde_json::to_string(&item.supporting_events).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&item.contradicting_events).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&item.affected_services).unwrap_or_else(|_| "[]".into()),
                serde_json::to_string(&item.suggested_checks).unwrap_or_else(|_| "[]".into()),
                item.confidence_label,
                if item.is_valid { 1 } else { 0 },
                serde_json::to_string(&item.invalidation_reasons).unwrap_or_else(|_| "[]".into()),
                item.created_at,
                item.updated_at,
            ])
            .context("insert hypothesis")?;
        }
        drop(stmt);
        tx.commit().context("commit hypothesis transaction")?;
        Ok(())
    }

    pub fn upsert_cluster(
        &self,
        incident_id: &str,
        cluster_id: &str,
        cluster_data: &Value,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO incident_clusters (incident_id, cluster_id, cluster_data)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(incident_id, cluster_id) DO UPDATE SET cluster_data = excluded.cluster_data",
                rusqlite::params![incident_id, cluster_id, cluster_data.to_string()],
            )
            .context("upsert incident cluster")?;
        Ok(())
    }

    pub fn add_explanation(&self, explanation: &StoredExplanation) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO explanations (
                    explanation_id, incident_id, summary, primary_text, evidence_text, timeline_text,
                    alternatives, actions, uncertainty, model_used, guardrail_flags, created_at,
                    explanation_schema_version, hypotheses_hash, events_hash_head, quality
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16
                )",
                rusqlite::params![
                    explanation.explanation_id,
                    explanation.incident_id,
                    explanation.summary,
                    explanation.primary_text,
                    explanation.evidence_text,
                    explanation.timeline_text,
                    serde_json::to_string(&explanation.alternatives).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&explanation.actions).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&explanation.uncertainty).unwrap_or_else(|_| "[]".into()),
                    explanation.model_used,
                    serde_json::to_string(&explanation.guardrail_flags).unwrap_or_else(|_| "[]".into()),
                    explanation.created_at,
                    explanation.explanation_schema_version,
                    explanation.hypotheses_hash,
                    explanation.events_hash_head,
                    explanation.quality,
                ],
            )
            .context("insert explanation")?;
        Ok(())
    }

    pub fn latest_explanation(&self, incident_id: &str) -> Result<Option<Value>> {
        self.conn
            .query_row(
                "SELECT explanation_id, summary, primary_text, evidence_text, timeline_text,
                        alternatives, actions, uncertainty, model_used, guardrail_flags, created_at,
                        explanation_schema_version, hypotheses_hash, events_hash_head, quality
                 FROM explanations WHERE incident_id = ?1 ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![incident_id],
                explanation_json_from_row,
            )
            .optional()
            .context("query latest explanation")
    }

    pub fn cached_explanation(
        &self,
        incident_id: &str,
        hypotheses_hash: &str,
        events_hash_head: &str,
    ) -> Result<Option<Value>> {
        self.conn
            .query_row(
                "SELECT explanation_id, summary, primary_text, evidence_text, timeline_text,
                        alternatives, actions, uncertainty, model_used, guardrail_flags, created_at,
                        explanation_schema_version, hypotheses_hash, events_hash_head, quality
                 FROM explanations
                 WHERE incident_id = ?1 AND hypotheses_hash = ?2 AND events_hash_head = ?3
                 ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![incident_id, hypotheses_hash, events_hash_head],
                explanation_json_from_row,
            )
            .optional()
            .context("query cached explanation")
    }

    pub fn add_feedback(&self, feedback: &StoredFeedback) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO feedback (
                    feedback_id, incident_id, correct_hypothesis_id, feedback_type,
                    operator_notes, resolved_at, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, COALESCE(?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))",
                rusqlite::params![
                    feedback.feedback_id,
                    feedback.incident_id,
                    feedback.correct_hypothesis_id,
                    feedback.feedback_type,
                    feedback.operator_notes,
                    feedback.resolved_at,
                    feedback.created_at,
                ],
            )
            .context("insert feedback")?;
        Ok(())
    }

    pub fn all_feedback(&self) -> Result<Vec<StoredFeedback>> {
        let mut stmt = match self.conn.prepare(
            "SELECT feedback_id, incident_id, correct_hypothesis_id, feedback_type,
                    operator_notes, resolved_at, created_at
             FROM feedback ORDER BY COALESCE(created_at, resolved_at) ASC, feedback_id ASC",
        ) {
            Ok(stmt) => stmt,
            Err(error) if is_missing_table_error(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error).context("prepare feedback scan"),
        };
        let rows = match stmt.query_map([], |row| {
            Ok(StoredFeedback {
                feedback_id: row.get(0)?,
                incident_id: row.get(1)?,
                correct_hypothesis_id: row.get(2)?,
                feedback_type: row.get(3)?,
                operator_notes: row.get(4)?,
                resolved_at: row.get(5)?,
                created_at: row.get(6)?,
            })
        }) {
            Ok(rows) => rows,
            Err(error) if is_missing_table_error(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error).context("query feedback scan"),
        };
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn list_feedback(&self, incident_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT feedback_id, correct_hypothesis_id, feedback_type, operator_notes, resolved_at, created_at
                 FROM feedback WHERE incident_id = ?1 ORDER BY created_at DESC",
            )
            .context("prepare feedback query")?;
        let rows = stmt
            .query_map(rusqlite::params![incident_id], |row| {
                Ok(serde_json::json!({
                    "feedback_id": row.get::<_, String>(0)?,
                    "correct_hypothesis_id": row.get::<_, Option<String>>(1)?,
                    "feedback_type": row.get::<_, String>(2)?,
                    "operator_notes": row.get::<_, String>(3)?,
                    "resolved_at": row.get::<_, String>(4)?,
                    "created_at": row.get::<_, String>(5)?,
                }))
            })
            .context("query feedback")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn add_adaptive_learning_audit_entry(
        &self,
        entry: &StoredAdaptiveLearningAuditEntry,
    ) -> Result<()> {
        match self.conn.execute(
            "INSERT OR IGNORE INTO adaptive_learning_audit (
                audit_id, artifact_kind, artifact_id, action, reason,
                previous_status, new_status, review_status_before, review_status_after,
                runtime_effect, created_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9,
                ?10, ?11
            )",
            rusqlite::params![
                entry.audit_id,
                entry.artifact_kind,
                entry.artifact_id,
                entry.action,
                entry.reason,
                entry.previous_status,
                entry.new_status,
                entry.review_status_before,
                entry.review_status_after,
                entry.runtime_effect,
                entry.created_at,
            ],
        ) {
            Ok(_) => Ok(()),
            Err(error) if is_missing_table_error(&error) => Ok(()),
            Err(error) => Err(error).context("insert adaptive learning audit entry"),
        }
    }

    pub fn list_adaptive_learning_audit(
        &self,
        query: &AdaptiveLearningAuditQuery,
    ) -> Result<Vec<StoredAdaptiveLearningAuditEntry>> {
        let limit = query.limit.max(1) as i64;
        let offset = query.offset as i64;
        let mut stmt = match self.conn.prepare(
            "SELECT audit_id, artifact_kind, artifact_id, action, reason,
                    previous_status, new_status, review_status_before, review_status_after,
                    runtime_effect, created_at
             FROM adaptive_learning_audit
             WHERE (?1 IS NULL OR artifact_kind = ?1)
               AND (?2 IS NULL OR artifact_id = ?2)
               AND (?3 IS NULL OR action = ?3)
               AND (?4 IS NULL OR review_status_after = ?4)
             ORDER BY created_at DESC, audit_id DESC
             LIMIT ?5 OFFSET ?6",
        ) {
            Ok(stmt) => stmt,
            Err(error) if is_missing_table_error(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error).context("prepare adaptive learning audit query"),
        };
        let rows = match stmt.query_map(
            rusqlite::params![
                query.artifact_kind,
                query.artifact_id,
                query.action,
                query.review_status_after,
                limit,
                offset,
            ],
            |row| {
                Ok(StoredAdaptiveLearningAuditEntry {
                    audit_id: row.get(0)?,
                    artifact_kind: row.get(1)?,
                    artifact_id: row.get(2)?,
                    action: row.get(3)?,
                    reason: row.get(4)?,
                    previous_status: row.get(5)?,
                    new_status: row.get(6)?,
                    review_status_before: row.get(7)?,
                    review_status_after: row.get(8)?,
                    runtime_effect: row.get(9)?,
                    created_at: row.get(10)?,
                })
            },
        ) {
            Ok(rows) => rows,
            Err(error) if is_missing_table_error(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error).context("query adaptive learning audit"),
        };
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn add_adaptive_learning_history_entries(
        &mut self,
        entries: &[StoredAdaptiveLearningHistoryEntry],
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .context("begin adaptive learning history transaction")?;
        let mut stmt = match tx.prepare(
            "INSERT OR IGNORE INTO adaptive_learning_history (
                entry_id, artifact_kind, artifact_id, artifact_label, incident_id,
                cause_type, hypothesis_id, observed_at, score, rank,
                estimated_impact, impact_metric, score_delta, rank_delta, edge_delta
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10,
                ?11, ?12, ?13, ?14, ?15
            )",
        ) {
            Ok(stmt) => stmt,
            Err(error) if is_missing_table_error(&error) => return Ok(()),
            Err(error) => return Err(error).context("prepare adaptive learning history insert"),
        };
        for entry in entries {
            stmt.execute(rusqlite::params![
                entry.entry_id,
                entry.artifact_kind,
                entry.artifact_id,
                entry.artifact_label,
                entry.incident_id,
                entry.cause_type,
                entry.hypothesis_id,
                entry.observed_at,
                entry.score,
                entry.rank,
                entry.estimated_impact,
                entry.impact_metric,
                entry.score_delta,
                entry.rank_delta,
                entry.edge_delta,
            ])
            .context("insert adaptive learning history entry")?;
        }
        drop(stmt);
        tx.commit()
            .context("commit adaptive learning history transaction")?;
        Ok(())
    }

    pub fn list_adaptive_learning_history(
        &self,
        query: &AdaptiveLearningHistoryQuery,
    ) -> Result<Vec<StoredAdaptiveLearningHistoryEntry>> {
        let limit = query.limit.max(1) as i64;
        let offset = query.offset as i64;
        let mut stmt = match self.conn.prepare(
            "SELECT entry_id, artifact_kind, artifact_id, artifact_label, incident_id,
                    cause_type, hypothesis_id, observed_at, score, rank,
                    estimated_impact, impact_metric, score_delta, rank_delta, edge_delta
             FROM adaptive_learning_history
             WHERE (?1 IS NULL OR artifact_kind = ?1)
               AND (?2 IS NULL OR artifact_id = ?2)
               AND (?3 IS NULL OR incident_id = ?3)
               AND (?4 IS NULL OR cause_type = ?4)
             ORDER BY observed_at DESC, entry_id DESC
             LIMIT ?5 OFFSET ?6",
        ) {
            Ok(stmt) => stmt,
            Err(error) if is_missing_table_error(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error).context("prepare adaptive learning history query"),
        };
        let rows = match stmt.query_map(
            rusqlite::params![
                query.artifact_kind,
                query.artifact_id,
                query.incident_id,
                query.cause_type,
                limit,
                offset,
            ],
            |row| {
                Ok(StoredAdaptiveLearningHistoryEntry {
                    entry_id: row.get(0)?,
                    artifact_kind: row.get(1)?,
                    artifact_id: row.get(2)?,
                    artifact_label: row.get(3)?,
                    incident_id: row.get(4)?,
                    cause_type: row.get(5)?,
                    hypothesis_id: row.get(6)?,
                    observed_at: row.get(7)?,
                    score: row.get(8)?,
                    rank: row.get(9)?,
                    estimated_impact: row.get(10)?,
                    impact_metric: row.get(11)?,
                    score_delta: row.get(12)?,
                    rank_delta: row.get(13)?,
                    edge_delta: row.get(14)?,
                })
            },
        ) {
            Ok(rows) => rows,
            Err(error) if is_missing_table_error(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error).context("query adaptive learning history"),
        };
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn adaptive_learning_model(&self) -> Result<Option<StoredAdaptiveLearningModel>> {
        let meta = match self
            .conn
            .query_row(
                "SELECT schema_version, last_updated
                 FROM adaptive_learning_registry_meta
                 WHERE singleton_id = 1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
        {
            Ok(value) => value,
            Err(error) if is_missing_table_error(&error) => return Ok(None),
            Err(error) => return Err(error).context("query adaptive registry meta"),
        };

        let processed_feedback_ids = {
            let mut stmt = match self.conn.prepare(
                "SELECT feedback_id
                 FROM adaptive_learning_processed_feedback
                 ORDER BY feedback_id ASC",
            ) {
                Ok(stmt) => stmt,
                Err(error) if is_missing_table_error(&error) => return Ok(None),
                Err(error) => {
                    return Err(error).context("prepare adaptive processed feedback query")
                }
            };
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .context("query adaptive processed feedback")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };

        let learned_detectors = {
            let mut stmt = match self.conn.prepare(
                "SELECT detector_id, requirement_name, cause_type, positive_terms, tags, source_types,
                        min_severity, confirmations, false_positives, created_from_feedback_id,
                        updated_at, manually_disabled, status_reason, review_status, review_reason,
                        last_reviewed_at
                 FROM adaptive_learned_detectors
                 ORDER BY requirement_name ASC, detector_id ASC",
            ) {
                Ok(stmt) => stmt,
                Err(error) if is_missing_table_error(&error) => return Ok(None),
                Err(error) => return Err(error).context("prepare adaptive detectors query"),
            };
            let rows = stmt
                .query_map([], |row| {
                    Ok(StoredLearnedDetector {
                        detector_id: row.get(0)?,
                        requirement_name: row.get(1)?,
                        cause_type: row.get(2)?,
                        positive_terms: parse_json_array(row.get(3)?),
                        tags: parse_json_array(row.get(4)?),
                        source_types: parse_json_array(row.get(5)?),
                        min_severity: row.get(6)?,
                        confirmations: row.get(7)?,
                        false_positives: row.get(8)?,
                        created_from_feedback_id: row.get(9)?,
                        updated_at: row.get(10)?,
                        manually_disabled: row.get::<_, i64>(11)? != 0,
                        status_reason: row.get(12)?,
                        review_status: row.get(13)?,
                        review_reason: row.get(14)?,
                        last_reviewed_at: row.get(15)?,
                    })
                })
                .context("query adaptive detectors")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };

        let learned_templates = {
            let mut stmt = match self.conn.prepare(
                "SELECT template_id, template_name, cause_type, cause_subtype, title_template,
                        confidence, requires_json, requires_same_service, requires_temporal_order,
                        confirmations, false_positives, created_from_feedback_id, updated_at,
                        manually_disabled, status_reason, review_status, review_reason,
                        last_reviewed_at
                 FROM adaptive_learned_templates
                 ORDER BY template_name ASC, template_id ASC",
            ) {
                Ok(stmt) => stmt,
                Err(error) if is_missing_table_error(&error) => return Ok(None),
                Err(error) => return Err(error).context("prepare adaptive templates query"),
            };
            let rows = stmt
                .query_map([], |row| {
                    Ok(StoredLearnedTemplate {
                        template_id: row.get(0)?,
                        template_name: row.get(1)?,
                        cause_type: row.get(2)?,
                        cause_subtype: row.get(3)?,
                        title_template: row.get(4)?,
                        confidence: row.get(5)?,
                        requires: parse_json_array(row.get(6)?),
                        requires_same_service: row.get::<_, i64>(7)? != 0,
                        requires_temporal_order: row.get::<_, i64>(8)? != 0,
                        confirmations: row.get(9)?,
                        false_positives: row.get(10)?,
                        created_from_feedback_id: row.get(11)?,
                        updated_at: row.get(12)?,
                        manually_disabled: row.get::<_, i64>(13)? != 0,
                        status_reason: row.get(14)?,
                        review_status: row.get(15)?,
                        review_reason: row.get(16)?,
                        last_reviewed_at: row.get(17)?,
                    })
                })
                .context("query adaptive templates")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };

        let learned_compositions = {
            let mut stmt = match self.conn.prepare(
                "SELECT composition_id, composition_name, cause_type, cause_subtype, title_template,
                        confidence, requires_json, requires_same_service, requires_temporal_order,
                        preferred_edge_types, confirmations, false_positives,
                        created_from_feedback_id, updated_at, manually_disabled, status_reason,
                        review_status, review_reason, last_reviewed_at
                 FROM adaptive_learned_compositions
                 ORDER BY composition_name ASC, composition_id ASC",
            ) {
                Ok(stmt) => stmt,
                Err(error) if is_missing_table_error(&error) => return Ok(None),
                Err(error) => return Err(error).context("prepare adaptive compositions query"),
            };
            let rows = stmt
                .query_map([], |row| {
                    Ok(StoredLearnedComposition {
                        composition_id: row.get(0)?,
                        composition_name: row.get(1)?,
                        cause_type: row.get(2)?,
                        cause_subtype: row.get(3)?,
                        title_template: row.get(4)?,
                        confidence: row.get(5)?,
                        requires: parse_json_array(row.get(6)?),
                        requires_same_service: row.get::<_, i64>(7)? != 0,
                        requires_temporal_order: row.get::<_, i64>(8)? != 0,
                        preferred_edge_types: parse_json_array(row.get(9)?),
                        confirmations: row.get(10)?,
                        false_positives: row.get(11)?,
                        created_from_feedback_id: row.get(12)?,
                        updated_at: row.get(13)?,
                        manually_disabled: row.get::<_, i64>(14)? != 0,
                        status_reason: row.get(15)?,
                        review_status: row.get(16)?,
                        review_reason: row.get(17)?,
                        last_reviewed_at: row.get(18)?,
                    })
                })
                .context("query adaptive compositions")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };

        let learned_edge_profiles = {
            let mut stmt = match self.conn.prepare(
                "SELECT profile_id, edge_type, source_service, target_service, cause_type,
                        confirmations, false_positives, average_plausibility,
                        average_latency_ms, created_from_feedback_id, updated_at,
                        manually_disabled, status_reason, review_status, review_reason,
                        last_reviewed_at
                 FROM adaptive_learned_edge_profiles
                 ORDER BY profile_id ASC",
            ) {
                Ok(stmt) => stmt,
                Err(error) if is_missing_table_error(&error) => return Ok(None),
                Err(error) => return Err(error).context("prepare adaptive edge profiles query"),
            };
            let rows = stmt
                .query_map([], |row| {
                    Ok(StoredLearnedEdgeProfile {
                        profile_id: row.get(0)?,
                        edge_type: row.get(1)?,
                        source_service: row.get(2)?,
                        target_service: row.get(3)?,
                        cause_type: row.get(4)?,
                        confirmations: row.get(5)?,
                        false_positives: row.get(6)?,
                        average_plausibility: row.get(7)?,
                        average_latency_ms: row.get(8)?,
                        created_from_feedback_id: row.get(9)?,
                        updated_at: row.get(10)?,
                        manually_disabled: row.get::<_, i64>(11)? != 0,
                        status_reason: row.get(12)?,
                        review_status: row.get(13)?,
                        review_reason: row.get(14)?,
                        last_reviewed_at: row.get(15)?,
                    })
                })
                .context("query adaptive edge profiles")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            out
        };

        if meta.is_none()
            && processed_feedback_ids.is_empty()
            && learned_detectors.is_empty()
            && learned_templates.is_empty()
            && learned_compositions.is_empty()
            && learned_edge_profiles.is_empty()
        {
            return Ok(None);
        }

        let (schema_version, last_updated) = meta.unwrap_or((1, None));
        Ok(Some(StoredAdaptiveLearningModel {
            schema_version,
            last_updated,
            processed_feedback_ids,
            learned_detectors,
            learned_templates,
            learned_compositions,
            learned_edge_profiles,
        }))
    }

    pub fn replace_adaptive_learning_model(
        &mut self,
        model: &StoredAdaptiveLearningModel,
    ) -> Result<()> {
        if !table_exists(&self.conn, "adaptive_learning_registry_meta")?
            || !table_exists(&self.conn, "adaptive_learned_detectors")?
        {
            return Ok(());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .context("begin adaptive registry transaction")?;
        tx.execute(
            "INSERT INTO adaptive_learning_registry_meta (singleton_id, schema_version, last_updated)
             VALUES (1, ?1, ?2)
             ON CONFLICT(singleton_id) DO UPDATE SET
                schema_version = excluded.schema_version,
                last_updated = excluded.last_updated",
            rusqlite::params![model.schema_version, model.last_updated],
        )
        .context("upsert adaptive registry meta")?;
        tx.execute("DELETE FROM adaptive_learning_processed_feedback", [])
            .context("clear adaptive processed feedback")?;
        tx.execute("DELETE FROM adaptive_learned_detectors", [])
            .context("clear adaptive detectors")?;
        tx.execute("DELETE FROM adaptive_learned_templates", [])
            .context("clear adaptive templates")?;
        tx.execute("DELETE FROM adaptive_learned_compositions", [])
            .context("clear adaptive compositions")?;
        tx.execute("DELETE FROM adaptive_learned_edge_profiles", [])
            .context("clear adaptive edge profiles")?;

        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO adaptive_learning_processed_feedback (
                        feedback_id, processed_at
                    ) VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                )
                .context("prepare adaptive processed feedback insert")?;
            for feedback_id in &model.processed_feedback_ids {
                stmt.execute(rusqlite::params![feedback_id])
                    .context("insert adaptive processed feedback")?;
            }
        }

        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO adaptive_learned_detectors (
                        detector_id, requirement_name, cause_type, positive_terms, tags, source_types,
                        min_severity, confirmations, false_positives, created_from_feedback_id,
                        updated_at, manually_disabled, status_reason, review_status, review_reason,
                        last_reviewed_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6,
                        ?7, ?8, ?9, ?10,
                        ?11, ?12, ?13, ?14, ?15,
                        ?16
                    )",
                )
                .context("prepare adaptive detector insert")?;
            for item in &model.learned_detectors {
                stmt.execute(rusqlite::params![
                    item.detector_id,
                    item.requirement_name,
                    item.cause_type,
                    serde_json::to_string(&item.positive_terms).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&item.tags).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&item.source_types).unwrap_or_else(|_| "[]".into()),
                    item.min_severity,
                    item.confirmations,
                    item.false_positives,
                    item.created_from_feedback_id,
                    item.updated_at,
                    if item.manually_disabled { 1 } else { 0 },
                    item.status_reason,
                    item.review_status,
                    item.review_reason,
                    item.last_reviewed_at,
                ])
                .context("insert adaptive detector")?;
            }
        }

        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO adaptive_learned_templates (
                        template_id, template_name, cause_type, cause_subtype, title_template,
                        confidence, requires_json, requires_same_service, requires_temporal_order,
                        confirmations, false_positives, created_from_feedback_id, updated_at,
                        manually_disabled, status_reason, review_status, review_reason,
                        last_reviewed_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5,
                        ?6, ?7, ?8, ?9,
                        ?10, ?11, ?12, ?13,
                        ?14, ?15, ?16, ?17,
                        ?18
                    )",
                )
                .context("prepare adaptive template insert")?;
            for item in &model.learned_templates {
                stmt.execute(rusqlite::params![
                    item.template_id,
                    item.template_name,
                    item.cause_type,
                    item.cause_subtype,
                    item.title_template,
                    item.confidence,
                    serde_json::to_string(&item.requires).unwrap_or_else(|_| "[]".into()),
                    if item.requires_same_service { 1 } else { 0 },
                    if item.requires_temporal_order { 1 } else { 0 },
                    item.confirmations,
                    item.false_positives,
                    item.created_from_feedback_id,
                    item.updated_at,
                    if item.manually_disabled { 1 } else { 0 },
                    item.status_reason,
                    item.review_status,
                    item.review_reason,
                    item.last_reviewed_at,
                ])
                .context("insert adaptive template")?;
            }
        }

        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO adaptive_learned_compositions (
                        composition_id, composition_name, cause_type, cause_subtype, title_template,
                        confidence, requires_json, requires_same_service, requires_temporal_order,
                        preferred_edge_types, confirmations, false_positives,
                        created_from_feedback_id, updated_at, manually_disabled, status_reason,
                        review_status, review_reason, last_reviewed_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5,
                        ?6, ?7, ?8, ?9,
                        ?10, ?11, ?12,
                        ?13, ?14, ?15, ?16,
                        ?17, ?18, ?19
                    )",
                )
                .context("prepare adaptive composition insert")?;
            for item in &model.learned_compositions {
                stmt.execute(rusqlite::params![
                    item.composition_id,
                    item.composition_name,
                    item.cause_type,
                    item.cause_subtype,
                    item.title_template,
                    item.confidence,
                    serde_json::to_string(&item.requires).unwrap_or_else(|_| "[]".into()),
                    if item.requires_same_service { 1 } else { 0 },
                    if item.requires_temporal_order { 1 } else { 0 },
                    serde_json::to_string(&item.preferred_edge_types)
                        .unwrap_or_else(|_| "[]".into()),
                    item.confirmations,
                    item.false_positives,
                    item.created_from_feedback_id,
                    item.updated_at,
                    if item.manually_disabled { 1 } else { 0 },
                    item.status_reason,
                    item.review_status,
                    item.review_reason,
                    item.last_reviewed_at,
                ])
                .context("insert adaptive composition")?;
            }
        }

        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO adaptive_learned_edge_profiles (
                        profile_id, edge_type, source_service, target_service, cause_type,
                        confirmations, false_positives, average_plausibility,
                        average_latency_ms, created_from_feedback_id, updated_at,
                        manually_disabled, status_reason, review_status, review_reason,
                        last_reviewed_at
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5,
                        ?6, ?7, ?8,
                        ?9, ?10, ?11,
                        ?12, ?13, ?14, ?15,
                        ?16
                    )",
                )
                .context("prepare adaptive edge profile insert")?;
            for item in &model.learned_edge_profiles {
                stmt.execute(rusqlite::params![
                    item.profile_id,
                    item.edge_type,
                    item.source_service,
                    item.target_service,
                    item.cause_type,
                    item.confirmations,
                    item.false_positives,
                    item.average_plausibility,
                    item.average_latency_ms,
                    item.created_from_feedback_id,
                    item.updated_at,
                    if item.manually_disabled { 1 } else { 0 },
                    item.status_reason,
                    item.review_status,
                    item.review_reason,
                    item.last_reviewed_at,
                ])
                .context("insert adaptive edge profile")?;
            }
        }

        tx.commit()
            .context("commit adaptive registry transaction")?;
        Ok(())
    }

    pub fn list_adaptive_review_views(&self) -> Result<Vec<StoredAdaptiveReviewView>> {
        let mut stmt = match self.conn.prepare(
            "SELECT view_id, name, description, search_text, assigned_reviewer,
                    created_at, updated_at, last_used_at
             FROM adaptive_review_views
             ORDER BY COALESCE(last_used_at, updated_at) DESC, name ASC",
        ) {
            Ok(stmt) => stmt,
            Err(error) if is_missing_table_error(&error) => return Ok(Vec::new()),
            Err(error) => return Err(error).context("prepare adaptive review views query"),
        };
        let rows = stmt
            .query_map([], |row| {
                Ok(StoredAdaptiveReviewView {
                    view_id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    search_text: row.get(3)?,
                    assigned_reviewer: row.get(4)?,
                    artifact_selections: Vec::new(),
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    last_used_at: row.get(7)?,
                })
            })
            .context("query adaptive review views")?;
        let mut views = Vec::new();
        for row in rows {
            views.push(row?);
        }
        if views.is_empty() {
            return Ok(Vec::new());
        }
        let mut selections_by_view =
            HashMap::<String, Vec<StoredAdaptiveReviewViewSelection>>::new();
        let mut selection_stmt = match self.conn.prepare(
            "SELECT view_id, artifact_kind, artifact_id
             FROM adaptive_review_view_artifacts
             ORDER BY view_id ASC, artifact_kind ASC, artifact_id ASC",
        ) {
            Ok(stmt) => stmt,
            Err(error) if is_missing_table_error(&error) => return Ok(views),
            Err(error) => {
                return Err(error).context("prepare adaptive review view artifacts query")
            }
        };
        let selection_rows = selection_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    StoredAdaptiveReviewViewSelection {
                        artifact_kind: row.get(1)?,
                        artifact_id: row.get(2)?,
                    },
                ))
            })
            .context("query adaptive review view artifacts")?;
        for row in selection_rows {
            let (view_id, selection) = row?;
            selections_by_view
                .entry(view_id)
                .or_default()
                .push(selection);
        }
        for view in &mut views {
            view.artifact_selections = selections_by_view.remove(&view.view_id).unwrap_or_default();
        }
        Ok(views)
    }

    pub fn upsert_adaptive_review_view(&mut self, view: &StoredAdaptiveReviewView) -> Result<()> {
        if !table_exists(&self.conn, "adaptive_review_views")? {
            return Ok(());
        }
        let tx = self
            .conn
            .unchecked_transaction()
            .context("begin adaptive review view transaction")?;
        tx.execute(
            "INSERT INTO adaptive_review_views (
                view_id, name, description, search_text, assigned_reviewer,
                created_at, updated_at, last_used_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5,
                ?6, ?7, ?8
            )
            ON CONFLICT(view_id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                search_text = excluded.search_text,
                assigned_reviewer = excluded.assigned_reviewer,
                updated_at = excluded.updated_at,
                last_used_at = excluded.last_used_at",
            rusqlite::params![
                view.view_id,
                view.name,
                view.description,
                view.search_text,
                view.assigned_reviewer,
                view.created_at,
                view.updated_at,
                view.last_used_at,
            ],
        )
        .context("upsert adaptive review view")?;
        tx.execute(
            "DELETE FROM adaptive_review_view_artifacts WHERE view_id = ?1",
            rusqlite::params![view.view_id],
        )
        .context("clear adaptive review view artifacts")?;
        let mut stmt = tx
            .prepare(
                "INSERT INTO adaptive_review_view_artifacts (
                    view_id, artifact_kind, artifact_id
                ) VALUES (?1, ?2, ?3)",
            )
            .context("prepare adaptive review view artifact insert")?;
        for selection in &view.artifact_selections {
            stmt.execute(rusqlite::params![
                view.view_id,
                selection.artifact_kind,
                selection.artifact_id,
            ])
            .context("insert adaptive review view artifact")?;
        }
        drop(stmt);
        tx.commit()
            .context("commit adaptive review view transaction")?;
        Ok(())
    }

    pub fn delete_adaptive_review_view(&self, view_id: &str) -> Result<()> {
        if !table_exists(&self.conn, "adaptive_review_views")? {
            return Ok(());
        }
        self.conn
            .execute(
                "DELETE FROM adaptive_review_view_artifacts WHERE view_id = ?1",
                rusqlite::params![view_id],
            )
            .context("delete adaptive review view artifacts")?;
        self.conn
            .execute(
                "DELETE FROM adaptive_review_views WHERE view_id = ?1",
                rusqlite::params![view_id],
            )
            .context("delete adaptive review view")?;
        Ok(())
    }

    pub fn touch_adaptive_review_view(&self, view_id: &str, used_at: &str) -> Result<()> {
        if !table_exists(&self.conn, "adaptive_review_views")? {
            return Ok(());
        }
        self.conn
            .execute(
                "UPDATE adaptive_review_views
                 SET last_used_at = ?2, updated_at = CASE WHEN updated_at > ?2 THEN updated_at ELSE ?2 END
                 WHERE view_id = ?1",
                rusqlite::params![view_id, used_at],
            )
            .context("touch adaptive review view")?;
        Ok(())
    }

    pub fn add_ai_trace(&self, trace: &StoredAiTrace) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO incident_ai_traces (
                    trace_id, incident_id, trace_kind, sanitized_system_prompt, sanitized_user_prompt,
                    allowed_fields, blocked_fields, raw_logs_sent, trace_schema_version, created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10
                )",
                rusqlite::params![
                    trace.trace_id,
                    trace.incident_id,
                    trace.trace_kind,
                    trace.sanitized_system_prompt,
                    trace.sanitized_user_prompt,
                    serde_json::to_string(&trace.allowed_fields).unwrap_or_else(|_| "[]".into()),
                    serde_json::to_string(&trace.blocked_fields).unwrap_or_else(|_| "[]".into()),
                    if trace.raw_logs_sent { 1 } else { 0 },
                    trace.trace_schema_version,
                    trace.created_at,
                ],
            )
            .context("insert ai trace")?;
        Ok(())
    }

    pub fn add_chat_message(&self, message: &StoredChatMessage) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO incident_chat_messages (
                    message_id, incident_id, role, content, message_schema_version, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    message.message_id,
                    message.incident_id,
                    message.role,
                    message.content,
                    message.message_schema_version,
                    message.created_at,
                ],
            )
            .context("insert incident chat message")?;
        Ok(())
    }

    pub fn list_chat_messages(&self, incident_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT message_id, role, content, message_schema_version, created_at
                 FROM incident_chat_messages
                 WHERE incident_id = ?1 ORDER BY created_at ASC",
            )
            .context("prepare incident chat query")?;
        let rows = stmt
            .query_map(rusqlite::params![incident_id], |row| {
                Ok(serde_json::json!({
                    "message_id": row.get::<_, String>(0)?,
                    "role": row.get::<_, String>(1)?,
                    "content": row.get::<_, String>(2)?,
                    "message_schema_version": row.get::<_, i64>(3)?,
                    "created_at": row.get::<_, String>(4)?,
                }))
            })
            .context("query incident chat messages")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn latest_ai_trace(&self, incident_id: &str) -> Result<Option<Value>> {
        self.conn
            .query_row(
                "SELECT trace_id, trace_kind, sanitized_system_prompt, sanitized_user_prompt,
                        allowed_fields, blocked_fields, raw_logs_sent, trace_schema_version, created_at
                 FROM incident_ai_traces
                 WHERE incident_id = ?1 ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![incident_id],
                |row| {
                    Ok(serde_json::json!({
                        "trace_id": row.get::<_, String>(0)?,
                        "trace_kind": row.get::<_, String>(1)?,
                        "sanitized_system_prompt": row.get::<_, String>(2)?,
                        "sanitized_user_prompt": row.get::<_, String>(3)?,
                        "allowed_fields": row
                            .get::<_, String>(4)
                            .ok()
                            .map(parse_json_array)
                            .unwrap_or_default(),
                        "blocked_fields": row
                            .get::<_, String>(5)
                            .ok()
                            .map(parse_json_array)
                            .unwrap_or_default(),
                        "raw_logs_sent": row.get::<_, i64>(6)? != 0,
                        "trace_schema_version": row.get::<_, i64>(7)?,
                        "created_at": row.get::<_, String>(8)?,
                    }))
                },
            )
            .optional()
            .context("query latest ai trace")
    }

    pub fn add_ai_generation(&self, generation: &StoredAiGeneration) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO ai_generations (
                    generation_id, scope_key, focus, mode, question, response_json,
                    bundle_hash, used_ai, provider_json, created_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6,
                    ?7, ?8, ?9, ?10
                )",
                rusqlite::params![
                    generation.generation_id,
                    generation.scope_key,
                    generation.focus,
                    generation.mode,
                    generation.question,
                    generation.response.to_string(),
                    generation.bundle_hash,
                    if generation.used_ai { 1 } else { 0 },
                    generation.provider.to_string(),
                    generation.created_at,
                ],
            )
            .context("insert ai generation")?;
        Ok(())
    }

    pub fn latest_ai_generation(&self, scope_key: &str) -> Result<Option<Value>> {
        self.conn
            .query_row(
                "SELECT generation_id, scope_key, focus, mode, question, response_json,
                        bundle_hash, used_ai, provider_json, created_at
                 FROM ai_generations
                 WHERE scope_key = ?1
                 ORDER BY created_at DESC LIMIT 1",
                rusqlite::params![scope_key],
                ai_generation_json_from_row,
            )
            .optional()
            .context("query latest ai generation")
    }

    pub fn list_ai_generations(
        &self,
        scope_prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let limit = limit.clamp(1, 200);
        let (sql, params): (&str, Vec<rusqlite::types::Value>) =
            if let Some(prefix) = scope_prefix.filter(|value| !value.trim().is_empty()) {
                (
                    "SELECT generation_id, scope_key, focus, mode, question, response_json,
                        bundle_hash, used_ai, provider_json, created_at
                 FROM ai_generations
                 WHERE scope_key LIKE ?1 || '%'
                 ORDER BY created_at DESC LIMIT ?2",
                    vec![prefix.to_string().into(), (limit as i64).into()],
                )
            } else {
                (
                    "SELECT generation_id, scope_key, focus, mode, question, response_json,
                        bundle_hash, used_ai, provider_json, created_at
                 FROM ai_generations
                 ORDER BY created_at DESC LIMIT ?1",
                    vec![(limit as i64).into()],
                )
            };
        let mut stmt = self
            .conn
            .prepare(sql)
            .context("prepare ai generations list")?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter()),
                ai_generation_json_from_row,
            )
            .context("query ai generations list")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn record_state_log(
        &self,
        incident_id: &str,
        old_state: &str,
        new_state: &str,
        reason: &str,
        changed_at: Option<&str>,
    ) -> Result<()> {
        match self.conn.execute(
            "INSERT INTO incident_state_log (
                    incident_id, old_state, new_state, changed_at, reason
                ) VALUES (
                    ?1, ?2, ?3, COALESCE(?4, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), ?5
                )",
            rusqlite::params![incident_id, old_state, new_state, changed_at, reason],
        ) {
            Ok(_) => {}
            Err(error) if is_missing_table_error(&error) => return Ok(()),
            Err(error) => return Err(error).context("insert incident state log"),
        }
        Ok(())
    }

    pub fn list_state_log(&self, incident_id: &str) -> Result<Vec<Value>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT log_id, old_state, new_state, changed_at, reason
                 FROM incident_state_log WHERE incident_id = ?1 ORDER BY changed_at ASC",
            )
            .context("prepare state log query")?;
        let rows = stmt
            .query_map(rusqlite::params![incident_id], |row| {
                Ok(Value::Object(serde_json::Map::from_iter([
                    ("log_id".into(), Value::from(row.get::<_, i64>(0)?)),
                    ("old_state".into(), Value::from(row.get::<_, String>(1)?)),
                    ("new_state".into(), Value::from(row.get::<_, String>(2)?)),
                    ("changed_at".into(), Value::from(row.get::<_, String>(3)?)),
                    ("reason".into(), Value::from(row.get::<_, String>(4)?)),
                ])))
            })
            .context("query state log")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn transition_state(
        &self,
        incident_id: &str,
        new_state: &str,
        reason: &str,
        changed_at: &str,
    ) -> Result<()> {
        let old_state: Option<String> = self
            .conn
            .query_row(
                "SELECT state FROM incidents WHERE incident_id = ?1",
                rusqlite::params![incident_id],
                |row| row.get(0),
            )
            .optional()
            .context("load incident state before transition")?;
        let Some(old_state) = old_state else {
            anyhow::bail!("incident not found");
        };
        self.conn
            .execute(
                "UPDATE incidents SET state = ?2, updated_at = ?3 WHERE incident_id = ?1",
                rusqlite::params![incident_id, new_state, changed_at],
            )
            .context("update incident state")?;
        self.record_state_log(incident_id, &old_state, new_state, reason, Some(changed_at))?;
        Ok(())
    }

    pub fn resolve_incident(
        &self,
        incident_id: &str,
        resolution_info: &Value,
        resolved_at: &str,
    ) -> Result<()> {
        let old_state: Option<String> = self
            .conn
            .query_row(
                "SELECT state FROM incidents WHERE incident_id = ?1",
                rusqlite::params![incident_id],
                |row| row.get(0),
            )
            .optional()
            .context("load incident state before resolve")?;
        let Some(old_state) = old_state else {
            anyhow::bail!("incident not found");
        };
        self.conn
            .execute(
                "UPDATE incidents SET state = 'resolved', updated_at = ?2, resolution_info = ?3 WHERE incident_id = ?1",
                rusqlite::params![incident_id, resolved_at, resolution_info.to_string()],
            )
            .context("resolve incident")?;
        self.record_state_log(
            incident_id,
            &old_state,
            "resolved",
            "resolved",
            Some(resolved_at),
        )?;
        Ok(())
    }

    pub fn stale_incident_ids_before(&self, cutoff: &str, limit: usize) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT incident_id FROM incidents
                 WHERE state IN ('open','investigating','explained')
                   AND updated_at < ?1
                 ORDER BY updated_at ASC
                 LIMIT ?2",
            )
            .context("prepare stale incident query")?;
        let rows = stmt
            .query_map(rusqlite::params![cutoff, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .context("query stale incidents")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn archive_candidate_ids_before(&self, cutoff: &str, limit: usize) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT incident_id FROM incidents
                 WHERE state IN ('resolved','stale')
                   AND updated_at < ?1
                 ORDER BY updated_at ASC
                 LIMIT ?2",
            )
            .context("prepare archive candidate query")?;
        let rows = stmt
            .query_map(rusqlite::params![cutoff, limit as i64], |row| {
                row.get::<_, String>(0)
            })
            .context("query archive candidates")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn incident_archive_payload(&self, incident_id: &str) -> Result<Option<Value>> {
        let Some(incident) = self.get_incident(incident_id)? else {
            return Ok(None);
        };
        let event_ids = self.incident_event_ids(incident_id)?;
        let hypotheses = self.hypothesis_records(incident_id)?;
        let clusters = self.clusters(incident_id)?;
        let explanation = self.latest_explanation(incident_id)?;
        let latest_trace = self.latest_ai_trace(incident_id)?;
        let state_log = self.list_state_log(incident_id)?;
        let feedback = self.list_feedback(incident_id)?;
        let inference_graph = self.inference_graph_snapshot(incident_id)?;
        let mut chat_stmt = self
            .conn
            .prepare(
                "SELECT message_id, role, content, message_schema_version, created_at
                 FROM incident_chat_messages WHERE incident_id = ?1 ORDER BY created_at ASC",
            )
            .context("prepare archive chat query")?;
        let chat_rows = chat_stmt
            .query_map(rusqlite::params![incident_id], |row| {
                Ok(serde_json::json!({
                    "message_id": row.get::<_, String>(0)?,
                    "role": row.get::<_, String>(1)?,
                    "content": row.get::<_, String>(2)?,
                    "message_schema_version": row.get::<_, i64>(3)?,
                    "created_at": row.get::<_, String>(4)?,
                }))
            })
            .context("query archive chat rows")?;
        let mut chat_messages = Vec::new();
        for row in chat_rows {
            chat_messages.push(row?);
        }
        Ok(Some(serde_json::json!({
            "incident": incident,
            "event_ids": event_ids,
            "hypotheses": hypotheses,
            "clusters": clusters,
            "explanation": explanation,
            "latest_trace": latest_trace,
            "state_log": state_log,
            "feedback": feedback,
            "inference_graph": inference_graph,
            "chat_messages": chat_messages,
        })))
    }

    pub fn archive_incident_to_path(
        &mut self,
        incident_id: &str,
        archive_db_path: &Path,
        archived_at: &str,
    ) -> Result<bool> {
        let Some(payload) = self.incident_archive_payload(incident_id)? else {
            return Ok(false);
        };
        if let Some(parent) = archive_db_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create archive dir {}", parent.display()))?;
        }
        let archive_conn = Connection::open(archive_db_path)
            .with_context(|| format!("open archive db {}", archive_db_path.display()))?;
        archive_conn
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 CREATE TABLE IF NOT EXISTS archived_incidents (
                     incident_id TEXT PRIMARY KEY,
                     state TEXT NOT NULL,
                     archived_at TEXT NOT NULL,
                     payload_json TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_archived_state ON archived_incidents(state);
                 CREATE INDEX IF NOT EXISTS idx_archived_at ON archived_incidents(archived_at);",
            )
            .context("initialize archive database")?;
        let state = payload
            .get("incident")
            .and_then(|value| value.get("state"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        archive_conn
            .execute(
                "INSERT INTO archived_incidents (incident_id, state, archived_at, payload_json)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(incident_id) DO UPDATE SET
                    state = excluded.state,
                    archived_at = excluded.archived_at,
                    payload_json = excluded.payload_json",
                rusqlite::params![incident_id, state, archived_at, payload.to_string()],
            )
            .context("insert archived incident")?;
        let tx = self
            .conn
            .unchecked_transaction()
            .context("begin archive delete transaction")?;
        for table in [
            "incident_events",
            "hypotheses",
            "incident_clusters",
            "explanations",
            "incident_ai_traces",
            "inference_graph_snapshots",
            "feedback",
            "incident_state_log",
            "incident_chat_messages",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE incident_id = ?1"),
                rusqlite::params![incident_id],
            )
            .with_context(|| format!("delete archived incident rows from {table}"))?;
        }
        tx.execute(
            "DELETE FROM incidents WHERE incident_id = ?1",
            rusqlite::params![incident_id],
        )
        .context("delete archived incident row")?;
        tx.commit().context("commit archive delete transaction")?;
        Ok(true)
    }
}

fn initialize_events_db(path: &Path) -> Result<()> {
    let conn = Connection::open(path)
        .with_context(|| format!("open events db for initialization {}", path.display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS _schema_version (
             schema_name TEXT PRIMARY KEY,
             version INTEGER NOT NULL,
             applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS schema_version (
             name TEXT PRIMARY KEY,
             version INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS events (
             event_id TEXT PRIMARY KEY,
             timestamp TEXT NOT NULL,
             timestamp_source TEXT NOT NULL DEFAULT 'collector',
             service_id TEXT NOT NULL DEFAULT '',
             host_id TEXT NOT NULL DEFAULT 'local',
             severity INTEGER NOT NULL DEFAULT 0,
             event_type INTEGER NOT NULL DEFAULT 0,
             message TEXT NOT NULL DEFAULT '',
             structured_data TEXT,
             tags TEXT,
             fingerprint TEXT NOT NULL DEFAULT '',
             quality TEXT,
             source_type TEXT NOT NULL DEFAULT 'runtime',
             source_id TEXT NOT NULL DEFAULT '',
             raw_offset INTEGER,
             collected_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             schema_version INTEGER NOT NULL DEFAULT 1,
             inserted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS collector_state (
             collector_id TEXT NOT NULL,
             state_key TEXT NOT NULL,
             state_value TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             PRIMARY KEY (collector_id, state_key)
         );
         CREATE TABLE IF NOT EXISTS raw_events (
             raw_event_id TEXT PRIMARY KEY,
             event_id TEXT,
             raw_payload TEXT NOT NULL DEFAULT '',
             source_type TEXT NOT NULL DEFAULT 'runtime',
             source_id TEXT NOT NULL DEFAULT '',
             collected_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             inserted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS fingerprint_seen (
             fingerprint TEXT PRIMARY KEY,
             first_seen_at TEXT NOT NULL,
             last_seen_at TEXT NOT NULL,
             hit_count INTEGER NOT NULL DEFAULT 1
         );
         CREATE TABLE IF NOT EXISTS dedup_window (
             fingerprint TEXT PRIMARY KEY,
             first_event_id TEXT NOT NULL DEFAULT '',
             last_event_id TEXT NOT NULL DEFAULT '',
             count INTEGER NOT NULL DEFAULT 1,
             window_start TEXT NOT NULL DEFAULT '',
             window_end TEXT NOT NULL DEFAULT '',
             suppressed_count INTEGER NOT NULL DEFAULT 0
         );",
    )
    .context("initialize events db schema")?;
    ensure_column(
        &conn,
        "events",
        "timestamp_source",
        "TEXT NOT NULL DEFAULT 'collector'",
    )?;
    ensure_column(&conn, "events", "host_id", "TEXT NOT NULL DEFAULT 'local'")?;
    ensure_column(&conn, "events", "event_type", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(&conn, "events", "structured_data", "TEXT")?;
    ensure_column(&conn, "events", "fingerprint", "TEXT NOT NULL DEFAULT ''")?;
    ensure_column(&conn, "events", "quality", "TEXT")?;
    ensure_column(&conn, "events", "source_id", "TEXT NOT NULL DEFAULT ''")?;
    ensure_column(&conn, "events", "raw_offset", "INTEGER")?;
    ensure_column(&conn, "events", "collected_at", "TEXT NOT NULL DEFAULT ''")?;
    ensure_column(
        &conn,
        "events",
        "schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    ensure_column(&conn, "events", "inserted_at", "TEXT NOT NULL DEFAULT ''")?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
         CREATE INDEX IF NOT EXISTS idx_events_service_ts ON events(service_id, timestamp);
         CREATE INDEX IF NOT EXISTS idx_events_severity_ts ON events(severity, timestamp);
         CREATE INDEX IF NOT EXISTS idx_events_fingerprint ON events(fingerprint);
         CREATE INDEX IF NOT EXISTS idx_events_inserted ON events(inserted_at);
         CREATE INDEX IF NOT EXISTS idx_events_service_severity_ts ON events(service_id, severity, timestamp);
         CREATE INDEX IF NOT EXISTS idx_events_type_ts ON events(event_type, timestamp);
         CREATE INDEX IF NOT EXISTS idx_events_host_ts ON events(host_id, timestamp);
         CREATE INDEX IF NOT EXISTS idx_collector_state_updated ON collector_state(updated_at);
         CREATE INDEX IF NOT EXISTS idx_raw_events_event_id ON raw_events(event_id);
         CREATE INDEX IF NOT EXISTS idx_raw_events_inserted ON raw_events(inserted_at);
         CREATE INDEX IF NOT EXISTS idx_dedup_window_end ON dedup_window(window_end);",
    )
    .context("initialize events db indexes")?;
    conn.execute(
        "INSERT INTO _schema_version(schema_name, version) VALUES ('events', 5)
         ON CONFLICT(schema_name) DO UPDATE SET version = excluded.version, applied_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        [],
    )
    .context("update events _schema_version")?;
    conn.execute(
        "INSERT INTO schema_version(name, version) VALUES ('events', 5)
         ON CONFLICT(name) DO UPDATE SET version = excluded.version",
        [],
    )
    .context("update events schema_version")?;
    Ok(())
}

fn initialize_incidents_db(path: &Path) -> Result<()> {
    let conn = Connection::open(path)
        .with_context(|| format!("open incidents db for initialization {}", path.display()))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS _schema_version (
             schema_name TEXT PRIMARY KEY,
             version INTEGER NOT NULL,
             applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS schema_version (
             name TEXT PRIMARY KEY,
             version INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS incidents (
             incident_id TEXT PRIMARY KEY,
             state TEXT NOT NULL DEFAULT 'open',
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             severity INTEGER NOT NULL DEFAULT 0,
             primary_service TEXT,
             affected_services TEXT NOT NULL DEFAULT '[]',
             time_range_start TEXT NOT NULL DEFAULT '',
             time_range_end TEXT NOT NULL DEFAULT '',
             event_count INTEGER NOT NULL DEFAULT 0,
             schema_version INTEGER NOT NULL DEFAULT 1,
             cluster_ids TEXT NOT NULL DEFAULT '[]',
             runtime_context TEXT,
             resolution_info TEXT
         );
         CREATE TABLE IF NOT EXISTS incident_events (
             incident_id TEXT NOT NULL,
             event_id TEXT NOT NULL,
             added_at TEXT NOT NULL,
             PRIMARY KEY (incident_id, event_id)
         );
         CREATE TABLE IF NOT EXISTS hypotheses (
             hypothesis_id TEXT PRIMARY KEY,
             incident_id TEXT NOT NULL,
             rank INTEGER,
             cause_type TEXT,
             description TEXT NOT NULL DEFAULT '',
             total_score REAL,
             score_breakdown TEXT NOT NULL DEFAULT '{}',
             supporting_events TEXT NOT NULL DEFAULT '[]',
             contradicting_events TEXT NOT NULL DEFAULT '[]',
             affected_services TEXT NOT NULL DEFAULT '[]',
             confidence_label TEXT,
             suggested_checks TEXT NOT NULL DEFAULT '[]',
             is_valid INTEGER NOT NULL DEFAULT 1,
             invalidation_reasons TEXT NOT NULL DEFAULT '[]',
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS incident_clusters (
             incident_id TEXT NOT NULL,
             cluster_id TEXT NOT NULL,
             cluster_data TEXT NOT NULL DEFAULT '{}',
             PRIMARY KEY (incident_id, cluster_id)
         );
         CREATE TABLE IF NOT EXISTS explanations (
             explanation_id TEXT PRIMARY KEY,
             incident_id TEXT NOT NULL,
             summary TEXT NOT NULL DEFAULT '',
             primary_text TEXT NOT NULL DEFAULT '',
             evidence_text TEXT,
             timeline_text TEXT,
             alternatives TEXT NOT NULL DEFAULT '[]',
             actions TEXT NOT NULL DEFAULT '[]',
             uncertainty TEXT NOT NULL DEFAULT '[]',
             model_used TEXT NOT NULL DEFAULT '',
             guardrail_flags TEXT NOT NULL DEFAULT '[]',
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             explanation_schema_version INTEGER NOT NULL DEFAULT 1,
             hypotheses_hash TEXT NOT NULL DEFAULT '',
             events_hash_head TEXT NOT NULL DEFAULT '',
             quality TEXT NOT NULL DEFAULT 'ok'
         );
         CREATE TABLE IF NOT EXISTS inference_graph_snapshots (
             incident_id TEXT PRIMARY KEY,
             graph_data TEXT NOT NULL DEFAULT '{}',
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             event_count INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE IF NOT EXISTS ui_snapshots (
             data_type TEXT PRIMARY KEY,
             payload_json TEXT NOT NULL DEFAULT '{}',
             source TEXT NOT NULL DEFAULT 'inferra_core',
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             schema_version INTEGER NOT NULL DEFAULT 1,
             interval_seconds INTEGER
         );
         CREATE TABLE IF NOT EXISTS feedback (
             feedback_id TEXT PRIMARY KEY,
             incident_id TEXT NOT NULL,
             correct_hypothesis_id TEXT,
             feedback_type TEXT NOT NULL,
             operator_notes TEXT NOT NULL DEFAULT '',
             resolved_at TEXT NOT NULL,
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS incident_state_log (
             log_id INTEGER PRIMARY KEY AUTOINCREMENT,
             incident_id TEXT NOT NULL,
             old_state TEXT NOT NULL,
             new_state TEXT NOT NULL,
             changed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             reason TEXT NOT NULL DEFAULT ''
         );
         CREATE TABLE IF NOT EXISTS incident_ai_traces (
             trace_id TEXT PRIMARY KEY,
             incident_id TEXT NOT NULL,
             trace_kind TEXT NOT NULL,
             sanitized_system_prompt TEXT NOT NULL,
             sanitized_user_prompt TEXT NOT NULL,
             allowed_fields TEXT NOT NULL,
             blocked_fields TEXT NOT NULL,
             raw_logs_sent INTEGER NOT NULL DEFAULT 0,
             trace_schema_version INTEGER NOT NULL DEFAULT 1,
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS incident_chat_messages (
             message_id TEXT PRIMARY KEY,
             incident_id TEXT NOT NULL,
             role TEXT NOT NULL,
             content TEXT NOT NULL,
             message_schema_version INTEGER NOT NULL DEFAULT 1,
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS ai_generations (
             generation_id TEXT PRIMARY KEY,
             scope_key TEXT NOT NULL,
             focus TEXT NOT NULL,
             mode TEXT NOT NULL,
             question TEXT NOT NULL DEFAULT '',
             response_json TEXT NOT NULL DEFAULT '{}',
             bundle_hash TEXT NOT NULL DEFAULT '',
             used_ai INTEGER NOT NULL DEFAULT 0,
             provider_json TEXT NOT NULL DEFAULT '{}',
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS adaptive_learning_audit (
             audit_id TEXT PRIMARY KEY,
             artifact_kind TEXT NOT NULL,
             artifact_id TEXT NOT NULL,
             action TEXT NOT NULL,
             reason TEXT,
             previous_status TEXT NOT NULL,
             new_status TEXT NOT NULL,
             review_status_before TEXT,
             review_status_after TEXT,
             runtime_effect TEXT,
             created_at TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS adaptive_learning_history (
             entry_id TEXT PRIMARY KEY,
             artifact_kind TEXT NOT NULL,
             artifact_id TEXT NOT NULL,
             artifact_label TEXT NOT NULL,
             incident_id TEXT NOT NULL,
             cause_type TEXT NOT NULL,
             hypothesis_id TEXT NOT NULL,
             observed_at TEXT NOT NULL,
             score REAL,
             rank INTEGER,
             estimated_impact REAL NOT NULL DEFAULT 0,
             impact_metric TEXT,
             score_delta REAL,
             rank_delta INTEGER,
             edge_delta REAL
         );
         CREATE TABLE IF NOT EXISTS adaptive_learning_registry_meta (
             singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
             schema_version INTEGER NOT NULL DEFAULT 1,
             last_updated TEXT
         );
         CREATE TABLE IF NOT EXISTS adaptive_learning_processed_feedback (
             feedback_id TEXT PRIMARY KEY,
             processed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         );
         CREATE TABLE IF NOT EXISTS adaptive_learned_detectors (
             detector_id TEXT PRIMARY KEY,
             requirement_name TEXT NOT NULL,
             cause_type TEXT NOT NULL,
             positive_terms TEXT NOT NULL DEFAULT '[]',
             tags TEXT NOT NULL DEFAULT '[]',
             source_types TEXT NOT NULL DEFAULT '[]',
             min_severity INTEGER,
             confirmations INTEGER NOT NULL DEFAULT 0,
             false_positives INTEGER NOT NULL DEFAULT 0,
             created_from_feedback_id TEXT NOT NULL DEFAULT '',
             updated_at TEXT NOT NULL DEFAULT '',
             manually_disabled INTEGER NOT NULL DEFAULT 0,
             status_reason TEXT,
             review_status TEXT NOT NULL DEFAULT 'unreviewed',
             review_reason TEXT,
             last_reviewed_at TEXT
         );
         CREATE TABLE IF NOT EXISTS adaptive_learned_templates (
             template_id TEXT PRIMARY KEY,
             template_name TEXT NOT NULL,
             cause_type TEXT NOT NULL,
             cause_subtype TEXT,
             title_template TEXT NOT NULL DEFAULT '',
             confidence REAL NOT NULL DEFAULT 0,
             requires_json TEXT NOT NULL DEFAULT '[]',
             requires_same_service INTEGER NOT NULL DEFAULT 0,
             requires_temporal_order INTEGER NOT NULL DEFAULT 0,
             confirmations INTEGER NOT NULL DEFAULT 0,
             false_positives INTEGER NOT NULL DEFAULT 0,
             created_from_feedback_id TEXT NOT NULL DEFAULT '',
             updated_at TEXT NOT NULL DEFAULT '',
             manually_disabled INTEGER NOT NULL DEFAULT 0,
             status_reason TEXT,
             review_status TEXT NOT NULL DEFAULT 'unreviewed',
             review_reason TEXT,
             last_reviewed_at TEXT
         );
         CREATE TABLE IF NOT EXISTS adaptive_learned_compositions (
             composition_id TEXT PRIMARY KEY,
             composition_name TEXT NOT NULL,
             cause_type TEXT NOT NULL,
             cause_subtype TEXT,
             title_template TEXT NOT NULL DEFAULT '',
             confidence REAL NOT NULL DEFAULT 0,
             requires_json TEXT NOT NULL DEFAULT '[]',
             requires_same_service INTEGER NOT NULL DEFAULT 0,
             requires_temporal_order INTEGER NOT NULL DEFAULT 0,
             preferred_edge_types TEXT NOT NULL DEFAULT '[]',
             confirmations INTEGER NOT NULL DEFAULT 0,
             false_positives INTEGER NOT NULL DEFAULT 0,
             created_from_feedback_id TEXT NOT NULL DEFAULT '',
             updated_at TEXT NOT NULL DEFAULT '',
             manually_disabled INTEGER NOT NULL DEFAULT 0,
             status_reason TEXT,
             review_status TEXT NOT NULL DEFAULT 'unreviewed',
             review_reason TEXT,
             last_reviewed_at TEXT
         );
         CREATE TABLE IF NOT EXISTS adaptive_learned_edge_profiles (
             profile_id TEXT PRIMARY KEY,
             edge_type TEXT NOT NULL,
             source_service TEXT,
             target_service TEXT,
             cause_type TEXT,
             confirmations INTEGER NOT NULL DEFAULT 0,
             false_positives INTEGER NOT NULL DEFAULT 0,
             average_plausibility REAL NOT NULL DEFAULT 0,
             average_latency_ms REAL NOT NULL DEFAULT 0,
             created_from_feedback_id TEXT NOT NULL DEFAULT '',
             updated_at TEXT NOT NULL DEFAULT '',
             manually_disabled INTEGER NOT NULL DEFAULT 0,
             status_reason TEXT,
             review_status TEXT NOT NULL DEFAULT 'unreviewed',
             review_reason TEXT,
             last_reviewed_at TEXT
         );
         CREATE TABLE IF NOT EXISTS adaptive_review_views (
             view_id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             description TEXT,
             search_text TEXT,
             assigned_reviewer TEXT,
             created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
             last_used_at TEXT
         );
         CREATE TABLE IF NOT EXISTS adaptive_review_view_artifacts (
             view_id TEXT NOT NULL,
             artifact_kind TEXT NOT NULL,
             artifact_id TEXT NOT NULL,
             PRIMARY KEY (view_id, artifact_kind, artifact_id)
         );
         CREATE INDEX IF NOT EXISTS idx_incidents_state ON incidents(state);
         CREATE INDEX IF NOT EXISTS idx_incidents_updated ON incidents(updated_at);
         CREATE INDEX IF NOT EXISTS idx_incidents_created ON incidents(created_at);
         CREATE INDEX IF NOT EXISTS idx_incidents_severity ON incidents(severity DESC);
         CREATE INDEX IF NOT EXISTS idx_incidents_state_severity ON incidents(state, severity DESC);
         CREATE INDEX IF NOT EXISTS idx_incident_events_incident_id ON incident_events(incident_id);
         CREATE INDEX IF NOT EXISTS idx_ie_event ON incident_events(event_id);
         CREATE INDEX IF NOT EXISTS idx_hypotheses_incident_id ON hypotheses(incident_id);
         CREATE INDEX IF NOT EXISTS idx_hyp_incident_rank ON hypotheses(incident_id, rank);
         CREATE INDEX IF NOT EXISTS idx_incident_clusters_incident_id ON incident_clusters(incident_id);
         CREATE INDEX IF NOT EXISTS idx_explanations_incident ON explanations(incident_id);
         CREATE INDEX IF NOT EXISTS idx_explanations_cache ON explanations(incident_id, hypotheses_hash, events_hash_head);
         CREATE INDEX IF NOT EXISTS idx_ui_snapshots_updated ON ui_snapshots(updated_at DESC);
         CREATE INDEX IF NOT EXISTS idx_ui_snapshots_source ON ui_snapshots(source);
         CREATE INDEX IF NOT EXISTS idx_feedback_incident ON feedback(incident_id);
         CREATE INDEX IF NOT EXISTS idx_feedback_created ON feedback(created_at);
         CREATE INDEX IF NOT EXISTS idx_state_log_incident ON incident_state_log(incident_id);
         CREATE INDEX IF NOT EXISTS idx_state_log_changed ON incident_state_log(changed_at);
         CREATE INDEX IF NOT EXISTS idx_ai_traces_incident ON incident_ai_traces(incident_id, created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_chat_incident ON incident_chat_messages(incident_id, created_at ASC);
         CREATE INDEX IF NOT EXISTS idx_ai_generations_scope ON ai_generations(scope_key, created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_ai_generations_focus ON ai_generations(focus, created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_audit_created ON adaptive_learning_audit(created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_audit_artifact ON adaptive_learning_audit(artifact_kind, artifact_id, created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_audit_action ON adaptive_learning_audit(action, created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_history_observed ON adaptive_learning_history(observed_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_history_artifact ON adaptive_learning_history(artifact_kind, artifact_id, observed_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_history_incident ON adaptive_learning_history(incident_id, observed_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_processed_feedback ON adaptive_learning_processed_feedback(processed_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_detectors_requirement ON adaptive_learned_detectors(requirement_name);
         CREATE INDEX IF NOT EXISTS idx_adaptive_templates_name ON adaptive_learned_templates(template_name);
         CREATE INDEX IF NOT EXISTS idx_adaptive_compositions_name ON adaptive_learned_compositions(composition_name);
         CREATE INDEX IF NOT EXISTS idx_adaptive_edge_profiles_edge ON adaptive_learned_edge_profiles(edge_type, cause_type);
         CREATE INDEX IF NOT EXISTS idx_adaptive_review_views_updated ON adaptive_review_views(updated_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_review_views_assignee ON adaptive_review_views(assigned_reviewer, updated_at DESC);
         CREATE INDEX IF NOT EXISTS idx_adaptive_review_view_artifacts_view ON adaptive_review_view_artifacts(view_id, artifact_kind);",
    )
    .context("initialize incidents db schema")?;
    ensure_column(
        &conn,
        "incidents",
        "time_range_start",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &conn,
        "incidents",
        "time_range_end",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &conn,
        "incidents",
        "schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    ensure_column(
        &conn,
        "incidents",
        "cluster_ids",
        "TEXT NOT NULL DEFAULT '[]'",
    )?;
    ensure_column(&conn, "incidents", "runtime_context", "TEXT")?;
    ensure_column(&conn, "incidents", "resolution_info", "TEXT")?;
    ensure_column(
        &conn,
        "hypotheses",
        "score_breakdown",
        "TEXT NOT NULL DEFAULT '{}'",
    )?;
    ensure_column(
        &conn,
        "hypotheses",
        "supporting_events",
        "TEXT NOT NULL DEFAULT '[]'",
    )?;
    ensure_column(
        &conn,
        "hypotheses",
        "contradicting_events",
        "TEXT NOT NULL DEFAULT '[]'",
    )?;
    ensure_column(
        &conn,
        "hypotheses",
        "affected_services",
        "TEXT NOT NULL DEFAULT '[]'",
    )?;
    ensure_column(
        &conn,
        "hypotheses",
        "is_valid",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    ensure_column(
        &conn,
        "hypotheses",
        "invalidation_reasons",
        "TEXT NOT NULL DEFAULT '[]'",
    )?;
    ensure_column(
        &conn,
        "hypotheses",
        "created_at",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &conn,
        "hypotheses",
        "updated_at",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &conn,
        "explanations",
        "explanation_schema_version",
        "INTEGER NOT NULL DEFAULT 1",
    )?;
    ensure_column(
        &conn,
        "explanations",
        "hypotheses_hash",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &conn,
        "explanations",
        "events_hash_head",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &conn,
        "explanations",
        "quality",
        "TEXT NOT NULL DEFAULT 'ok'",
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ai_operator_context (
            scope_key TEXT PRIMARY KEY,
            body TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );",
        [],
    )
    .context("create ai_operator_context")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ai_generations (
            generation_id TEXT PRIMARY KEY,
            scope_key TEXT NOT NULL,
            focus TEXT NOT NULL,
            mode TEXT NOT NULL,
            question TEXT NOT NULL DEFAULT '',
            response_json TEXT NOT NULL DEFAULT '{}',
            bundle_hash TEXT NOT NULL DEFAULT '',
            used_ai INTEGER NOT NULL DEFAULT 0,
            provider_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        );",
        [],
    )
    .context("create ai_generations")?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ai_generations_scope ON ai_generations(scope_key, created_at DESC)",
        [],
    )
    .context("create ai_generations scope index")?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_ai_generations_focus ON ai_generations(focus, created_at DESC)",
        [],
    )
    .context("create ai_generations focus index")?;
    conn.execute(
        "INSERT INTO _schema_version(schema_name, version) VALUES ('incidents', 9)
         ON CONFLICT(schema_name) DO UPDATE SET version = excluded.version, applied_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
        [],
    )
    .context("update incidents _schema_version")?;
    conn.execute(
        "INSERT INTO schema_version(name, version) VALUES ('incidents', 9)
         ON CONFLICT(name) DO UPDATE SET version = excluded.version",
        [],
    )
    .context("update incidents schema_version")?;
    Ok(())
}

fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    let pragma = format!("PRAGMA table_info({table})");
    let mut stmt = conn
        .prepare(&pragma)
        .with_context(|| format!("prepare table info for {table}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .with_context(|| format!("query table info for {table}"))?;
    for row in rows {
        if row? == column {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )
    .with_context(|| format!("add column {column} to {table}"))?;
    Ok(())
}

fn explanation_json_from_row(row: &Row<'_>) -> rusqlite::Result<Value> {
    Ok(serde_json::json!({
        "explanation_id": row.get::<_, String>(0)?,
        "summary": row.get::<_, String>(1)?,
        "primary_text": row.get::<_, String>(2)?,
        "evidence_text": row.get::<_, Option<String>>(3)?,
        "timeline_text": row.get::<_, Option<String>>(4)?,
        "alternatives": parse_json_array(row.get::<_, String>(5)?),
        "actions": parse_json_array(row.get::<_, String>(6)?),
        "uncertainty": parse_json_array(row.get::<_, String>(7)?),
        "model_used": row.get::<_, String>(8)?,
        "guardrail_flags": parse_json_array(row.get::<_, String>(9)?),
        "created_at": row.get::<_, String>(10)?,
        "explanation_schema_version": row.get::<_, i64>(11)?,
        "hypotheses_hash": row.get::<_, String>(12)?,
        "events_hash_head": row.get::<_, String>(13)?,
        "quality": row.get::<_, String>(14)?,
    }))
}

fn incident_row_from_row(row: &Row<'_>) -> rusqlite::Result<IncidentRow> {
    let affected_raw: String = row.get(4)?;
    let affected: Option<Vec<String>> = serde_json::from_str(&affected_raw).ok();
    Ok(IncidentRow {
        incident_id: row.get(0)?,
        state: row.get(1)?,
        severity: row.get(2)?,
        primary_service: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
        affected_services: affected,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        event_count: row.get(7)?,
    })
}

fn event_row_from_row(row: &Row<'_>) -> rusqlite::Result<EventRow> {
    let severity = row.get::<_, Option<i64>>(2)?;
    let tags_raw: Option<String> = row.get(6)?;
    Ok(EventRow {
        event_id: row.get(0)?,
        timestamp: row.get(1)?,
        severity: severity.map(SeverityValue::Level),
        service_id: row.get(3)?,
        message: row.get(4)?,
        summary: row.get(4)?,
        source_ref: Some(EventSourceRef {
            source_type: row.get(5)?,
        }),
        tags: tags_raw.and_then(parse_tags),
    })
}

fn parse_tags(raw: String) -> Option<Vec<String>> {
    if raw.trim().is_empty() {
        return None;
    }
    if let Ok(parsed) = serde_json::from_str::<Vec<String>>(&raw) {
        return Some(parsed);
    }
    Some(
        raw.split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn parse_json_array(raw: String) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default()
}

fn ai_generation_json_from_row(row: &Row<'_>) -> rusqlite::Result<Value> {
    let response_text: String = row.get(5)?;
    let provider_text: String = row.get(8)?;
    Ok(serde_json::json!({
        "generation_id": row.get::<_, String>(0)?,
        "scope_key": row.get::<_, String>(1)?,
        "focus": row.get::<_, String>(2)?,
        "mode": row.get::<_, String>(3)?,
        "question": row.get::<_, String>(4)?,
        "response": serde_json::from_str::<Value>(&response_text).unwrap_or(Value::Null),
        "bundle_hash": row.get::<_, String>(6)?,
        "used_ai": row.get::<_, i64>(7)? != 0,
        "provider": serde_json::from_str::<Value>(&provider_text).unwrap_or(Value::Null),
        "created_at": row.get::<_, String>(9)?,
    }))
}

fn table_exists(conn: &Connection, table_name: &str) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
        rusqlite::params![table_name],
        |_| Ok(()),
    )
    .optional()
    .map(|value| value.is_some())
    .context("query sqlite_master")
}

fn is_missing_table_error(error: &impl std::fmt::Display) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("no such table")
}

fn ui_snapshot_from_row(row: &Row<'_>) -> rusqlite::Result<StoredUiSnapshot> {
    let payload_raw: String = row.get(1)?;
    Ok(StoredUiSnapshot {
        data_type: row.get(0)?,
        payload: serde_json::from_str(&payload_raw).unwrap_or(Value::Null),
        source: row.get(2)?,
        updated_at: row.get(3)?,
        schema_version: row.get(4)?,
        interval_seconds: row.get(5)?,
    })
}

fn now_iso() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

fn parse_rfc3339(raw: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(raw, &time::format_description::well_known::Rfc3339).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db_paths(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("inferra-storage-{name}-{unique}"));
        let events = root.join("events.db");
        let incidents = root.join("incidents.db");
        (root, events, incidents)
    }

    #[test]
    fn initialize_databases_creates_extended_python_compatible_schema() {
        let (_root, events_db, incidents_db) = temp_db_paths("schema");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");

        let events = Connection::open(&events_db).expect("open events db");
        let fingerprint: String = events
            .query_row(
                "SELECT name FROM pragma_table_info('events') WHERE name = 'fingerprint'",
                [],
                |row| row.get(0),
            )
            .expect("fingerprint column");
        assert_eq!(fingerprint, "fingerprint");
        let state_table: String = events
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'collector_state'",
                [],
                |row| row.get(0),
            )
            .expect("collector_state table");
        assert_eq!(state_table, "collector_state");

        let incidents = Connection::open(&incidents_db).expect("open incidents db");
        let resolution_info: String = incidents
            .query_row(
                "SELECT name FROM pragma_table_info('incidents') WHERE name = 'resolution_info'",
                [],
                |row| row.get(0),
            )
            .expect("resolution_info column");
        assert_eq!(resolution_info, "resolution_info");
        let feedback_table: String = incidents
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'feedback'",
                [],
                |row| row.get(0),
            )
            .expect("feedback table");
        assert_eq!(feedback_table, "feedback");
        let snapshots_table: String = incidents
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'ui_snapshots'",
                [],
                |row| row.get(0),
            )
            .expect("ui_snapshots table");
        assert_eq!(snapshots_table, "ui_snapshots");
    }

    #[test]
    fn events_store_supports_insert_batch_and_collector_state() {
        let (_root, events_db, incidents_db) = temp_db_paths("events-write");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
        let mut store = EventsStore::open(&events_db)
            .expect("open events result")
            .expect("events store present");

        let inserted = store
            .insert_batch(&[NewEventRecord {
                event_id: "evt-1".into(),
                timestamp: "2026-05-07T10:00:00Z".into(),
                service_id: "api".into(),
                severity: 3,
                message: "timeout calling postgres".into(),
                source_type: "host".into(),
                source_id: "host.local".into(),
                tags: vec!["database".into()],
                fingerprint: "fp-1".into(),
                host_id: "host.local".into(),
                event_type: 1,
                timestamp_source: "collector".into(),
                collected_at: "2026-05-07T10:00:01Z".into(),
                quality: Some("normalized".into()),
                structured_data: Some(serde_json::json!({"collector":"host_metrics"})),
                raw_offset: None,
            }])
            .expect("insert batch");
        assert_eq!(inserted, 1);
        assert!(store
            .fingerprint_exists("fp-1")
            .expect("fingerprint exists"));

        store
            .set_collector_state("host_metrics", "cursor", "42", "2026-05-07T10:01:00Z")
            .expect("set collector state");
        let cursor = store
            .get_collector_state("host_metrics", "cursor")
            .expect("get collector state");
        assert_eq!(cursor.as_deref(), Some("42"));
    }

    #[test]
    fn incidents_store_persists_ui_snapshots_by_data_type() {
        let (_root, events_db, incidents_db) = temp_db_paths("ui-snapshot");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
        let store = IncidentsStore::open(&incidents_db)
            .expect("open incidents result")
            .expect("incidents store present");

        store
            .upsert_ui_snapshot(
                "workspace",
                &serde_json::json!({"runtime_apps":[{"name":"inferra"}]}),
                "inferra_core",
                Some(120),
            )
            .expect("upsert snapshot");
        let snapshot = store
            .ui_snapshot("workspace")
            .expect("query snapshot")
            .expect("snapshot present");
        assert_eq!(snapshot.data_type, "workspace");
        assert_eq!(snapshot.source, "inferra_core");
        assert_eq!(snapshot.interval_seconds, Some(120));
        assert_eq!(snapshot.payload["runtime_apps"][0]["name"], "inferra");
        assert_eq!(store.ui_snapshots().expect("list snapshots").len(), 1);
    }

    #[test]
    fn get_events_returns_rows_in_requested_order() {
        let (_root, events_db, incidents_db) = temp_db_paths("events-batch-read");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
        let mut store = EventsStore::open(&events_db)
            .expect("open events result")
            .expect("events store present");
        store
            .insert_batch(&[
                NewEventRecord::minimal(
                    "evt-a",
                    "2026-05-07T10:00:00Z",
                    "api",
                    2,
                    "first event",
                    "app_http",
                    "2026-05-07T10:00:00Z",
                ),
                NewEventRecord::minimal(
                    "evt-b",
                    "2026-05-07T10:00:01Z",
                    "api",
                    3,
                    "second event",
                    "app_http",
                    "2026-05-07T10:00:01Z",
                ),
            ])
            .expect("insert events");

        let events = store
            .get_events(&["evt-b".into(), "evt-a".into()])
            .expect("load events");
        let ids = events
            .into_iter()
            .filter_map(|event| event.event_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["evt-b".to_string(), "evt-a".to_string()]);
    }

    #[test]
    fn governed_insert_suppresses_duplicates_inside_window() {
        let (_root, events_db, incidents_db) = temp_db_paths("events-governed-dedup");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
        let mut store = EventsStore::open(&events_db)
            .expect("open events result")
            .expect("events store present");
        let governance = IngestGovernance {
            dedup_enabled: true,
            noise_enabled: false,
            ..Default::default()
        };

        let first = store
            .insert_batch_governed(
                &[NewEventRecord {
                    event_id: "evt-dup-1".into(),
                    timestamp: "2026-05-07T10:00:00Z".into(),
                    service_id: "api".into(),
                    severity: 2,
                    message: "timeout calling postgres".into(),
                    source_type: "app_http".into(),
                    source_id: "ingest".into(),
                    tags: vec!["database".into()],
                    fingerprint: "api:timeout:postgres".into(),
                    host_id: "host.local".into(),
                    event_type: 1,
                    timestamp_source: "collector".into(),
                    collected_at: "2026-05-07T10:00:00Z".into(),
                    quality: Some("normalized".into()),
                    structured_data: None,
                    raw_offset: None,
                }],
                &governance,
            )
            .expect("insert first");
        assert_eq!(first.inserted, 1);

        let second = store
            .insert_batch_governed(
                &[NewEventRecord {
                    event_id: "evt-dup-2".into(),
                    timestamp: "2026-05-07T10:00:20Z".into(),
                    service_id: "api".into(),
                    severity: 2,
                    message: "timeout calling postgres".into(),
                    source_type: "app_http".into(),
                    source_id: "ingest".into(),
                    tags: vec!["database".into()],
                    fingerprint: "api:timeout:postgres".into(),
                    host_id: "host.local".into(),
                    event_type: 1,
                    timestamp_source: "collector".into(),
                    collected_at: "2026-05-07T10:00:20Z".into(),
                    quality: Some("normalized".into()),
                    structured_data: None,
                    raw_offset: None,
                }],
                &governance,
            )
            .expect("insert duplicate");
        assert_eq!(second.inserted, 0);
        assert_eq!(second.suppressed_duplicates, 1);

        let summary = store.governance_summary().expect("governance summary");
        assert_eq!(summary.dedup_suppressed_total, 1);
        assert_eq!(summary.active_dedup_windows, 1);
        assert_eq!(summary.tracked_fingerprints, 1);
    }

    #[test]
    fn governed_insert_applies_noise_rules_and_allowlist_tags() {
        let (_root, events_db, incidents_db) = temp_db_paths("events-governed-noise");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
        let mut store = EventsStore::open(&events_db)
            .expect("open events result")
            .expect("events store present");
        let governance = IngestGovernance {
            dedup_enabled: false,
            noise_enabled: true,
            blocklist_enabled: true,
            allowlist_enabled: true,
            always_keep_severity: 3,
            blocklist: vec![GovernanceRule {
                pattern: "health check passed".into(),
                severity_max: Some(1),
                reason: Some("routine health signal".into()),
                ..Default::default()
            }],
            allowlist: vec![GovernanceRule {
                pattern: "out of memory".into(),
                severity_min: Some(3),
                tags: vec!["oom".into()],
                reason: Some("resource failures are always relevant".into()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let suppressed = store
            .insert_batch_governed(
                &[NewEventRecord {
                    event_id: "evt-noise-1".into(),
                    timestamp: "2026-05-07T10:00:00Z".into(),
                    service_id: "api".into(),
                    severity: 1,
                    message: "health check passed".into(),
                    source_type: "app_http".into(),
                    source_id: "ingest".into(),
                    tags: Vec::new(),
                    fingerprint: "api:healthcheck".into(),
                    host_id: "host.local".into(),
                    event_type: 1,
                    timestamp_source: "collector".into(),
                    collected_at: "2026-05-07T10:00:00Z".into(),
                    quality: Some("normalized".into()),
                    structured_data: None,
                    raw_offset: None,
                }],
                &governance,
            )
            .expect("insert blocklisted event");
        assert_eq!(suppressed.inserted, 0);
        assert_eq!(suppressed.suppressed_noise, 1);

        let allowlisted = store
            .insert_batch_governed(
                &[NewEventRecord {
                    event_id: "evt-noise-2".into(),
                    timestamp: "2026-05-07T10:01:00Z".into(),
                    service_id: "api".into(),
                    severity: 3,
                    message: "out of memory".into(),
                    source_type: "host_metrics".into(),
                    source_id: "host.local".into(),
                    tags: Vec::new(),
                    fingerprint: "api:oom".into(),
                    host_id: "host.local".into(),
                    event_type: 1,
                    timestamp_source: "collector".into(),
                    collected_at: "2026-05-07T10:01:00Z".into(),
                    quality: Some("normalized".into()),
                    structured_data: None,
                    raw_offset: None,
                }],
                &governance,
            )
            .expect("insert allowlisted event");
        assert_eq!(allowlisted.inserted, 1);
        assert_eq!(allowlisted.allowlisted, 1);

        let stored = store
            .latest_events(5)
            .expect("latest events")
            .into_iter()
            .find(|event| event.event_id.as_deref() == Some("evt-noise-2"))
            .expect("stored allowlisted event");
        assert!(stored
            .tags
            .unwrap_or_default()
            .iter()
            .any(|tag| tag == "oom"));

        let summary = store.governance_summary().expect("governance summary");
        assert_eq!(summary.noise_suppressed_total, 1);
        assert_eq!(summary.allowlisted_total, 1);
        assert_eq!(
            summary.last_noise_reason.as_deref(),
            Some("routine health signal")
        );
    }

    #[test]
    fn incidents_store_supports_write_side_lifecycle_records() {
        let (_root, events_db, incidents_db) = temp_db_paths("incidents-write");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
        let mut events = EventsStore::open(&events_db)
            .expect("open events")
            .expect("events store");
        events
            .insert_batch(&[NewEventRecord {
                event_id: "evt-1".into(),
                timestamp: "2026-05-07T10:00:00Z".into(),
                service_id: "api".into(),
                severity: 4,
                message: "database unavailable".into(),
                source_type: "host".into(),
                source_id: "host.local".into(),
                tags: vec!["database".into()],
                fingerprint: "fp-1".into(),
                host_id: "host.local".into(),
                event_type: 1,
                timestamp_source: "collector".into(),
                collected_at: "2026-05-07T10:00:01Z".into(),
                quality: None,
                structured_data: None,
                raw_offset: None,
            }])
            .expect("seed events");

        let mut incidents = IncidentsStore::open(&incidents_db)
            .expect("open incidents")
            .expect("incidents store");
        incidents
            .upsert_incident(
                &IncidentRecord {
                    incident_id: "inc-1".into(),
                    state: "open".into(),
                    severity: 4,
                    primary_service: "api".into(),
                    affected_services: vec!["api".into()],
                    created_at: "2026-05-07T10:00:00Z".into(),
                    updated_at: "2026-05-07T10:00:00Z".into(),
                    time_range_start: "2026-05-07T09:59:00Z".into(),
                    time_range_end: "2026-05-07T10:00:00Z".into(),
                    event_count: 1,
                    cluster_ids: vec!["cluster-1".into()],
                    runtime_context: Some(serde_json::json!({"host":"local"})),
                    resolution_info: None,
                },
                &["evt-1".into()],
            )
            .expect("upsert incident");
        incidents
            .replace_hypotheses(
                "inc-1",
                &[StoredHypothesis {
                    hypothesis_id: "hyp-1".into(),
                    rank: Some(1),
                    cause_type: "database".into(),
                    description: "primary datastore is timing out".into(),
                    total_score: Some(0.92),
                    score_breakdown: serde_json::json!({"latency":0.92}),
                    supporting_events: vec!["evt-1".into()],
                    contradicting_events: Vec::new(),
                    affected_services: vec!["api".into()],
                    suggested_checks: vec!["check postgres latency".into()],
                    confidence_label: Some("high".into()),
                    is_valid: true,
                    invalidation_reasons: Vec::new(),
                    created_at: "2026-05-07T10:00:00Z".into(),
                    updated_at: "2026-05-07T10:00:00Z".into(),
                }],
            )
            .expect("replace hypotheses");
        incidents
            .upsert_cluster(
                "inc-1",
                "cluster-1",
                &serde_json::json!({"kind":"database"}),
            )
            .expect("upsert cluster");
        incidents
            .add_explanation(&StoredExplanation {
                explanation_id: "exp-1".into(),
                incident_id: "inc-1".into(),
                summary: "Database outage".into(),
                primary_text: "Primary datastore is unavailable.".into(),
                evidence_text: Some("Timeouts and connection refusals were observed.".into()),
                timeline_text: None,
                alternatives: vec!["network partition".into()],
                actions: vec!["check database health".into()],
                uncertainty: vec!["single host snapshot".into()],
                model_used: "deterministic".into(),
                guardrail_flags: vec!["read_only".into()],
                created_at: "2026-05-07T10:01:00Z".into(),
                explanation_schema_version: 1,
                hypotheses_hash: "hash-h".into(),
                events_hash_head: "hash-e".into(),
                quality: "ok".into(),
            })
            .expect("add explanation");
        incidents
            .add_feedback(&StoredFeedback {
                feedback_id: "fb-1".into(),
                incident_id: "inc-1".into(),
                correct_hypothesis_id: Some("hyp-1".into()),
                feedback_type: "confirmed".into(),
                operator_notes: "matched reality".into(),
                resolved_at: "2026-05-07T10:02:00Z".into(),
                created_at: Some("2026-05-07T10:02:00Z".into()),
            })
            .expect("add feedback");
        incidents
            .transition_state("inc-1", "investigating", "triage", "2026-05-07T10:03:00Z")
            .expect("transition state");
        incidents
            .resolve_incident(
                "inc-1",
                &serde_json::json!({"resolved_by":"operator"}),
                "2026-05-07T10:04:00Z",
            )
            .expect("resolve incident");

        let incident = incidents
            .get_incident("inc-1")
            .expect("load incident")
            .expect("incident exists");
        assert_eq!(incident.state, "resolved");
        assert_eq!(incident.event_count, Some(1));
        assert_eq!(
            incidents
                .hypotheses("inc-1")
                .expect("load hypotheses")
                .len(),
            1
        );
        assert_eq!(incidents.clusters("inc-1").expect("load clusters").len(), 1);
        assert!(incidents
            .cached_explanation("inc-1", "hash-h", "hash-e")
            .expect("cached explanation")
            .is_some());
        incidents
            .add_ai_trace(&StoredAiTrace {
                trace_id: "trace-1".into(),
                incident_id: "inc-1".into(),
                trace_kind: "investigate".into(),
                sanitized_system_prompt: "system".into(),
                sanitized_user_prompt: "user".into(),
                allowed_fields: vec!["incident".into()],
                blocked_fields: vec!["secrets".into()],
                raw_logs_sent: false,
                trace_schema_version: 1,
                created_at: "2026-05-07T10:03:30Z".into(),
            })
            .expect("add ai trace");
        incidents
            .add_ai_generation(&StoredAiGeneration {
                generation_id: "gen-1".into(),
                scope_key: "incident:inc-1|mode=developer|report=false|q=0".into(),
                focus: "incident:inc-1".into(),
                mode: "developer".into(),
                question: String::new(),
                response: serde_json::json!({"output":{"headline":"Database outage"}, "used_ai": false}),
                bundle_hash: "bundle-hash".into(),
                used_ai: false,
                provider: serde_json::json!({"enabled": false}),
                created_at: "2026-05-07T10:03:31Z".into(),
            })
            .expect("add ai generation");
        incidents
            .upsert_inference_graph_snapshot(&StoredInferenceGraphSnapshot {
                incident_id: "inc-1".into(),
                graph_data: serde_json::json!({"nodes":["api", "postgres"], "edges":[["api","postgres"]]}),
                created_at: "2026-05-07T10:03:15Z".into(),
                event_count: 1,
            })
            .expect("add inference graph snapshot");
        incidents
            .add_chat_message(&StoredChatMessage {
                message_id: "msg-1".into(),
                incident_id: "inc-1".into(),
                role: "user".into(),
                content: "What failed first?".into(),
                message_schema_version: 1,
                created_at: "2026-05-07T10:03:20Z".into(),
            })
            .expect("add chat message");
        assert!(incidents
            .latest_ai_trace("inc-1")
            .expect("latest ai trace")
            .is_some());
        assert!(incidents
            .latest_ai_generation("incident:inc-1|mode=developer|report=false|q=0")
            .expect("latest ai generation")
            .is_some());
        assert!(incidents
            .inference_graph_snapshot("inc-1")
            .expect("graph snapshot")
            .is_some());
        assert_eq!(incidents.list_feedback("inc-1").expect("feedback").len(), 1);
        assert_eq!(
            incidents.list_chat_messages("inc-1").expect("chat").len(),
            1
        );
        assert_eq!(
            incidents.list_state_log("inc-1").expect("state log").len(),
            2
        );
    }

    #[test]
    fn incidents_store_round_trips_adaptive_learning_registry() {
        let (_root, events_db, incidents_db) = temp_db_paths("adaptive-registry");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
        let mut incidents = IncidentsStore::open(&incidents_db)
            .expect("open incidents")
            .expect("incidents store");

        incidents
            .replace_adaptive_learning_model(&StoredAdaptiveLearningModel {
                schema_version: 1,
                last_updated: Some("2026-05-08T10:10:00Z".into()),
                processed_feedback_ids: vec!["fb-1".into(), "fb-2".into()],
                learned_detectors: vec![StoredLearnedDetector {
                    detector_id: "detector-1".into(),
                    requirement_name: "database_latency".into(),
                    cause_type: "database_failure".into(),
                    positive_terms: vec!["timeout".into(), "postgres".into()],
                    tags: vec!["database".into()],
                    source_types: vec!["app_http".into()],
                    min_severity: Some(3),
                    confirmations: 4,
                    false_positives: 1,
                    created_from_feedback_id: "fb-1".into(),
                    updated_at: "2026-05-08T10:10:00Z".into(),
                    manually_disabled: false,
                    status_reason: None,
                    review_status: "approved".into(),
                    review_reason: Some("looks stable".into()),
                    last_reviewed_at: Some("2026-05-08T10:11:00Z".into()),
                }],
                learned_templates: vec![StoredLearnedTemplate {
                    template_id: "template-1".into(),
                    template_name: "db timeout".into(),
                    cause_type: "database_failure".into(),
                    cause_subtype: Some("latency".into()),
                    title_template: "Database latency spike".into(),
                    confidence: 0.82,
                    requires: vec!["database_latency".into()],
                    requires_same_service: true,
                    requires_temporal_order: false,
                    confirmations: 3,
                    false_positives: 0,
                    created_from_feedback_id: "fb-1".into(),
                    updated_at: "2026-05-08T10:10:00Z".into(),
                    manually_disabled: false,
                    status_reason: None,
                    review_status: "watch".into(),
                    review_reason: Some("monitor drift".into()),
                    last_reviewed_at: Some("2026-05-08T10:12:00Z".into()),
                }],
                learned_compositions: vec![StoredLearnedComposition {
                    composition_id: "composition-1".into(),
                    composition_name: "db timeout cascade".into(),
                    cause_type: "database_failure".into(),
                    cause_subtype: Some("cascade".into()),
                    title_template: "Timeout cascade from database".into(),
                    confidence: 0.91,
                    requires: vec!["database_latency".into(), "restart_signal".into()],
                    requires_same_service: true,
                    requires_temporal_order: true,
                    preferred_edge_types: vec!["depends_on".into()],
                    confirmations: 2,
                    false_positives: 0,
                    created_from_feedback_id: "fb-2".into(),
                    updated_at: "2026-05-08T10:13:00Z".into(),
                    manually_disabled: false,
                    status_reason: None,
                    review_status: "unreviewed".into(),
                    review_reason: None,
                    last_reviewed_at: None,
                }],
                learned_edge_profiles: vec![StoredLearnedEdgeProfile {
                    profile_id: "edge-1".into(),
                    edge_type: "depends_on".into(),
                    source_service: Some("api".into()),
                    target_service: Some("postgres".into()),
                    cause_type: Some("database_failure".into()),
                    confirmations: 5,
                    false_positives: 1,
                    average_plausibility: 0.67,
                    average_latency_ms: 245.0,
                    created_from_feedback_id: "fb-2".into(),
                    updated_at: "2026-05-08T10:14:00Z".into(),
                    manually_disabled: true,
                    status_reason: Some("under review".into()),
                    review_status: "rejected".into(),
                    review_reason: Some("too broad".into()),
                    last_reviewed_at: Some("2026-05-08T10:15:00Z".into()),
                }],
            })
            .expect("replace adaptive registry");

        let loaded = incidents
            .adaptive_learning_model()
            .expect("load adaptive registry")
            .expect("adaptive registry present");
        assert_eq!(
            loaded.processed_feedback_ids,
            vec!["fb-1".to_string(), "fb-2".to_string()]
        );
        assert_eq!(loaded.learned_detectors.len(), 1);
        assert_eq!(loaded.learned_templates.len(), 1);
        assert_eq!(loaded.learned_compositions.len(), 1);
        assert_eq!(loaded.learned_edge_profiles.len(), 1);
        assert_eq!(
            loaded.learned_detectors[0].positive_terms,
            vec!["timeout", "postgres"]
        );
        assert!(loaded.learned_templates[0].requires_same_service);
        assert_eq!(
            loaded.learned_compositions[0].preferred_edge_types,
            vec!["depends_on".to_string()]
        );
        assert!(loaded.learned_edge_profiles[0].manually_disabled);
        assert_eq!(loaded.learned_edge_profiles[0].review_status, "rejected");
    }

    #[test]
    fn incidents_store_round_trips_saved_adaptive_review_views() {
        let (_root, events_db, incidents_db) = temp_db_paths("adaptive-review-views");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
        let mut incidents = IncidentsStore::open(&incidents_db)
            .expect("open incidents")
            .expect("incidents store");

        incidents
            .upsert_adaptive_review_view(&StoredAdaptiveReviewView {
                view_id: "view-1".into(),
                name: "DB queue".into(),
                description: Some("database-focused triage".into()),
                search_text: Some("database".into()),
                assigned_reviewer: Some("alice".into()),
                artifact_selections: vec![
                    StoredAdaptiveReviewViewSelection {
                        artifact_kind: "detector".into(),
                        artifact_id: "det-1".into(),
                    },
                    StoredAdaptiveReviewViewSelection {
                        artifact_kind: "template".into(),
                        artifact_id: "tpl-1".into(),
                    },
                ],
                created_at: "2026-05-08T10:00:00Z".into(),
                updated_at: "2026-05-08T10:05:00Z".into(),
                last_used_at: Some("2026-05-08T10:07:00Z".into()),
            })
            .expect("upsert review view");

        let views = incidents
            .list_adaptive_review_views()
            .expect("list review views");
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].name, "DB queue");
        assert_eq!(views[0].assigned_reviewer.as_deref(), Some("alice"));
        assert_eq!(views[0].artifact_selections.len(), 2);

        incidents
            .touch_adaptive_review_view("view-1", "2026-05-08T10:10:00Z")
            .expect("touch review view");
        let touched = incidents
            .list_adaptive_review_views()
            .expect("reload review views");
        assert_eq!(
            touched[0].last_used_at.as_deref(),
            Some("2026-05-08T10:10:00Z")
        );

        incidents
            .delete_adaptive_review_view("view-1")
            .expect("delete review view");
        assert!(incidents
            .list_adaptive_review_views()
            .expect("list after delete")
            .is_empty());
    }

    #[test]
    fn archive_incident_moves_terminal_records_out_of_active_store() {
        let (root, events_db, incidents_db) = temp_db_paths("archive");
        initialize_databases(&events_db, &incidents_db).expect("initialize dbs");
        let mut incidents = IncidentsStore::open(&incidents_db)
            .expect("open incidents")
            .expect("incidents store");
        incidents
            .upsert_incident(
                &IncidentRecord {
                    incident_id: "inc-archive".into(),
                    state: "resolved".into(),
                    severity: 3,
                    primary_service: "api".into(),
                    affected_services: vec!["api".into()],
                    created_at: "2026-05-01T10:00:00Z".into(),
                    updated_at: "2026-05-01T10:00:00Z".into(),
                    time_range_start: "2026-05-01T10:00:00Z".into(),
                    time_range_end: "2026-05-01T10:00:00Z".into(),
                    event_count: 0,
                    cluster_ids: vec!["cluster-archive".into()],
                    runtime_context: Some(serde_json::json!({"service":"api"})),
                    resolution_info: Some(serde_json::json!({"resolved_by":"operator"})),
                },
                &[],
            )
            .expect("upsert archive incident");
        incidents
            .replace_hypotheses(
                "inc-archive",
                &[StoredHypothesis {
                    hypothesis_id: "hyp-archive".into(),
                    rank: Some(1),
                    cause_type: "dependency_failure".into(),
                    description: "archived hypothesis".into(),
                    total_score: Some(0.8),
                    score_breakdown: serde_json::json!({"evidence_coverage":0.8}),
                    supporting_events: Vec::new(),
                    contradicting_events: Vec::new(),
                    affected_services: vec!["api".into()],
                    suggested_checks: Vec::new(),
                    confidence_label: Some("medium".into()),
                    is_valid: true,
                    invalidation_reasons: Vec::new(),
                    created_at: "2026-05-01T10:00:00Z".into(),
                    updated_at: "2026-05-01T10:00:00Z".into(),
                }],
            )
            .expect("replace archive hypotheses");
        incidents
            .upsert_inference_graph_snapshot(&StoredInferenceGraphSnapshot {
                incident_id: "inc-archive".into(),
                graph_data: serde_json::json!({"nodes":["n1"],"edges":[]}),
                created_at: "2026-05-01T10:00:00Z".into(),
                event_count: 0,
            })
            .expect("store archive graph");
        incidents
            .add_feedback(&StoredFeedback {
                feedback_id: "fb-archive".into(),
                incident_id: "inc-archive".into(),
                correct_hypothesis_id: Some("hyp-archive".into()),
                feedback_type: "confirmed".into(),
                operator_notes: "archive me".into(),
                resolved_at: "2026-05-01T10:05:00Z".into(),
                created_at: Some("2026-05-01T10:05:00Z".into()),
            })
            .expect("store archive feedback");
        incidents
            .add_chat_message(&StoredChatMessage {
                message_id: "msg-archive".into(),
                incident_id: "inc-archive".into(),
                role: "user".into(),
                content: "hello".into(),
                message_schema_version: 1,
                created_at: "2026-05-01T10:06:00Z".into(),
            })
            .expect("store archive chat");

        let archive_db = root.join("archive").join("incidents_20260508.db");
        let archived = incidents
            .archive_incident_to_path("inc-archive", &archive_db, "2026-05-08T00:00:00Z")
            .expect("archive incident");
        assert!(archived);
        assert!(incidents
            .get_incident("inc-archive")
            .expect("load post-archive")
            .is_none());

        let archive = Connection::open(&archive_db).expect("open archive db");
        let payload_raw: String = archive
            .query_row(
                "SELECT payload_json FROM archived_incidents WHERE incident_id = 'inc-archive'",
                [],
                |row| row.get(0),
            )
            .expect("archived payload");
        let payload: Value = serde_json::from_str(&payload_raw).expect("parse archived payload");
        assert_eq!(
            payload["incident"]["incident_id"].as_str(),
            Some("inc-archive")
        );
        assert_eq!(
            payload["feedback"].as_array().map(|items| items.len()),
            Some(1)
        );
        assert_eq!(
            payload["chat_messages"].as_array().map(|items| items.len()),
            Some(1)
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
