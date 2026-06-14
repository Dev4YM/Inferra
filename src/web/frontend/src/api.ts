export const INFERRA_AUTH_TOKEN_STORAGE_KEY = "inferra_auth_token";

/** Optional bearer token for protected `/api/*` routes (matches `server.auth_token_env` on the host). */
export function getInferraAuthToken(): string {
  if (typeof sessionStorage === "undefined") return "";
  return sessionStorage.getItem(INFERRA_AUTH_TOKEN_STORAGE_KEY)?.trim() ?? "";
}

export function setInferraAuthToken(token: string): void {
  if (typeof sessionStorage === "undefined") return;
  const trimmed = token.trim();
  if (!trimmed) {
    sessionStorage.removeItem(INFERRA_AUTH_TOKEN_STORAGE_KEY);
    return;
  }
  sessionStorage.setItem(INFERRA_AUTH_TOKEN_STORAGE_KEY, trimmed);
}

function apiAuthHeaders(path: string, init?: RequestInit): HeadersInit {
  const headers: Record<string, string> = {};
  if (path.startsWith("/api/") || path.startsWith("/v1/")) {
    const token = getInferraAuthToken();
    if (token) {
      headers.Authorization = `Bearer ${token}`;
    }
  }
  return { ...headers, ...(init?.headers as Record<string, string> | undefined) };
}

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

export function parseApiErrorBody(body: string): string {
  const trimmed = body.trim();
  if (!trimmed) return "";
  try {
    const parsed = JSON.parse(trimmed) as unknown;
    if (parsed && typeof parsed === "object") {
      const record = parsed as Record<string, unknown>;
      for (const key of ["detail", "message", "error"]) {
        const value = record[key];
        if (typeof value === "string" && value.trim()) return value.trim();
      }
    }
  } catch {
    // Fall through to plain text.
  }
  return trimmed;
}

export function apiErrorDetail(error: ApiError): string {
  const detail = parseApiErrorBody(error.body);
  if (error.status === 401) return detail || "API token required or invalid. Add the bearer token in Settings.";
  if (error.status === 403) {
    return detail === "local clients only"
      ? "This API only accepts local clients. Use a local browser, tunnel, or protected reverse proxy."
      : detail || "Request forbidden by the Inferra API.";
  }
  if (error.status === 404) return detail || "The requested Inferra resource was not found.";
  if (error.status === 429) return detail || "Rate limit exceeded. Wait briefly before trying again.";
  if (error.status === 503) {
    return detail.includes("auth_token_env")
      ? "Inferra is configured to require an API token, but the server token environment variable is not set."
      : detail || "Inferra is temporarily unavailable.";
  }
  return detail || error.message;
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

function networkErrorDetail(error: unknown, path: string): string {
  const message = error instanceof Error ? error.message : String(error);
  const lowered = message.toLowerCase();
  if (lowered.includes("failed to fetch") || lowered.includes("networkerror") || lowered.includes("load failed")) {
    return `Cannot reach the Inferra API at ${path}. The Rust runtime may be stopped, hung, or listening on a different port than the dev proxy expects. Check Control → Inferra runtime or run \`inferra service status\`.`;
  }
  if (lowered.includes("aborted") || lowered.includes("abort")) {
    return "Request to the Inferra API was cancelled.";
  }
  if (lowered.includes("timeout") || lowered.includes("timed out")) {
    return `Inferra API timed out on ${path}. The process may be hung — restart \`inferra serve\` or the Windows Inferra service.`;
  }
  return message || "Network request to the Inferra API failed.";
}

export function errorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    if (error.status === 0) return networkErrorDetail(error, error.path);
    return apiErrorDetail(error);
  }
  if (error instanceof Error) return error.message;
  return String(error);
}

