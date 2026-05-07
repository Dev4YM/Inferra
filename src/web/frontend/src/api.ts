export class ApiError extends Error {
  status: number;
  path: string;
  body: string;

  constructor(message: string, path: string, status = 0, body = "") {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.path = path;
    this.body = body;
  }
}

async function parseResponseBody(response: Response, path: string): Promise<unknown> {
  const text = await response.text();
  if (!text.trim()) {
    throw new ApiError(`Empty response from ${path}`, path, response.status);
  }
  try {
    return JSON.parse(text) as unknown;
  } catch (error) {
    throw new ApiError(
      `Invalid JSON from ${path}: ${error instanceof Error ? error.message : String(error)}`,
      path,
      response.status,
      text,
    );
  }
}

export function errorMessage(error: unknown): string {
  if (error instanceof ApiError) return error.message;
  if (error instanceof Error) return error.message;
  return String(error);
}

export async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      Accept: "application/json",
      ...(init?.headers ?? {}),
    },
  });
  if (!response.ok) {
    const text = await response.text();
    throw new ApiError(
      `${response.status} ${path}: ${text.trim() || response.statusText || "Request failed"}`,
      path,
      response.status,
      text,
    );
  }
  const parsed = await parseResponseBody(response, path);
  return parsed as T;
}

export async function postJson<T>(path: string, body: unknown, init?: RequestInit): Promise<T> {
  return fetchJson<T>(path, {
    ...init,
    method: "POST",
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
    body: JSON.stringify(body ?? {}),
  });
}

export async function putJson<T>(path: string, body: unknown, init?: RequestInit): Promise<T> {
  return fetchJson<T>(path, {
    ...init,
    method: "PUT",
    headers: {
      "content-type": "application/json",
      ...(init?.headers ?? {}),
    },
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

export type InvestigationTrace = {
  trace_id?: string;
  trace_kind: string;
  sanitized_system_prompt: string;
  sanitized_user_prompt: string;
  allowed_fields: string[];
  blocked_fields: string[];
  raw_logs_sent: boolean;
  trace_schema_version?: number;
  created_at?: string;
};

export type PersistedExplanation = {
  explanation_id: string;
  summary: string;
  primary_text: string;
  evidence_text?: string | null;
  timeline_text?: string | null;
  alternatives: string[];
  actions: string[];
  uncertainty: string[];
  model_used: string;
  guardrail_flags: string[];
  created_at: string;
  explanation_schema_version: number;
  hypotheses_hash: string;
  events_hash_head: string;
  quality: string;
};

export type InvestigationAudit = {
  incident_id: string;
  explanation?: PersistedExplanation | null;
  latest_trace?: InvestigationTrace | null;
  feedback?: Array<{
    feedback_id: string;
    correct_hypothesis_id?: string | null;
    feedback_type: string;
    operator_notes: string;
    resolved_at: string;
    created_at: string;
  }>;
  state_log?: Array<{
    old_state?: string;
    new_state?: string;
    changed_at?: string;
    reason?: string;
  }>;
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
  warnings?: string[];
  attempts?: number;
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
    schema_version?: number;
  } | null;
  audit?: InvestigationAudit;
  bundle?: Record<string, unknown>;
  focus?: string;
  question?: string;
  report?: boolean;
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

export type HypothesisRow = {
  hypothesis_id: string;
  cause_type: string;
  description?: string;
  total_score?: number;
  confidence_label?: string;
  suggested_checks?: string[];
};

export type EventRow = {
  event_id?: string;
  timestamp?: string;
  severity?: number | string;
  service_id?: string;
  message?: string;
  summary?: string;
  source_ref?: { source_type?: string };
  tags?: string[];
};

export type IncidentDetailResponse = {
  incident: IncidentRow;
  events: EventRow[];
  hypotheses: HypothesisRow[];
  clusters: unknown[];
  explanation?: PersistedExplanation | null;
  latest_trace?: InvestigationTrace | null;
  state_log?: Array<{
    old_state?: string;
    new_state?: string;
    changed_at?: string;
    reason?: string;
  }>;
  feedback?: Array<{
    feedback_id: string;
    correct_hypothesis_id?: string | null;
    feedback_type: string;
    operator_notes: string;
    resolved_at: string;
    created_at: string;
  }>;
};

export type AnomalyStatus = {
  enabled: boolean;
  service_id: string;
  status: string;
  window_hours: number;
  event_count: number;
  error_count: number;
  last_event_at?: string | null;
  buckets: Record<string, number>;
};

export type EventDetailResponse = {
  event: EventRow;
};

export type TopologyEdge = {
  source: string;
  target: string;
  relation_type?: string;
  type?: string;
  notes?: string;
};

export type ServiceDetailResponse = {
  service: ServiceRow;
  events: EventRow[];
  incidents: IncidentRow[];
};
