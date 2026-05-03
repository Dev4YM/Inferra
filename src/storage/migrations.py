from __future__ import annotations

EVENTS_SCHEMA = """
CREATE TABLE IF NOT EXISTS schema_version (
    name TEXT PRIMARY KEY,
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
    event_id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL,
    timestamp_source TEXT NOT NULL,
    service_id TEXT NOT NULL,
    host_id TEXT NOT NULL,
    severity INTEGER NOT NULL,
    event_type INTEGER NOT NULL,
    message TEXT NOT NULL,
    structured_data TEXT NOT NULL,
    tags TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    quality TEXT NOT NULL,
    source_type TEXT NOT NULL,
    source_id TEXT NOT NULL,
    raw_offset INTEGER,
    collected_at TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1,
    inserted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
CREATE INDEX IF NOT EXISTS idx_events_service_ts ON events(service_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_events_severity_ts ON events(severity, timestamp);
CREATE INDEX IF NOT EXISTS idx_events_fingerprint ON events(fingerprint);
CREATE INDEX IF NOT EXISTS idx_events_inserted ON events(inserted_at);

CREATE TABLE IF NOT EXISTS collector_state (
    collector_id TEXT NOT NULL,
    state_key TEXT NOT NULL,
    state_value TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (collector_id, state_key)
);

CREATE INDEX IF NOT EXISTS idx_collector_state_updated ON collector_state(updated_at);
"""

INCIDENTS_SCHEMA = """
CREATE TABLE IF NOT EXISTS schema_version (
    name TEXT PRIMARY KEY,
    version INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS incidents (
    incident_id TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    severity INTEGER NOT NULL,
    primary_service TEXT,
    affected_services TEXT NOT NULL,
    time_range_start TEXT NOT NULL,
    time_range_end TEXT NOT NULL,
    event_count INTEGER NOT NULL DEFAULT 0,
    schema_version INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS incident_events (
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    event_id TEXT NOT NULL,
    added_at TEXT NOT NULL,
    PRIMARY KEY (incident_id, event_id)
);

CREATE TABLE IF NOT EXISTS incident_clusters (
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    cluster_id TEXT NOT NULL,
    cluster_data TEXT NOT NULL,
    PRIMARY KEY (incident_id, cluster_id)
);

CREATE TABLE IF NOT EXISTS hypotheses (
    hypothesis_id TEXT PRIMARY KEY,
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    rank INTEGER,
    cause_type TEXT NOT NULL,
    description TEXT NOT NULL,
    total_score REAL,
    score_breakdown TEXT NOT NULL,
    supporting_events TEXT NOT NULL,
    contradicting_events TEXT NOT NULL,
    affected_services TEXT NOT NULL,
    suggested_checks TEXT NOT NULL,
    confidence_label TEXT,
    is_valid INTEGER NOT NULL DEFAULT 1,
    invalidation_reasons TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS explanations (
    explanation_id TEXT PRIMARY KEY,
    incident_id TEXT NOT NULL REFERENCES incidents(incident_id),
    summary TEXT NOT NULL,
    primary_text TEXT NOT NULL,
    evidence_text TEXT,
    timeline_text TEXT,
    alternatives TEXT NOT NULL,
    actions TEXT NOT NULL,
    uncertainty TEXT NOT NULL,
    model_used TEXT NOT NULL,
    guardrail_flags TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_incidents_state ON incidents(state);
CREATE INDEX IF NOT EXISTS idx_incidents_updated ON incidents(updated_at);
CREATE INDEX IF NOT EXISTS idx_ie_incident ON incident_events(incident_id);
CREATE INDEX IF NOT EXISTS idx_ie_event ON incident_events(event_id);
CREATE INDEX IF NOT EXISTS idx_clusters_incident ON incident_clusters(incident_id);
CREATE INDEX IF NOT EXISTS idx_hyp_incident_rank ON hypotheses(incident_id, rank);
CREATE INDEX IF NOT EXISTS idx_explanations_incident ON explanations(incident_id);
"""