export async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      ...init,
      headers: {
        Accept: "application/json",
        ...apiAuthHeaders(path, init),
      },
    });
  } catch (error) {
    throw new ApiError(networkErrorDetail(error, path), path, 0, "");
  }
  if (!response.ok) {
    const text = await response.text();
    const detail = parseApiErrorBody(text) || response.statusText || "Request failed";
    throw new ApiError(
      `${response.status} ${path}: ${detail}`,
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

export type AdaptiveArtifactKind = "detector" | "template" | "composition" | "edge_profile";
export type AdaptiveReviewDecision = "approve" | "watch" | "reject" | "reset";
export type AdaptiveRuntimeAction = "enable" | "disable";

export type AdaptiveAuditEntry = {
  audit_id: string;
  artifact_kind: string;
  artifact_id: string;
  action: string;
  reason?: string | null;
  previous_status?: string | null;
  new_status?: string | null;
  review_status_before?: string | null;
  review_status_after?: string | null;
  runtime_effect?: string | null;
  created_at?: string | null;
};

export type AdaptiveArtifactCounts = {
  detectors: number;
  templates: number;
  compositions: number;
  edge_profiles: number;
  active_detectors: number;
  active_templates: number;
  active_compositions: number;
  active_edge_profiles: number;
  manually_disabled: number;
};

type AdaptiveArtifactBase = {
  cause_type?: string | null;
  confirmations: number;
  false_positives: number;
  manually_disabled: boolean;
  status?: string;
  status_reason?: string | null;
  review_status?: string;
  review_reason?: string | null;
  last_reviewed_at?: string | null;
  created_from_feedback_id?: string | null;
  updated_at?: string | null;
};

export type AdaptiveDetector = AdaptiveArtifactBase & {
  detector_id: string;
  requirement_name: string;
  positive_terms: string[];
  tags: string[];
  source_types: string[];
  min_severity?: number | null;
};

export type AdaptiveTemplate = AdaptiveArtifactBase & {
  template_id: string;
  template_name: string;
  cause_subtype?: string | null;
  title_template: string;
  confidence: number;
  requires: string[];
  requires_same_service: boolean;
  requires_temporal_order: boolean;
};

export type AdaptiveComposition = AdaptiveTemplate & {
  composition_id: string;
  composition_name: string;
  preferred_edge_types: string[];
};

export type AdaptiveEdgeProfile = AdaptiveArtifactBase & {
  profile_id: string;
  edge_type: string;
  source_service?: string | null;
  target_service?: string | null;
  average_plausibility: number;
  average_latency_ms: number;
};

export type AdaptiveReviewQueueItem = {
  artifact_kind: AdaptiveArtifactKind;
  artifact_id: string;
  label: string;
  status: string;
  review_status: string;
  confirmations: number;
  false_positives: number;
  updated_at?: string | null;
};

export type AdaptiveIncidentInfluence = {
  incident_id: string;
  state?: string;
  primary_service?: string | null;
  severity?: number;
  learning: {
    incident_id: string;
    influenced_hypotheses: number;
    estimated_total_impact: number;
    artifacts: AdaptiveInfluenceArtifact[];
  };
};

export type AdaptiveInfluenceArtifact = {
  kind?: string | null;
  artifact_id?: string | null;
  label?: string | null;
  reason?: string | null;
  impact_metric?: string | null;
  impact_value?: number | null;
  cumulative_impact?: number;
};

export type HypothesisLearningProvenance = {
  has_learned_influence?: boolean;
  estimated_total_impact?: number;
  artifacts?: AdaptiveInfluenceArtifact[];
};

export type IncidentLearningProvenance = {
  incident_id: string;
  influenced_hypotheses: number;
  estimated_total_impact: number;
  artifacts: AdaptiveInfluenceArtifact[];
};

export type AdaptiveHistorySummaryArtifact = {
  artifact_kind: string;
  artifact_id: string;
  artifact_label?: string | null;
  observations: number;
  latest_observed_at?: string | null;
  latest_score?: number | null;
  latest_rank?: number | null;
  best_rank?: number | null;
  cumulative_score_delta?: number;
  cumulative_edge_delta?: number;
  latest_estimated_impact?: number;
};

export type AdaptiveHistorySummary = {
  path: string;
  artifacts: AdaptiveHistorySummaryArtifact[];
  count: number;
};

export type AdaptiveComparisonRow = {
  artifact_kind: AdaptiveArtifactKind;
  artifact_id: string;
  label: string;
  status: string;
  review_status: string;
  confirmations: number;
  false_positives: number;
  noise_ratio: number;
  manually_disabled: boolean;
  attention: boolean;
  updated_at?: string | null;
  last_reviewed_at?: string | null;
  pending_review_age_hours?: number | null;
  watch_age_hours?: number | null;
  aging_bucket: "fresh" | "warm" | "stale" | "aged";
  history_observations: number;
  latest_estimated_impact?: number | null;
  latest_score?: number | null;
  best_rank?: number | null;
  cumulative_score_delta?: number | null;
  cumulative_edge_delta?: number | null;
  active_incident_count: number;
  active_cumulative_impact: number;
  incident_ids: string[];
};

export type AdaptiveKindBreakdown = {
  artifact_kind: string;
  total: number;
  unreviewed: number;
  attention: number;
  manually_disabled: number;
};

export type AdaptiveReviewAnalytics = {
  kind_breakdown: AdaptiveKindBreakdown[];
  top_confirmed: AdaptiveComparisonRow[];
  top_noisy: AdaptiveComparisonRow[];
  top_impact: AdaptiveComparisonRow[];
  recently_changed: AdaptiveComparisonRow[];
};

export type AdaptiveSavedReviewViewSelection = {
  artifact_kind: AdaptiveArtifactKind;
  artifact_id: string;
};

export type AdaptiveSavedReviewView = {
  view_id: string;
  name: string;
  description?: string | null;
  search_text?: string | null;
  assigned_reviewer?: string | null;
  created_at: string;
  updated_at: string;
  last_used_at?: string | null;
  artifact_selections: AdaptiveSavedReviewViewSelection[];
  match_count: number;
  pending_review_count: number;
  attention_count: number;
  active_incident_count: number;
  active_cumulative_impact: number;
  oldest_pending_age_hours?: number | null;
  oldest_pending_label?: string | null;
  aging_bucket: "fresh" | "warm" | "stale" | "aged";
  stale_pending: boolean;
};

export type AdaptiveTrendObservation = {
  observed_at: string;
  incident_id: string;
  hypothesis_id: string;
  score?: number | null;
  rank?: number | null;
  estimated_impact: number;
  impact_metric?: string | null;
  score_delta?: number | null;
  rank_delta?: number | null;
  edge_delta?: number | null;
};

export type AdaptiveTrendDrilldown = {
  artifact_kind: AdaptiveArtifactKind;
  artifact_id: string;
  artifact_label: string;
  observation_count: number;
  total_abs_delta: number;
  observations: AdaptiveTrendObservation[];
};

export type AdaptiveLearningSummaryResponse = {
  schema_version: number;
  last_updated?: string | null;
  path: string;
  audit_path: string;
  processed_feedback_count: number;
  counts: AdaptiveArtifactCounts;
  detectors: AdaptiveDetector[];
  templates: AdaptiveTemplate[];
  compositions: AdaptiveComposition[];
  edge_profiles: AdaptiveEdgeProfile[];
  recent_audit: AdaptiveAuditEntry[];
};

export type AdaptiveLearningReviewResponse = {
  summary: AdaptiveLearningSummaryResponse;
  active_incident_influence: AdaptiveIncidentInfluence[];
  artifacts_requiring_attention: Array<Record<string, unknown>>;
  review_counts: Record<string, number>;
  review_queue: AdaptiveReviewQueueItem[];
  recent_review_activity: AdaptiveAuditEntry[];
  history_summary: AdaptiveHistorySummary;
  comparison_rows: AdaptiveComparisonRow[];
  saved_views: AdaptiveSavedReviewView[];
  trend_drilldowns: AdaptiveTrendDrilldown[];
  analytics: AdaptiveReviewAnalytics;
};

export type AdaptiveReviewMutationResponse = {
  updated: boolean;
  artifact_kind: string;
  artifact_id: string;
  decision: string;
  review: AdaptiveLearningReviewResponse;
};

export type AdaptiveStateMutationResponse = {
  updated: boolean;
  artifact_kind: string;
  artifact_id: string;
  action: string;
  learning: AdaptiveLearningSummaryResponse;
};

export type AdaptiveBulkMutationArtifact = {
  artifact_kind: AdaptiveArtifactKind;
  artifact_id: string;
  label: string;
  previous_status: string;
  new_status: string;
  review_status_before?: string | null;
  review_status_after?: string | null;
  runtime_effect?: string | null;
  updated_at?: string | null;
};

export type AdaptiveBulkReviewMutationResponse = {
  updated: boolean;
  updated_count: number;
  decision: string;
  artifacts: AdaptiveBulkMutationArtifact[];
  review: AdaptiveLearningReviewResponse;
};

export type AdaptiveBulkStateMutationResponse = {
  updated: boolean;
  updated_count: number;
  action: string;
  artifacts: AdaptiveBulkMutationArtifact[];
  review: AdaptiveLearningReviewResponse;
};

export type AdaptiveSavedReviewViewMutationResponse = {
  updated?: boolean;
  deleted?: boolean;
  view_id: string;
  used_at?: string | null;
  review: AdaptiveLearningReviewResponse;
};

export async function reviewAdaptiveArtifact(
  artifactKind: string,
  artifactId: string,
  body: { decision: AdaptiveReviewDecision; reason?: string },
): Promise<AdaptiveReviewMutationResponse> {
  return postJson<AdaptiveReviewMutationResponse>(
    `/api/learning/adaptive/${encodeURIComponent(artifactKind)}/${encodeURIComponent(artifactId)}/review`,
    body,
  );
}

export async function setAdaptiveArtifactState(
  artifactKind: string,
  artifactId: string,
  body: { action: AdaptiveRuntimeAction; reason?: string },
): Promise<AdaptiveStateMutationResponse> {
  return postJson<AdaptiveStateMutationResponse>(
    `/api/learning/adaptive/${encodeURIComponent(artifactKind)}/${encodeURIComponent(artifactId)}`,
    body,
  );
}

export async function reviewAdaptiveArtifactsBulk(
  artifacts: Array<{ artifact_kind: AdaptiveArtifactKind; artifact_id: string }>,
  body: { decision: AdaptiveReviewDecision; reason?: string },
): Promise<AdaptiveBulkReviewMutationResponse> {
  return postJson<AdaptiveBulkReviewMutationResponse>("/api/learning/adaptive/bulk/review", {
    artifacts,
    ...body,
  });
}

export async function setAdaptiveArtifactsStateBulk(
  artifacts: Array<{ artifact_kind: AdaptiveArtifactKind; artifact_id: string }>,
  body: { action: AdaptiveRuntimeAction; reason?: string },
): Promise<AdaptiveBulkStateMutationResponse> {
  return postJson<AdaptiveBulkStateMutationResponse>("/api/learning/adaptive/bulk/state", {
    artifacts,
    ...body,
  });
}

export async function saveAdaptiveReviewView(body: {
  view_id?: string;
  name: string;
  description?: string;
  search_text?: string;
  assigned_reviewer?: string;
  artifacts: Array<{ artifact_kind: AdaptiveArtifactKind; artifact_id: string }>;
}): Promise<AdaptiveSavedReviewViewMutationResponse> {
  return postJson<AdaptiveSavedReviewViewMutationResponse>("/api/learning/adaptive/views", body);
}

export async function deleteAdaptiveReviewView(viewId: string): Promise<AdaptiveSavedReviewViewMutationResponse> {
  return fetchJson<AdaptiveSavedReviewViewMutationResponse>(`/api/learning/adaptive/views/${encodeURIComponent(viewId)}`, {
    method: "DELETE",
  });
}

export async function useAdaptiveReviewView(viewId: string): Promise<AdaptiveSavedReviewViewMutationResponse> {
  return postJson<AdaptiveSavedReviewViewMutationResponse>(
    `/api/learning/adaptive/views/${encodeURIComponent(viewId)}/use`,
    {},
  );
}

export type InvestigateStreamCallbacks = {
  onMeta?: (jsonLine: string) => void;
  onDelta?: (text: string) => void;
};

/** POST `/api/ai/investigate-stream` (SSE). Parses `meta`, `delta`, `done`, `error` events until `done`. */
export async function postInvestigateStream(
  body: { question: string; scope: string; mode: string; monitor_seconds: number },
  callbacks?: InvestigateStreamCallbacks,
  signal?: AbortSignal,
): Promise<InvestigationResponse> {
  const path = "/api/ai/investigate-stream";
  const response = await fetch(path, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "text/event-stream",
      ...apiAuthHeaders(path),
    },
    body: JSON.stringify(body),
    signal,
  });
  if (!response.ok) {
    const text = await response.text();
    const detail = parseApiErrorBody(text) || response.statusText || "Request failed";
    throw new ApiError(
      `${response.status} ${path}: ${detail}`,
      path,
      response.status,
      text,
    );
  }
  if (!response.body) {
    throw new ApiError(`No response body from ${path}`, path, response.status);
  }

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let carry = "";
  let lastDone: InvestigationResponse | null = null;

  const processCarry = () => {
    const sep = "\n\n";
    while (true) {
      const pos = carry.indexOf(sep);
      if (pos === -1) break;
      const block = carry.slice(0, pos).trim();
      carry = carry.slice(pos + sep.length);
      if (!block || block.startsWith(":")) {
        continue;
      }
      let eventName = "message";
      const dataLines: string[] = [];
      for (const rawLine of block.split("\n")) {
        const line = rawLine.replace(/\r$/, "");
        if (!line || line.startsWith(":")) continue;
        if (line.startsWith("event:")) {
          eventName = line.slice(6).trim();
        } else if (line.startsWith("data:")) {
          dataLines.push(line.slice(5).trimStart());
        }
      }
      const data = dataLines.join("\n");
      if (eventName === "meta") {
        callbacks?.onMeta?.(data);
      } else if (eventName === "delta") {
        callbacks?.onDelta?.(data);
      } else if (eventName === "done") {
        try {
          lastDone = JSON.parse(data) as InvestigationResponse;
        } catch (error) {
          throw new ApiError(
            `Invalid JSON in done event: ${error instanceof Error ? error.message : String(error)}`,
            path,
            502,
            data,
          );
        }
      } else if (eventName === "error") {
        throw new ApiError(data || "stream error", path, 502, data);
      }
    }
  };

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    carry += decoder.decode(value, { stream: true });
    processCarry();
  }
  carry += decoder.decode();
  processCarry();

  if (!lastDone) {
    throw new ApiError("Stream ended without a valid done event", path, 502);
  }
  return lastDone;
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

