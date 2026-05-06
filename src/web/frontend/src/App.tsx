import { useCallback, useEffect, useMemo, useState } from "react";
import { NavLink, Route, Routes, useParams } from "react-router-dom";
import {
  type AiDoctorResponse,
  type AiStatus,
  type CollectorRow,
  type ConfigResponse,
  type IncidentRow,
  type InferraConfigPayload,
  type InvestigationResponse,
  type OverviewResponse,
  type ServiceRow,
  type WorkspaceMapResponse,
  fetchJson,
  postJson,
  putJson,
} from "./api";

type Mode = "operator" | "developer";

const MODE_KEY = "inferra.mode";

function useMode(initial: Mode = "operator"): [Mode, (mode: Mode) => void] {
  const [mode, setMode] = useState<Mode>(() => {
    if (typeof window === "undefined") return initial;
    const stored = window.localStorage.getItem(MODE_KEY);
    if (stored === "operator" || stored === "developer") return stored;
    return initial;
  });
  useEffect(() => {
    if (typeof window !== "undefined") window.localStorage.setItem(MODE_KEY, mode);
  }, [mode]);
  return [mode, setMode];
}

function useJson<T>(path: string | null, deps: unknown[] = []) {
  const [data, setData] = useState<T | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const reload = useCallback(() => {
    if (path === null) return;
    let alive = true;
    setLoading(true);
    fetchJson<T>(path)
      .then((d) => {
        if (alive) {
          setData(d);
          setErr(null);
        }
      })
      .catch((e: Error) => {
        if (alive) setErr(e.message);
      })
      .finally(() => {
        if (alive) setLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [path]);
  useEffect(() => {
    if (path === null) return;
    return reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, ...deps]);
  return { data, err, loading, reload };
}

function isAdvancedMode(mode: Mode): boolean {
  return mode === "developer";
}

function ModeToggle({
  mode,
  setMode,
  status,
}: {
  mode: Mode;
  setMode: (mode: Mode) => void;
  status?: string;
}) {
  return (
    <>
      <div className="mode-toggle">
        <button className={mode === "operator" ? "active" : ""} onClick={() => setMode("operator")}>
          Operator
        </button>
        <button className={mode === "developer" ? "active" : ""} onClick={() => setMode("developer")}>
          Developer
        </button>
      </div>
      {status ? <div className="mode-status">{status}</div> : null}
    </>
  );
}

function configMode(config: InferraConfigPayload | null | undefined): Mode | null {
  const value = config?.experience?.mode;
  return value === "operator" || value === "developer" ? value : null;
}

function CommandCard({
  title,
  command,
  note,
}: {
  title: string;
  command: string;
  note: string;
}) {
  return (
    <div className="command-card">
      <div>
        <strong>{title}</strong>
        <p className="muted">{note}</p>
      </div>
      <code>{command}</code>
    </div>
  );
}

function FirstRunGuide({ mode, hasIncidents, aiEnabled }: { mode: Mode; hasIncidents: boolean; aiEnabled: boolean }) {
  const profile = mode === "developer" ? "developer" : "operator";
  return (
    <section className="guide-panel">
      <div>
        <span className="eyebrow">First-run path</span>
        <h2>Make Inferra useful in five commands</h2>
        <p className="muted">
          This dashboard mirrors the CLI control plane. Start with the guide command, then run only the checks you choose.
          Inferra observes and suggests; it does not mutate the systems it watches.
        </p>
      </div>
      <div className="guide-grid">
        <CommandCard
          title="Let the CLI pick the path"
          command={`inferra guide --profile ${profile}`}
          note="Shows the next best setup, AI, service, workspace, and investigation steps for your mode."
        />
        <CommandCard
          title="Open this control plane"
          command="inferra dashboard"
          note="Opens the local web UI, checks API reachability, and can jump to sections with --section."
        />
        <CommandCard
          title="Validate readiness"
          command="inferra doctor --release"
          note="Checks docs, packaged UI, repo hygiene, package metadata, and generated artifact state."
        />
        <CommandCard
          title={hasIncidents ? "Investigate what matters" : "Create demo evidence"}
          command={hasIncidents ? "inferra investigate latest" : "inferra demo seed"}
          note={hasIncidents ? "Builds an evidence-backed investigation plan." : "Adds safe local demo data so the UI has something to explain."}
        />
        <CommandCard
          title={aiEnabled ? "Check AI safety" : "Enable AI when ready"}
          command={aiEnabled ? "inferra ai doctor" : "inferra ai setup --enable --model gemma4:e4b --skip-connection-test"}
          note={aiEnabled ? "Reviews provider, redaction, and remote-risk policy." : "Configures the local-first AI investigator without requiring a live model probe."}
        />
      </div>
    </section>
  );
}

function Severity({ value }: { value: number | string }) {
  const label = ((): string => {
    if (typeof value === "number") {
      const labels = ["debug", "info", "warn", "error", "critical"];
      return labels[Math.max(0, Math.min(4, value))] ?? "info";
    }
    return String(value || "info");
  })();
  const cls =
    label === "error" || label === "critical"
      ? "tag tag-error"
      : label === "warn"
      ? "tag tag-warn"
      : label === "info"
      ? "tag tag-info"
      : "tag";
  return <span className={cls}>{label}</span>;
}

function PageHeader({ title, subtitle, mode }: { title: string; subtitle?: string; mode?: string }) {
  return (
    <div className="page-header">
      <div>
        <h2 className="page-title">{title}</h2>
        {subtitle ? <p className="page-subtitle">{subtitle}</p> : null}
      </div>
      {mode ? <span className={`mode-pill ${mode}`}>{mode} mode</span> : null}
    </div>
  );
}

function OverviewPage({ mode }: { mode: Mode }) {
  const { data, err, loading, reload } = useJson<OverviewResponse>("/api/overview");
  if (err) return <p className="muted">Could not load overview: {err}</p>;
  if (!data) return <p className="muted">{loading ? "Loading observability snapshot…" : "No data"}</p>;
  const qa = data.quick_analysis;
  const dash = data.dashboard;
  const health = dash.health || {};
  const incidents = dash.incidents || [];
  const services = dash.services || [];
  const risky = services.filter((s) => s.status === "critical" || s.status === "degraded" || s.status === "elevated");
  const projects = data.workspace_projects || [];
  const top = incidents[0];
  return (
    <>
      <PageHeader
        title="Overview"
        subtitle="What is happening, what changed, what to inspect next."
        mode={qa.mode}
      />
      <div className={`banner risk-${qa.risk_level}`}>
        <div className="headline-row">
          <strong>Quick analysis</strong>
          <button onClick={reload}>Refresh</button>
        </div>
        <div style={{ marginTop: "0.4rem" }}>{qa.headline}</div>
        <div className="muted" style={{ marginTop: "0.35rem" }}>
          AI role: {qa.ai_role} · Safe actions:{" "}
          {data.experience.suggest_safe_actions && !data.experience.execute_actions ? "suggest only" : "custom policy"}
        </div>
      </div>

      <FirstRunGuide mode={mode} hasIncidents={incidents.length > 0} aiEnabled={Boolean(health.ai_enabled)} />

      <section className="section">
        <h2>Top concern</h2>
        {top ? (
          <div className="card-grid">
            <div className="card danger">
              <h3>Top incident</h3>
              <div className="value">{top.primary_service}</div>
              <div className="muted">
                <Severity value={top.severity} /> · {top.state} · {top.event_count ?? 0} events
              </div>
              <div>
                <NavLink to={`/incidents/${top.incident_id}`}>{top.incident_id}</NavLink>
              </div>
            </div>
            <div className="card">
              <h3>Active incidents</h3>
              <div className="value">{health.active_incidents ?? incidents.length}</div>
              <div className="muted">{services.length} services observed</div>
            </div>
            <div className="card">
              <h3>AI investigator</h3>
              <div className="value">
                {health.ai_enabled ? (health.ai_available ? "ready" : "unavailable") : "off"}
              </div>
              <div className="muted">{health.ai_reason || "deterministic engine always on"}</div>
            </div>
            <div className="card">
              <h3>Workspace</h3>
              <div className="value">{projects.length}</div>
              <div className="muted">code projects detected</div>
            </div>
          </div>
        ) : (
          <div className="card-grid">
            <div className="card">
              <h3>Status</h3>
              <div className="value">No active incidents</div>
              <div className="muted">{services.length} services observed · {health.queue_depth ?? 0} queued events</div>
            </div>
            <div className="card">
              <h3>AI investigator</h3>
              <div className="value">{health.ai_enabled ? (health.ai_available ? "ready" : "off") : "off"}</div>
              <div className="muted">{health.ai_reason || "—"}</div>
            </div>
            <div className="card">
              <h3>Workspace</h3>
              <div className="value">{projects.length}</div>
              <div className="muted">code projects detected</div>
            </div>
          </div>
        )}
      </section>

      <section className="section">
        <h2>Services needing attention</h2>
        {risky.length === 0 ? (
          <p className="muted">No services are degraded right now.</p>
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Service</th>
                  <th>Status</th>
                  <th>Events</th>
                  <th>Errors</th>
                </tr>
              </thead>
              <tbody>
                {risky.slice(0, 10).map((s) => (
                  <tr key={s.service_id} className={`status-${s.status}`}>
                    <td>
                      <span className="status-dot" />
                      <NavLink to={`/systems/${s.service_id}`}>{s.service_id}</NavLink>
                    </td>
                    <td>{s.status}</td>
                    <td>{s.event_count ?? 0}</td>
                    <td>{s.error_count ?? 0}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {isAdvancedMode(mode) ? (
        <section className="section">
          <h2>Diagnostics</h2>
          <div className="card-grid">
            <div className="card">
              <h3>Queue depth</h3>
              <div className="value">{health.queue_depth ?? 0}</div>
            </div>
            <div className="card">
              <h3>Storage writes</h3>
              <div className="value">{health.storage_writes_ok === false ? "degraded" : "ok"}</div>
              <div className="muted">free bytes: {health.data_dir_bytes_free ?? "?"}</div>
            </div>
            <div className="card">
              <h3>Degraded reasons</h3>
              <div className="muted">
                {(health.degraded_reasons || []).length > 0 ? (health.degraded_reasons || []).join(", ") : "none"}
              </div>
            </div>
            <div className="card">
              <h3>AI provider</h3>
              <div className="muted">
                enabled={String(health.ai_enabled ?? false)} · available={String(health.ai_available ?? false)}
              </div>
            </div>
          </div>
        </section>
      ) : null}
    </>
  );
}

function IncidentsPage({ mode }: { mode: Mode }) {
  const { data, err, reload } = useJson<{ incidents: IncidentRow[] }>("/api/incidents");
  if (err) return <p className="muted">{err}</p>;
  const rows = data?.incidents ?? [];
  return (
    <>
      <PageHeader title="Incidents" subtitle="Active investigations." mode={mode} />
      <div className="row" style={{ marginBottom: "0.6rem" }}>
        <button onClick={reload}>Refresh</button>
        <span className="muted">Showing {rows.length} active incident(s).</span>
      </div>
      {rows.length === 0 ? (
        <p className="muted">No active incidents. Try seeding demo data: <code>inferra demo seed</code>.</p>
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>ID</th>
                <th>State</th>
                <th>Severity</th>
                <th>Primary service</th>
                <th>Events</th>
                <th>Updated</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((i) => (
                <tr key={i.incident_id}>
                  <td>
                    <NavLink to={`/incidents/${i.incident_id}`}>{i.incident_id}</NavLink>
                  </td>
                  <td>{i.state}</td>
                  <td>
                    <Severity value={i.severity} />
                  </td>
                  <td>{i.primary_service || "—"}</td>
                  <td>{i.event_count ?? 0}</td>
                  <td className="muted">{i.updated_at || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function InvestigationView({
  result,
  showRaw,
  onRefresh,
}: {
  result: InvestigationResponse;
  showRaw: boolean;
  onRefresh?: () => void;
}) {
  const out = result.output;
  return (
    <>
      <div className={`banner risk-${out.risk_level}`}>
        <div className="headline-row">
          <strong>{out.headline || "(no headline)"}</strong>
          <span className="mode-pill">
            risk {out.risk_level} · {Math.round((out.confidence ?? 0) * 100)}% confidence
          </span>
        </div>
        {out.what_happened.length > 0 ? (
          <ul style={{ marginTop: "0.5rem", paddingLeft: "1.2rem" }}>
            {out.what_happened.map((line, idx) => (
              <li key={idx}>{line}</li>
            ))}
          </ul>
        ) : null}
        {!result.used_ai ? (
          <div className="muted" style={{ marginTop: "0.4rem" }}>
            Deterministic fallback used: {result.fallback_reason || "AI unavailable"}
          </div>
        ) : null}
      </div>

      {out.why_it_matters.length > 0 ? (
        <section className="section">
          <h2>Why it matters</h2>
          <ul>
            {out.why_it_matters.map((line, idx) => (
              <li key={idx}>{line}</li>
            ))}
          </ul>
        </section>
      ) : null}

      {out.likely_causes.length > 0 ? (
        <section className="section">
          <h2>Likely causes</h2>
          <ul>
            {out.likely_causes.map((line, idx) => (
              <li key={idx}>{line}</li>
            ))}
          </ul>
        </section>
      ) : null}

      <section className="section">
        <h2>Evidence</h2>
        {out.evidence.length === 0 ? (
          <p className="muted">No evidence cited.</p>
        ) : (
          <div className="evidence-list">
            {out.evidence.map((item, idx) => (
              <div className="evidence-item" key={`${item.type}-${item.id}-${idx}`}>
                <span className="evidence-type">{item.type}</span>
                <code>{item.id || "?"}</code> — {item.summary}
              </div>
            ))}
          </div>
        )}
      </section>

      <section className="section">
        <h2>Safe next steps</h2>
        {out.next_steps.length === 0 ? (
          <p className="muted">No next steps proposed.</p>
        ) : (
          out.next_steps.map((step, idx) => (
            <div className="next-step" key={idx}>
              <span className="next-step-title">
                {step.title}
                <span className="safety-pill">{step.safety || "read_only"}</span>
              </span>
              {step.reason ? <div className="muted">{step.reason}</div> : null}
              {step.command ? <code className="next-step-cmd">{step.command}</code> : null}
            </div>
          ))
        )}
      </section>

      {out.missing_evidence.length > 0 ? (
        <section className="section">
          <h2>Missing evidence</h2>
          <ul>
            {out.missing_evidence.map((line, idx) => (
              <li key={idx}>{line}</li>
            ))}
          </ul>
        </section>
      ) : null}

      {out.uncertainty.length > 0 ? (
        <section className="section">
          <h2>Uncertainty</h2>
          <div className="uncertainty-list">
            {out.uncertainty.map((line, idx) => (
              <div key={idx}>· {line}</div>
            ))}
          </div>
        </section>
      ) : null}

      <section className="section">
        <h2>Provider</h2>
        <p className="muted">
          enabled={String(result.provider.enabled)} · available={String(result.provider.available)} · model=
          {result.provider.model || "?"} · allow_remote={String(result.provider.allow_remote ?? false)}
        </p>
      </section>

      {showRaw && result.trace ? (
        <details open>
          <summary>Prompt trace (developer)</summary>
          <p className="muted">
            allowed_fields: {result.trace.allowed_fields.join(", ")} <br />
            blocked_fields: {result.trace.blocked_fields.join(", ")} <br />
            raw_logs_sent: {String(result.trace.raw_logs_sent)}
          </p>
          <pre className="code">{result.trace.sanitized_user_prompt}</pre>
        </details>
      ) : null}

      {showRaw ? (
        <details>
          <summary>Raw evidence bundle (developer)</summary>
          <pre className="code">{JSON.stringify(result.bundle ?? {}, null, 2)}</pre>
        </details>
      ) : null}

      {onRefresh ? (
        <div className="row" style={{ marginTop: "1rem" }}>
          <button onClick={onRefresh}>Re-run investigation</button>
        </div>
      ) : null}
    </>
  );
}

function IncidentDetailPage({ mode }: { mode: Mode }) {
  const { incidentId } = useParams();
  const path = incidentId ? `/api/incidents/${incidentId}` : null;
  const { data, err } = useJson<{
    incident: IncidentRow;
    events: Record<string, unknown>[];
    hypotheses: { hypothesis_id: string; cause_type: string; description?: string; total_score?: number; confidence_label?: string }[];
    clusters: unknown[];
  }>(path);
  const investigation = useJson<InvestigationResponse>(
    incidentId ? `/api/investigate/incident/${incidentId}?mode=${mode}` : null,
    [mode]
  );
  if (!incidentId) return <p className="muted">Missing incident id</p>;
  if (err) return <p className="muted">{err}</p>;
  if (!data) return <p className="muted">Loading…</p>;
  const incident = data.incident;
  return (
    <>
      <PageHeader
        title={`Incident ${incident.incident_id}`}
        subtitle={`${incident.primary_service || "unknown"} · severity ${incident.severity} · ${incident.state}`}
        mode={mode}
      />
      <div className="split">
        <div className="col">
          {investigation.data ? (
            <InvestigationView result={investigation.data} showRaw={isAdvancedMode(mode)} onRefresh={investigation.reload} />
          ) : investigation.err ? (
            <p className="muted">Investigation unavailable: {investigation.err}</p>
          ) : (
            <p className="muted">Running investigation…</p>
          )}
        </div>
        <div className="col">
          <section className="section">
            <h2>Hypotheses</h2>
            {(data.hypotheses || []).length === 0 ? (
              <p className="muted">No hypotheses recorded yet.</p>
            ) : (
              <div className="evidence-list">
                {data.hypotheses.map((h) => (
                  <div className="evidence-item" key={h.hypothesis_id}>
                    <strong>{h.cause_type}</strong>
                    <div className="muted">{h.description}</div>
                    <div className="muted">
                      score {h.total_score ?? "?"} · confidence {h.confidence_label ?? "?"}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </section>
          <section className="section">
            <h2>Recent events</h2>
            <p className="muted">{(data.events || []).length} events linked to this incident.</p>
            {isAdvancedMode(mode) ? (
              <details>
                <summary>Show raw events</summary>
                <pre className="code">{JSON.stringify(data.events, null, 2)}</pre>
              </details>
            ) : null}
          </section>
        </div>
      </div>
    </>
  );
}

function SystemsPage({ mode }: { mode: Mode }) {
  const { data, err } = useJson<{ services: ServiceRow[] }>("/api/services");
  if (err) return <p className="muted">{err}</p>;
  const services = data?.services ?? [];
  return (
    <>
      <PageHeader title="Systems" subtitle="Services, processes, and dependencies." mode={mode} />
      {services.length === 0 ? (
        <p className="muted">No services observed yet.</p>
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Service</th>
                <th>Status</th>
                <th>Events</th>
                <th>Errors</th>
                <th>Last event</th>
              </tr>
            </thead>
            <tbody>
              {services.map((s) => (
                <tr key={s.service_id} className={`status-${s.status}`}>
                  <td>
                    <span className="status-dot" />
                    <NavLink to={`/systems/${s.service_id}`}>{s.service_id}</NavLink>
                  </td>
                  <td>{s.status}</td>
                  <td>{s.event_count ?? 0}</td>
                  <td>{s.error_count ?? 0}</td>
                  <td className="muted">{s.last_event_at ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function ServiceDetailPage({ mode }: { mode: Mode }) {
  const { serviceId } = useParams();
  const path = serviceId ? `/api/services/${serviceId}` : null;
  const { data, err } = useJson<{ service: ServiceRow; events: Record<string, unknown>[]; incidents: IncidentRow[] }>(path);
  const investigation = useJson<InvestigationResponse>(
    serviceId ? `/api/investigate/service/${serviceId}?mode=${mode}` : null,
    [mode]
  );
  if (!serviceId) return <p className="muted">Missing service id</p>;
  if (err) return <p className="muted">{err}</p>;
  if (!data) return <p className="muted">Loading…</p>;
  return (
    <>
      <PageHeader title={`Service ${serviceId}`} subtitle={`status ${data.service?.status || "?"}`} mode={mode} />
      <div className="split">
        <div className="col">
          {investigation.data ? (
            <InvestigationView result={investigation.data} showRaw={isAdvancedMode(mode)} onRefresh={investigation.reload} />
          ) : (
            <p className="muted">Investigation pending…</p>
          )}
        </div>
        <div className="col">
          <section className="section">
            <h2>Active incidents</h2>
            {data.incidents?.length ? (
              <ul>
                {data.incidents.map((i) => (
                  <li key={i.incident_id}>
                    <NavLink to={`/incidents/${i.incident_id}`}>{i.incident_id}</NavLink> — sev {i.severity}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted">No active incidents.</p>
            )}
          </section>
          <section className="section">
            <h2>Recent events</h2>
            <p className="muted">{(data.events || []).length} events in the last 24h.</p>
            {isAdvancedMode(mode) ? (
              <details>
                <summary>Show raw events</summary>
                <pre className="code">{JSON.stringify(data.events, null, 2)}</pre>
              </details>
            ) : null}
          </section>
        </div>
      </div>
    </>
  );
}

function EvidencePage({ mode }: { mode: Mode }) {
  const [service, setService] = useState("");
  const [severity, setSeverity] = useState<string>("");
  const [search, setSearch] = useState("");
  const [limit] = useState(100);
  const params = new URLSearchParams();
  if (service.trim()) params.set("service", service.trim());
  if (severity) params.set("severity", severity);
  if (search.trim()) params.set("search", search.trim());
  params.set("limit", String(limit));
  const path = `/api/logs?${params.toString()}`;
  const { data, err, reload } = useJson<{ logs: Record<string, unknown>[] }>(path, [path]);
  const rows = (data?.logs ?? []) as Array<{
    event_id?: string;
    timestamp?: string;
    severity?: number | string;
    service_id?: string;
    message?: string;
    source_ref?: { source_type?: string };
    tags?: string[];
  }>;
  return (
    <>
      <PageHeader title="Evidence" subtitle="Filter normalized events from the last 24 hours." mode={mode} />
      <div className="row" style={{ marginBottom: "0.7rem" }}>
        <input placeholder="service id" value={service} onChange={(e) => setService(e.target.value)} />
        <select value={severity} onChange={(e) => setSeverity(e.target.value)}>
          <option value="">any severity</option>
          <option value="1">info+</option>
          <option value="2">warn+</option>
          <option value="3">error+</option>
          <option value="4">critical</option>
        </select>
        <input placeholder="message contains…" value={search} onChange={(e) => setSearch(e.target.value)} />
        <button onClick={reload}>Apply</button>
        {err ? <span className="muted">{err}</span> : null}
      </div>
      {rows.length === 0 ? (
        <p className="muted">No events match.</p>
      ) : (
        <div className="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Timestamp</th>
                <th>Severity</th>
                <th>Service</th>
                <th>Source</th>
                <th>Message</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.event_id}>
                  <td className="muted">{r.timestamp}</td>
                  <td>
                    <Severity value={r.severity ?? "info"} />
                  </td>
                  <td>{r.service_id}</td>
                  <td>{r.source_ref?.source_type ?? "?"}</td>
                  <td>{r.message}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function AiInvestigatorPage({ mode }: { mode: Mode }) {
  const [question, setQuestion] = useState("What should I inspect first?");
  const [scope, setScope] = useState("overview");
  const [result, setResult] = useState<InvestigationResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const doctor = useJson<AiDoctorResponse>("/api/ai/doctor");
  const ask = useCallback(() => {
    setLoading(true);
    setErr(null);
    postJson<InvestigationResponse>("/api/ai/ask", { question, scope, mode })
      .then(setResult)
      .catch((e: Error) => setErr(e.message))
      .finally(() => setLoading(false));
  }, [question, scope, mode]);
  return (
    <>
      <PageHeader title="AI Investigator" subtitle="Read-only investigation with cited evidence." mode={mode} />
      {doctor.data ? (
        <div className={doctor.data.warnings?.length ? "warning-banner" : "muted"} style={{ marginBottom: "0.7rem" }}>
          <strong>Provider:</strong> {doctor.data.provider} · {doctor.data.base_url} · model {doctor.data.model} ·{" "}
          {doctor.data.allow_remote ? "remote allowed" : "local only"}
          {doctor.data.warnings?.length ? (
            <ul style={{ marginTop: "0.4rem", paddingLeft: "1.1rem" }}>
              {doctor.data.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
      <div className="card" style={{ marginBottom: "1rem" }}>
        <h3>Ask the investigator</h3>
        <textarea
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          placeholder="Ask about the runtime, an incident, or a service..."
        />
        <div className="row" style={{ marginTop: "0.5rem" }}>
          <select value={scope} onChange={(e) => setScope(e.target.value)}>
            <option value="overview">Overview</option>
            <option value="latest">Latest incident</option>
          </select>
          <button className="primary" onClick={ask} disabled={loading || !question.trim()}>
            {loading ? "Investigating…" : "Investigate"}
          </button>
          {err ? <span className="muted">{err}</span> : null}
        </div>
      </div>
      {result ? <InvestigationView result={result} showRaw={isAdvancedMode(mode)} /> : null}
    </>
  );
}

function WorkspacePage({ mode }: { mode: Mode }) {
  const { data, err, reload } = useJson<WorkspaceMapResponse>("/api/workspace/map");
  if (err) return <p className="muted">{err}</p>;
  if (!data) return <p className="muted">Loading…</p>;
  return (
    <>
      <PageHeader
        title="Workspace"
        subtitle="Detected projects and service-to-project mappings."
        mode={mode}
      />
      <div className="row" style={{ marginBottom: "0.6rem" }}>
        <button onClick={reload}>Re-scan</button>
        <span className="muted">
          {data.projects.length} projects · {data.service_mappings.length} mappings
        </span>
      </div>
      <section className="section">
        <h2>Service mappings</h2>
        {data.service_mappings.length === 0 ? (
          <p className="muted">No mappings inferred. Add explicit mappings under <code>[[workspace.service_mappings]]</code> in inferra.toml.</p>
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Service</th>
                  <th>Project path</th>
                  <th>Confidence</th>
                  <th>Source</th>
                  {isAdvancedMode(mode) ? <th>Signals</th> : null}
                </tr>
              </thead>
              <tbody>
                {data.service_mappings.map((m, idx) => (
                  <tr key={`${m.service_id}-${idx}`}>
                    <td>{m.service_id}</td>
                    <td className="muted">{m.project_path}</td>
                    <td>{(m.confidence ?? 0).toFixed(2)}</td>
                    <td>{m.source}</td>
                    {isAdvancedMode(mode) ? (
                      <td className="muted">
                        {m.signals.map((s) => `${s.name}(${s.confidence})`).join(" · ")}
                      </td>
                    ) : null}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      <section className="section">
        <h2>Detected projects</h2>
        {data.projects.length === 0 ? (
          <p className="muted">No code projects in the scan budget.</p>
        ) : (
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Kind</th>
                  <th>Path</th>
                  <th>Marker</th>
                </tr>
              </thead>
              <tbody>
                {data.projects.slice(0, 50).map((p) => (
                  <tr key={p.path}>
                    <td>
                      <span className="tag">{p.kind}</span>
                    </td>
                    <td className="muted">{p.path}</td>
                    <td className="muted">{p.marker}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </section>
      {data.unmapped_services.length ? (
        <section className="section">
          <h2>Unmapped services</h2>
          <p className="muted">{data.unmapped_services.join(", ")}</p>
        </section>
      ) : null}
    </>
  );
}

function ControlPage({ mode }: { mode: Mode }) {
  const collectors = useJson<{ collectors: CollectorRow[]; queue_depth: number }>("/api/collectors");
  const ai = useJson<AiStatus>("/api/ai/status");
  const [busy, setBusy] = useState<string>("");
  const collectorAction = useCallback(
    async (verb: "start" | "stop") => {
      setBusy(verb);
      try {
        await postJson(`/api/collectors/${verb}`, {});
        collectors.reload();
      } finally {
        setBusy("");
      }
    },
    [collectors]
  );
  return (
    <>
      <PageHeader
        title="Control"
        subtitle="Manage Inferra itself — collectors, AI provider, storage. Read-only toward observed systems."
        mode={mode}
      />
      <div className="card-grid">
        <div className="card">
          <h3>Collectors</h3>
          <div className="value">{collectors.data?.collectors?.length ?? 0}</div>
          <div className="muted">queue depth: {collectors.data?.queue_depth ?? 0}</div>
          <div className="row" style={{ marginTop: "0.4rem" }}>
            <button onClick={() => collectorAction("start")} disabled={busy !== ""}>
              Start
            </button>
            <button onClick={() => collectorAction("stop")} disabled={busy !== ""}>
              Stop
            </button>
          </div>
        </div>
        <div className="card">
          <h3>AI provider</h3>
          <div className="value">{ai.data?.enabled ? (ai.data.available ? "ready" : "off") : "disabled"}</div>
          <div className="muted">
            {ai.data?.provider} · {ai.data?.model} · {ai.data?.base_url}
          </div>
          {ai.data?.reason ? <div className="muted">reason: {ai.data.reason}</div> : null}
        </div>
        <div className="card">
          <h3>Storage</h3>
          <div className="muted">Use the CLI for verify, vacuum, and backup.</div>
          <code className="next-step-cmd">inferra storage verify</code>
        </div>
      </div>
      {isAdvancedMode(mode) ? (
        <section className="section">
          <h2>Collectors detail</h2>
          <pre className="code">{JSON.stringify(collectors.data?.collectors ?? [], null, 2)}</pre>
        </section>
      ) : null}
    </>
  );
}

function SettingsPage({ mode }: { mode: Mode }) {
  const { data, err, reload } = useJson<{ config: Record<string, unknown> }>("/api/config");
  const [text, setText] = useState("");
  const [saveErr, setSaveErr] = useState<string | null>(null);
  const [saveOk, setSaveOk] = useState(false);
  useEffect(() => {
    if (data) setText(JSON.stringify(data.config, null, 2));
  }, [data]);
  const save = useCallback(async () => {
    setSaveErr(null);
    setSaveOk(false);
    try {
      const parsed = JSON.parse(text) as Record<string, unknown>;
      await putJson("/api/config", { config: parsed });
      setSaveOk(true);
      reload();
    } catch (exc) {
      setSaveErr(exc instanceof Error ? exc.message : String(exc));
    }
  }, [text, reload]);
  if (err) return <p className="muted">{err}</p>;
  if (!data) return <p className="muted">Loading…</p>;
  const cfg = data.config as Record<string, Record<string, unknown> | undefined>;
  const experience = (cfg.experience || {}) as Record<string, unknown>;
  const ai = (cfg.ai || {}) as Record<string, unknown>;
  return (
    <>
      <PageHeader title="Settings" subtitle="Inferra configuration." mode={mode} />
      <div className="card-grid">
        <div className="card">
          <h3>Experience</h3>
          <div className="muted">mode: {String(experience.mode)}</div>
          <div className="muted">ai_role: {String(experience.ai_role)}</div>
          <div className="muted">show_raw_evidence_by_default: {String(experience.show_raw_evidence_by_default)}</div>
        </div>
        <div className="card">
          <h3>AI</h3>
          <div className="muted">enabled: {String(ai.enabled)}</div>
          <div className="muted">model: {String(ai.model)}</div>
          <div className="muted">allow_remote: {String(ai.allow_remote)}</div>
        </div>
      </div>
      {isAdvancedMode(mode) ? (
        <section className="section">
          <h2>Raw configuration (JSON)</h2>
          <textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            style={{ minHeight: "320px", fontFamily: "ui-monospace, Consolas, monospace", fontSize: "0.78rem" }}
          />
          <div className="row" style={{ marginTop: "0.5rem" }}>
            <button className="primary" onClick={save}>
              Save
            </button>
            {saveOk ? <span className="muted">saved.</span> : null}
            {saveErr ? <span className="muted" style={{ color: "var(--danger)" }}>{saveErr}</span> : null}
          </div>
        </section>
      ) : (
        <p className="muted" style={{ marginTop: "1rem" }}>Switch to Developer mode to edit the raw configuration.</p>
      )}
    </>
  );
}

export default function App() {
  const [mode, setMode] = useMode();
  const [modeStatus, setModeStatus] = useState("");
  const configState = useJson<ConfigResponse>("/api/config");
  const navMode = useMemo(() => mode, [mode]);
  useEffect(() => {
    const persisted = configMode(configState.data?.config);
    if (persisted && persisted !== mode) {
      setMode(persisted);
    }
    // Only sync when the backend config changes; mode changes are handled by persistMode.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [configState.data]);
  const persistMode = useCallback(
    async (nextMode: Mode) => {
      setMode(nextMode);
      setModeStatus("Saving mode...");
      try {
        const current = configState.data?.config ?? ((await fetchJson<ConfigResponse>("/api/config")).config);
        const nextConfig: InferraConfigPayload = {
          ...current,
          experience: {
            ...(current.experience ?? {}),
            mode: nextMode,
            show_raw_evidence_by_default: nextMode === "developer",
          },
        };
        const saved = await putJson<ConfigResponse>("/api/config", { config: nextConfig });
        configState.reload();
        setModeStatus(saved.applied ? "Mode saved to config." : "Mode updated.");
      } catch (exc) {
        setModeStatus(`Mode is local only: ${exc instanceof Error ? exc.message : String(exc)}`);
      }
    },
    [configState, setMode]
  );
  return (
    <div className="layout">
      <aside className="sidebar">
        <h1>Inferra</h1>
        <div className="tagline">runtime intelligence control plane</div>
        <ModeToggle mode={mode} setMode={persistMode} status={modeStatus} />
        <nav className="nav">
          <NavLink end to="/">
            Overview
          </NavLink>
          <NavLink to="/incidents">Incidents</NavLink>
          <NavLink to="/systems">Systems</NavLink>
          <NavLink to="/evidence">Evidence</NavLink>
          <NavLink to="/ai">AI Investigator</NavLink>
          <NavLink to="/workspace">Workspace</NavLink>
          <NavLink to="/control">Control</NavLink>
          <NavLink to="/settings">Settings</NavLink>
        </nav>
        <div className="sidebar-footer">
          Local-first · read-only towards observed systems
          <br />
          AI never executes commands.
        </div>
      </aside>
      <main className="main">
        <Routes>
          <Route path="/" element={<OverviewPage mode={navMode} />} />
          <Route path="/incidents" element={<IncidentsPage mode={navMode} />} />
          <Route path="/incidents/:incidentId" element={<IncidentDetailPage mode={navMode} />} />
          <Route path="/systems" element={<SystemsPage mode={navMode} />} />
          <Route path="/systems/:serviceId" element={<ServiceDetailPage mode={navMode} />} />
          <Route path="/evidence" element={<EvidencePage mode={navMode} />} />
          <Route path="/ai" element={<AiInvestigatorPage mode={navMode} />} />
          <Route path="/workspace" element={<WorkspacePage mode={navMode} />} />
          <Route path="/control" element={<ControlPage mode={navMode} />} />
          <Route path="/settings" element={<SettingsPage mode={navMode} />} />
        </Routes>
      </main>
    </div>
  );
}
