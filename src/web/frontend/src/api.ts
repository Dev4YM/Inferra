export async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`${res.status} ${path}: ${text}`);
  }
  return res.json() as Promise<T>;
}

export async function postJson<T>(path: string, body: unknown): Promise<T> {
  return fetchJson<T>(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
}

export async function putJson<T>(path: string, body: unknown): Promise<T> {
  return fetchJson<T>(path, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
}

export type ExperiencePayload = {
  mode: string;
  ai_role: string;
  suggest_safe_actions: boolean;
  execute_actions: boolean;
  show_raw_evidence_by_default: boolean;
};

export type InferraConfigPayload = {
  experience?: Partial<ExperiencePayload>;
  [key: string]: unknown;
};

export type ConfigResponse = {
  config: InferraConfigPayload;
  applied?: boolean;
};

export type QuickAnalysis = {
  headline: string;
  risk_level: string;
  containers_running: number;
  process_sample_size: number;
  code_projects_found: number;
  mode: string;
  ai_role: string;
};

export type ServiceRow = {
  service_id: string;
  status: string;
  event_count?: number;
  error_count?: number;
  error_ratio?: number;
  last_event_at?: string | null;
  active_incidents?: IncidentRow[];
};

export type IncidentRow = {
  incident_id: string;
  state: string;
  severity: number;
  primary_service: string;
  affected_services?: string[];
  created_at?: string;
  updated_at?: string;
  event_count?: number;
};

export type DashboardPayload = {
  health?: {
    status?: string;
    active_incidents?: number;
    queue_depth?: number;
    collector_errors?: number;
    degraded?: boolean;
    degraded_reasons?: string[];
    storage_writes_ok?: boolean;
    data_dir_bytes_free?: number;
    ai_enabled?: boolean;
    ai_available?: boolean;
    ai_reason?: string;
  };
  dedup?: Record<string, number>;
  noise?: Record<string, number>;
  incidents?: IncidentRow[];
  services?: ServiceRow[];
  event_rate?: { timestamp: string; total: number; warn?: number; error?: number; critical?: number }[];
  severity_counts?: Record<string, number>;
};

export type RuntimeContext = {
  hostname?: string;
  containers?: { name: string; image: string; state: string }[];
  processes?: { pid: number; name: string; cpu_percent: number; memory_mb: number }[];
};

export type WorkspaceProject = { path: string; kind: string; marker: string };

export type OverviewResponse = {
  quick_analysis: QuickAnalysis;
  dashboard: DashboardPayload;
  runtime: RuntimeContext;
  workspace_projects: WorkspaceProject[];
  experience: ExperiencePayload;
};

export type CollectorRow = {
  collector_id: string;
  status?: string;
  source_type?: string;
  is_running?: boolean;
  events_emitted?: number;
  events_per_second?: number;
  last_event_at?: string | null;
  error_count?: number;
  dropped_events?: number;
  queue_depth?: number;
  last_error?: string | null;
  lag_seconds?: number;
};

export type AiStatus = {
  enabled: boolean;
  provider?: string;
  model?: string;
  base_url?: string;
  available?: boolean;
  installed?: boolean;
  resolved_model?: string;
  reason?: string;
  error?: string;
  allow_remote?: boolean;
  registry_model?: Record<string, unknown> | null;
};

export type InvestigationStep = {
  title: string;
  reason?: string;
  safety?: string;
  command?: string;
  requires_user_action?: boolean;
};

export type InvestigationEvidence = { type: string; id: string; summary: string };

export type InvestigationOutput = {
  headline: string;
  risk_level: string;
  confidence: number;
  what_happened: string[];
  why_it_matters: string[];
  likely_causes: string[];
  evidence: InvestigationEvidence[];
  missing_evidence: string[];
  next_steps: InvestigationStep[];
  uncertainty: string[];
  citations: string[];
};

export type InvestigationResponse = {
  schema_version: number;
  output: InvestigationOutput;
  used_ai: boolean;
  fallback_reason: string;
  provider: {
    enabled: boolean;
    available: boolean;
    model?: string;
    base_url?: string;
    allow_remote?: boolean;
    reason?: string;
  };
  trace?: {
    trace_kind: string;
    sanitized_system_prompt: string;
    sanitized_user_prompt: string;
    allowed_fields: string[];
    blocked_fields: string[];
    raw_logs_sent: boolean;
  } | null;
  bundle?: Record<string, unknown>;
  focus?: string;
  question?: string;
};

export type WorkspaceMappingSignal = { name: string; confidence: number; detail: string };

export type WorkspaceMapping = {
  service_id: string;
  project_path: string;
  confidence: number;
  source: string;
  notes?: string;
  signals: WorkspaceMappingSignal[];
};

export type WorkspaceMapResponse = {
  enabled: boolean;
  projects: WorkspaceProject[];
  service_mappings: WorkspaceMapping[];
  unmapped_services: string[];
  config_mappings: WorkspaceMapping[];
};

export type AiDoctorResponse = {
  ok: boolean;
  enabled: boolean;
  provider: string;
  base_url: string;
  model: string;
  allow_remote: boolean;
  token_env_set: boolean;
  redact_raw_logs: boolean;
  available: boolean;
  warnings: string[];
  guidance?: string[];
};