export type HealthResponse = {
  status: string;
  runtime: string;
  storage_writes_ok: boolean;
  degraded_reasons?: string[];
  config_path?: string;
  data_dir?: string;
  events_db?: string;
  incidents_db?: string;
  ai_enabled?: boolean;
};

export type ProbeHealthResponse = {
  status: string;
  runtime: string;
  storage_writes_ok?: boolean;
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

export type IncidentRow = {
  incident_id: string;
  state: string;
  severity: number;
  primary_service: string;
  affected_services?: string[];
  created_at?: string;
  updated_at?: string;
  event_count?: number;
  latest_trace_summary?: TraceSummary | null;
};

export type TraceSummary = {
  trace_id: string;
  event_count: number;
  last_seen_at?: string | null;
  service_id?: string | null;
  source_type?: string | null;
  signal_kind?: string | null;
  deployment_environment?: string | null;
  severity?: number | string | null;
  sample_message?: string | null;
};

export type ServiceRow = {
  service_id: string;
  status: string;
  event_count?: number;
  error_count?: number;
  error_ratio?: number;
  last_event_at?: string | null;
  active_incidents?: IncidentRow[];
  latest_trace_summary?: TraceSummary | null;
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
  event_rate?: unknown;
  severity_counts?: unknown;
};

export type RuntimeContext = {
  hostname?: string;
  containers?: { name: string; image: string; state: string }[];
  processes?: {
    pid: number;
    name: string;
    cpu_percent: number;
    cpu_raw_percent?: number | null;
    cpu_percent_scope?: string | null;
    cpu_logical_processors?: number | null;
    memory_mb: number;
  }[];
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
  last_error_at?: string | null;
  error_hint?: string | null;
  log_query?: string | null;
  lag_seconds?: number;
};

export type ScannerStatusResponse = {
  scanner: Record<
    string,
    {
      data_type: string;
      mode: string;
      route?: string;
      interval_seconds?: number;
      min_interval_seconds?: number;
      max_interval_seconds?: number;
      last_scanned_at?: string | null;
      age_seconds?: number;
      next_scan_in_seconds?: number;
      cached?: boolean;
    }
  >;
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
  status_model?: string;
  investigate_model?: string;
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

export type InvestigationGrounding = {
  removed_evidence_ids: string[];
  removed_citation_ids: string[];
};

export type InvestigationResponse = {
  schema_version: number;
  output: InvestigationOutput;
  used_ai: boolean;
  fallback_reason: string;
  warnings?: string[];
  attempts?: number;
  grounding?: InvestigationGrounding;
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
  cached?: boolean;
  ai_generation?: {
    generation_id: string;
    scope_key: string;
    focus: string;
    mode: string;
    question: string;
    bundle_hash: string;
    used_ai: boolean;
    created_at: string;
  };
};

export type AiGeneration = {
  generation_id: string;
  scope_key: string;
  focus: string;
  mode: string;
  question: string;
  response: InvestigationResponse;
  bundle_hash: string;
  used_ai: boolean;
  provider?: Record<string, unknown> | null;
  created_at: string;
};

export type AiGenerationsResponse = {
  generations: AiGeneration[];
  count: number;
};

export type WorkspaceMappingSignal = { name: string; confidence: number; detail: string };

export type WorkspaceLogSource = {
  kind: string;
  label: string;
  path?: string | null;
  command?: string | null;
  stream?: string | null;
  exists?: boolean | null;
  readable?: boolean | null;
  source: string;
  confidence: number;
};

export type WorkspaceAppEndpoint = {
  url: string;
  host?: string | null;
  port?: number | null;
  protocol: string;
  source: string;
  confidence: number;
};

export type WorkspaceAppLocation = {
  project_path?: string | null;
  cwd?: string | null;
  script?: string | null;
  executable?: string | null;
  installation_dir?: string | null;
};

export type WorkspaceAppResources = {
  cpu_percent?: number | null;
  cpu_raw_percent?: number | null;
  cpu_percent_scope?: string | null;
  cpu_logical_processors?: number | null;
  memory_mb?: number | null;
  virtual_memory_mb?: number | null;
  uptime_seconds?: number | null;
  process_status?: string | null;
};

export type WorkspaceAppResourcesResponse = {
  app_name: string;
  pid?: number | null;
  sampled_at: string;
  live: boolean;
  resources?: WorkspaceAppResources | null;
};

export type WorkspaceAppRawLog = {
  source?: {
    kind?: string | null;
    label?: string | null;
    path?: string | null;
    command?: string | null;
    stream?: string | null;
    source?: string | null;
    confidence?: number | null;
  } | null;
  line: string;
  line_number_from_tail?: number | null;
  sampled_at?: string | null;
};

export type WorkspaceAppLogsResponse = {
  app_name: string;
  events: EventRow[];
  raw_logs: WorkspaceAppRawLog[];
  log_sources: WorkspaceLogSource[];
  sampled_at: string;
};

export type WorkspaceAppState = {
  health: string;
  status?: string | null;
  reason?: string | null;
  started_at?: string | null;
  restarts?: number | null;
  observed_by: string;
};

export type WorkspaceAppCapability = {
  key: string;
  supported: boolean;
  source: string;
  detail?: string | null;
};

export type WorkspaceRuntimeApp = {
  pid?: number | null;
  name: string;
  display_name?: string | null;
  runtime: string;
  language?: string | null;
  process_kind?: string | null;
  framework?: string | null;
  libraries?: string[];
  log_hints?: string[];
  log_sources?: WorkspaceLogSource[];
  app_url?: string | null;
  endpoints?: WorkspaceAppEndpoint[];
  health_endpoint?: WorkspaceAppEndpoint | null;
  app_location?: WorkspaceAppLocation | null;
  resources?: WorkspaceAppResources | null;
  app_state?: WorkspaceAppState | null;
  context_capabilities?: WorkspaceAppCapability[];
  app_structure?: Array<{ path: string; kind: string; role: string }>;
  manager?: string | null;
  status?: string | null;
  cwd?: string | null;
  script?: string | null;
  command?: string | null;
  project_path?: string | null;
  latest_trace_summary?: TraceSummary | null;
  confidence: number;
  source: string;
  signals: WorkspaceMappingSignal[];
};

export type WorkspaceSupportItem = {
  id: string;
  label: string;
  support_type: string;
  detects: string[];
  log_hints?: string[];
  children?: WorkspaceSupportItem[];
};

export type WorkspaceSupportLayer = {
  layer: string;
  title: string;
  description: string;
  items: WorkspaceSupportItem[];
};

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
  support_layers?: WorkspaceSupportLayer[];
  projects: WorkspaceProject[];
  runtime_apps?: WorkspaceRuntimeApp[];
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
  investigate_model?: string;
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
  rank?: number;
  description?: string;
  total_score?: number;
  confidence_label?: string;
  suggested_checks?: string[];
  provenance?: HypothesisLearningProvenance;
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
  trace_id?: string;
  span_id?: string;
  signal_kind?: string;
  deployment_environment?: string;
  severity_text?: string;
};

/** Response from `GET /api/v2/logs` (trace-aware normalized log explorer with keyset pagination). */
export type LogsV2Response = {
  items: EventRow[];
  limit: number;
  retention_hours: number;
  log_fts_enabled: boolean;
  next_cursor?: { cursor_timestamp?: string | null; cursor_event_id?: string | null } | null;
};

export function apiLogsV2Path(query?: Record<string, string | number | undefined>): string {
  const base = "/api/v2/logs";
  if (!query) return base;
  const sp = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined) continue;
    sp.set(k, String(v));
  }
  const qs = sp.toString();
  return qs ? `${base}?${qs}` : base;
}

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
  learning_provenance?: IncidentLearningProvenance | null;
};

/** Response from `GET /api/incidents/{incident_id}/logs` (log-shaped rows, pagination). */
export type IncidentLogsResponse = {
  incident_id: string;
  logs: EventRow[];
  limit: number;
  retention_hours: number;
  next_cursor?: { cursor_timestamp: string; cursor_event_id: string } | null;
};

export function apiIncidentLogsPath(
  incidentId: string,
  query?: Record<string, string | number | undefined>,
): string {
  const base = `/api/incidents/${encodeURIComponent(incidentId)}/logs`;
  if (!query) return base;
  const sp = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined) continue;
    sp.set(k, String(v));
  }
  const qs = sp.toString();
  return qs ? `${base}?${qs}` : base;
}

/** Response from `GET /api/traces/{trace_id}` (chronological log-shaped rows for one W3C trace id). */
export type TraceTimelineResponse = {
  trace_id: string;
  items: EventRow[];
  limit: number;
  retention_hours: number;
  count: number;
};

/** Build path for trace timeline; `traceId` is normalized to 32 lowercase hex (non-hex stripped) before encoding. */
export function apiTraceTimelinePath(
  traceId: string,
  query?: Record<string, string | number | undefined>,
): string {
  const normalized = traceId.replace(/[^0-9a-fA-F]/g, "").toLowerCase();
  const base = `/api/traces/${encodeURIComponent(normalized)}`;
  if (!query) return base;
  const sp = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) {
    if (v === undefined) continue;
    sp.set(k, String(v));
  }
  const qs = sp.toString();
  return qs ? `${base}?${qs}` : base;
}

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
