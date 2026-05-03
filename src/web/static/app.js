const state = {
  view: "dashboard",
  incidentTab: "timeline",
  selectedIncidentId: null,
  selectedServiceId: null,
  health: null,
  dashboard: null,
  incidents: [],
  events: [],
  logs: [],
  services: [],
  collectors: [],
  currentIncident: null,
  currentExplanation: null,
  currentTrace: null,
  aiStatus: null,
  aiModels: [],
  installedModels: [],
};

const severityNames = ["debug", "info", "warn", "error", "critical"];
const eventTypeNames = ["log", "metric", "state_change", "health_check"];

const els = {
  pageTitle: document.querySelector("#page-title"),
  status: document.querySelector("#status"),
  aiPill: document.querySelector("#ai-pill"),
  dashboard: document.querySelector("#view-dashboard"),
  incidents: document.querySelector("#incidents"),
  collectors: document.querySelector("#collectors"),
  services: document.querySelector("#services"),
  serviceDetail: document.querySelector("#service-detail"),
  serviceDetailTitle: document.querySelector("#service-detail-title"),
  logs: document.querySelector("#logs"),
  logFilters: document.querySelector("#log-filters"),
  logSearch: document.querySelector("#log-search"),
  logService: document.querySelector("#log-service"),
  logSeverity: document.querySelector("#log-severity"),
  logLimit: document.querySelector("#log-limit"),
  detailTitle: document.querySelector("#detail-title"),
  detailMeta: document.querySelector("#detail-meta"),
  incidentDetail: document.querySelector("#incident-detail"),
  incidentTabs: document.querySelector("#incident-tabs"),
  incidentPanel: document.querySelector("#incident-panel"),
  resolveIncident: document.querySelector("#resolve-incident"),
  aiStatusGrid: document.querySelector("#ai-status"),
  aiModels: document.querySelector("#ai-models"),
  aiConfigForm: document.querySelector("#ai-config-form"),
  aiEnabled: document.querySelector("#ai-enabled"),
  aiBaseUrl: document.querySelector("#ai-base-url"),
  aiModel: document.querySelector("#ai-model"),
  aiTokenEnv: document.querySelector("#ai-token-env"),
  drawer: document.querySelector("#event-drawer"),
  drawerSubtitle: document.querySelector("#drawer-subtitle"),
  eventDetail: document.querySelector("#event-detail"),
};

async function loadAll() {
  await Promise.all([loadAI(), loadOperationalData(), loadLogs()]);
}

async function loadOperationalData() {
  const [dashboard, incidents, events, collectors, services] = await Promise.all([
    getJson("/api/dashboard"),
    getJson("/api/incidents"),
    getJson("/api/events?limit=120"),
    getJson("/api/collectors"),
    getJson("/api/services"),
  ]);
  state.dashboard = dashboard;
  state.health = dashboard.health;
  state.incidents = incidents.incidents || [];
  state.events = events.events || [];
  state.collectors = collectors.collectors || [];
  state.services = services.services || [];
  renderShellStatus();
  renderDashboard();
  renderIncidents();
  renderCollectors();
  renderServices();
  renderLogServiceOptions();
  if (state.selectedIncidentId && state.incidents.some((item) => item.incident_id === state.selectedIncidentId)) {
    await selectIncident(state.selectedIncidentId, false);
  }
}

async function loadLogs() {
  const params = new URLSearchParams();
  const search = els.logSearch.value.trim();
  const service = els.logService.value;
  const severity = els.logSeverity.value;
  params.set("limit", els.logLimit.value || "120");
  if (search) params.set("search", search);
  if (service) params.set("service", service);
  if (severity) params.set("severity", severity);
  const payload = await getJson(`/api/logs?${params.toString()}`);
  state.logs = payload.logs || [];
  renderLogs();
}

async function loadAI() {
  const [status, models] = await Promise.all([getJson("/api/ai/status"), getJson("/api/ai/models")]);
  state.aiStatus = status;
  state.aiModels = models.registry || [];
  state.installedModels = models.installed || [];
  renderAIConfig();
  renderAIStatus();
  renderAIModels();
}

function setView(view) {
  state.view = view;
  document.querySelectorAll(".view").forEach((node) => node.classList.toggle("active", node.id === `view-${view}`));
  document.querySelectorAll(".nav-item").forEach((node) => node.classList.toggle("active", node.dataset.view === view));
  const titles = {
    dashboard: "Dashboard",
    incidents: "Incidents",
    logs: "Logs",
    services: "Services",
    collectors: "Collectors",
    settings: "Settings",
  };
  els.pageTitle.textContent = titles[view] || "Inferra";
}

function renderShellStatus() {
  const health = state.health || {};
  const active = Number(health.active_incidents || 0);
  const queue = Number(health.queue_depth || 0);
  const collectorErrors = Number(health.collector_errors || 0);
  const stateText = collectorErrors ? "collector attention" : active ? "investigating" : "observing";
  els.status.textContent = `${stateText} | ${active} active incidents | queue ${queue} | ${state.services.length} services`;
}

function renderDashboard() {
  const data = state.dashboard || {incidents: [], services: [], event_rate: [], severity_counts: {}};
  const active = data.incidents || [];
  const services = data.services || [];
  const counts = data.severity_counts || {};
  els.dashboard.innerHTML = `
    <div class="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.65fr)]">
      <section class="surface">
        <div class="section-head">
          <div>
            <h2>Operator Glance</h2>
            <p>Answer first, evidence one click away.</p>
          </div>
          <span class="${active.length ? "status-badge warn" : "status-badge ok"}">${active.length ? "Needs review" : "All quiet"}</span>
        </div>
        <div class="grid gap-3 md:grid-cols-4">
          ${statCard("Active incidents", active.length)}
          ${statCard("Services", services.length)}
          ${statCard("Error logs", Number(counts.error || 0) + Number(counts.critical || 0))}
          ${statCard("Queue depth", data.health?.queue_depth || 0)}
        </div>
        <div class="mt-4">
          <h3 class="subhead">Incident Summary</h3>
          <div class="grid gap-3 lg:grid-cols-2">
            ${active.length ? active.map(renderIncidentSummaryCard).join("") : emptyInline("No active incidents. Inferra is observing.")}
          </div>
        </div>
      </section>
      <section class="surface">
        <div class="section-head">
          <div>
            <h2>Event Rate</h2>
            <p>Last observed minute buckets.</p>
          </div>
        </div>
        ${renderRateChart(data.event_rate || [])}
        <div class="mt-4 grid gap-2">
          ${Object.entries(counts).map(([key, value]) => `<div class="flex items-center justify-between text-sm"><span class="capitalize text-slate-600">${escapeHtml(key)}</span><strong>${escapeHtml(value)}</strong></div>`).join("")}
        </div>
      </section>
    </div>
    <section class="surface mt-4">
      <div class="section-head">
        <div>
          <h2>Service Health</h2>
          <p>Derived from event volume, error ratio, and active incidents.</p>
        </div>
      </div>
      <div class="service-grid">${services.length ? services.map(renderServiceCard).join("") : emptyInline("No service events stored yet.")}</div>
    </section>
  `;
  wireDashboardClicks();
}

function renderIncidentSummaryCard(incident) {
  return `
    <button class="incident-card text-left" type="button" data-incident-id="${escapeAttr(incident.incident_id)}">
      <div class="flex items-start justify-between gap-3">
        <div>
          <div class="text-xs font-semibold uppercase tracking-wide ${severityTextClass(incident.severity)}">${severityLabel(incident.severity)}</div>
          <h3 class="mt-1 text-base font-semibold">${escapeHtml(incident.primary_service || "unknown service")}</h3>
        </div>
        <span class="status-badge ${incident.severity >= 3 ? "danger" : "warn"}">${escapeHtml(incident.state)}</span>
      </div>
      <p class="mt-3 text-sm text-slate-600">${escapeHtml(incident.event_count)} events across ${escapeHtml((incident.affected_services || []).join(", ") || "unknown")}</p>
      <div class="mt-4 text-xs text-slate-500">${escapeHtml(incident.time_range_start)} to ${escapeHtml(incident.time_range_end)}</div>
    </button>
  `;
}

function renderIncidents() {
  els.incidents.innerHTML = state.incidents.length
    ? state.incidents.map(renderIncidentListItem).join("")
    : emptyInline("No active incidents.");
  els.incidents.querySelectorAll("[data-incident-id]").forEach((button) => {
    button.addEventListener("click", () => {
      setView("incidents");
      selectIncident(button.dataset.incidentId);
    });
  });
}

function renderIncidentListItem(incident) {
  const selected = incident.incident_id === state.selectedIncidentId ? "selected" : "";
  return `
    <button class="list-row ${selected}" type="button" data-incident-id="${escapeAttr(incident.incident_id)}">
      <div class="min-w-0">
        <div class="flex items-center gap-2">
          <span class="dot ${severityDotClass(incident.severity)}"></span>
          <strong class="truncate">${escapeHtml(incident.primary_service || "unknown")}</strong>
        </div>
        <div class="mt-1 text-xs text-slate-500">${escapeHtml(incident.event_count)} events | ${escapeHtml(incident.state)} | ${escapeHtml(incident.incident_id)}</div>
      </div>
      <span class="status-badge ${incident.severity >= 3 ? "danger" : "warn"}">${severityLabel(incident.severity)}</span>
    </button>
  `;
}

async function selectIncident(incidentId, updateList = true) {
  state.selectedIncidentId = incidentId;
  if (updateList) renderIncidents();
  els.detailTitle.textContent = "Loading incident";
  els.detailMeta.textContent = incidentId;
  els.incidentDetail.innerHTML = `<div class="empty-state">Loading evidence...</div>`;
  els.incidentPanel.innerHTML = "";
  els.incidentTabs.hidden = false;
  els.resolveIncident.hidden = false;

  const [detail, explanation, trace] = await Promise.all([
    getJson(`/api/incidents/${encodeURIComponent(incidentId)}`),
    getJson(`/api/incidents/${encodeURIComponent(incidentId)}/explanation`),
    getJson(`/api/incidents/${encodeURIComponent(incidentId)}/ai-trace`),
  ]);
  state.currentIncident = detail;
  state.currentExplanation = explanation.explanation;
  state.currentTrace = trace;
  renderIncidentHeader();
  renderIncidentTab();
}

function renderIncidentHeader() {
  const detail = state.currentIncident;
  const incident = detail.incident;
  const top = detail.hypotheses?.[0];
  els.detailTitle.textContent = incident.primary_service || "Unknown service";
  els.detailMeta.textContent = `${incident.state} | severity ${incident.severity} | ${incident.event_count} events`;
  els.incidentDetail.innerHTML = `
    <div class="grid gap-3 md:grid-cols-4">
      ${statCard("Top score", top ? Number(top.total_score || 0).toFixed(2) : "none")}
      ${statCard("Confidence", top?.confidence_label || "unknown")}
      ${statCard("Affected", (incident.affected_services || []).length)}
      ${statCard("Window", shortTime(incident.time_range_start))}
    </div>
    <div class="mt-4 rounded-md border border-slate-200 bg-slate-50 p-4">
      <div class="text-xs font-semibold uppercase tracking-wide text-slate-500">Most likely hypothesis</div>
      <div class="mt-1 text-base font-semibold">${escapeHtml(top?.description || "No hypothesis generated yet.")}</div>
      <div class="mt-2 text-sm text-slate-600">${escapeHtml(top?.cause_type || "unknown")} | ${escapeHtml(top?.hypothesis_id || "no-id")}</div>
    </div>
  `;
}

function renderIncidentTab() {
  document.querySelectorAll(".tab").forEach((tab) => tab.classList.toggle("active", tab.dataset.tab === state.incidentTab));
  if (!state.currentIncident) {
    els.incidentPanel.innerHTML = "";
    return;
  }
  const renderers = {
    timeline: renderIncidentTimeline,
    logs: renderIncidentLogs,
    graph: renderIncidentGraph,
    explanation: renderIncidentExplanation,
    trace: renderIncidentTrace,
    chat: renderIncidentChat,
  };
  els.incidentPanel.innerHTML = renderers[state.incidentTab]();
  wireIncidentPanel();
}

function renderIncidentTimeline() {
  const events = state.currentIncident.events || [];
  const top = state.currentIncident.hypotheses?.[0] || {};
  const supporting = new Set(top.supporting_events || []);
  const contradicting = new Set(top.contradicting_events || []);
  return `
    <div class="grid gap-3">
      ${events.length ? events.map((event) => `
        <button class="timeline-row" type="button" data-event-id="${escapeAttr(event.event_id)}">
          <div class="timeline-pin ${severityDotClass(event.severity)}"></div>
          <div class="min-w-0">
            <div class="flex flex-wrap items-center gap-2 text-xs text-slate-500">
              <span>${escapeHtml(shortTime(event.timestamp))}</span>
              <span class="status-badge ${severityBadgeClass(event.severity)}">${severityLabel(event.severity)}</span>
              ${supporting.has(event.event_id) ? '<span class="status-badge ok">supporting</span>' : ""}
              ${contradicting.has(event.event_id) ? '<span class="status-badge warn">contradicting</span>' : ""}
            </div>
            <div class="mt-1 font-medium">${escapeHtml(event.service_id)}</div>
            <p class="mt-1 text-sm text-slate-600">${escapeHtml(event.message)}</p>
          </div>
        </button>
      `).join("") : emptyInline("No incident events linked yet.")}
    </div>
  `;
}

function renderIncidentLogs() {
  return renderLogTable(state.currentIncident.events || [], "No logs linked to this incident.");
}

function renderIncidentGraph() {
  const events = (state.currentIncident.events || []).slice(0, 20);
  const clusters = state.currentIncident.clusters || [];
  const edges = clusters.flatMap((cluster) => cluster.correlation_edges || []).slice(0, 30);
  if (!events.length) return emptyInline("No events available for graph.");
  const positions = new Map();
  const width = 920;
  const rowHeight = 74;
  const height = Math.max(220, events.length * rowHeight);
  events.forEach((event, index) => positions.set(event.event_id, {x: 120 + (index % 4) * 230, y: 56 + Math.floor(index / 4) * rowHeight}));
  const edgeSvg = edges.map((edge) => {
    const source = positions.get(edge.source_event_id);
    const target = positions.get(edge.target_event_id);
    if (!source || !target) return "";
    return `<line x1="${source.x}" y1="${source.y}" x2="${target.x}" y2="${target.y}" stroke="#94a3b8" stroke-width="${Math.max(1, Number(edge.weight || 0.4) * 4)}" stroke-linecap="round"><title>${escapeHtml(edge.edge_type || "correlation")} ${escapeHtml(edge.evidence || "")}</title></line>`;
  }).join("");
  const nodeSvg = events.map((event) => {
    const pos = positions.get(event.event_id);
    return `
      <g class="graph-node" data-event-id="${escapeAttr(event.event_id)}" transform="translate(${pos.x}, ${pos.y})">
        <circle r="16" fill="${severityColor(event.severity)}"></circle>
        <text x="26" y="-3" fill="#172033" font-size="12" font-weight="700">${escapeHtml(event.service_id.slice(0, 24))}</text>
        <text x="26" y="14" fill="#667085" font-size="11">${escapeHtml(severityLabel(event.severity))}</text>
        <title>${escapeHtml(event.message)}</title>
      </g>
    `;
  }).join("");
  return `
    <div class="overflow-auto rounded-md border border-slate-200 bg-white">
      <svg viewBox="0 0 ${width} ${height}" width="100%" height="${Math.min(620, height)}" role="img" aria-label="Incident inference graph">
        ${edgeSvg}
        ${nodeSvg}
      </svg>
    </div>
    <p class="mt-3 text-sm text-slate-600">Edges are correlation or plausible sequence evidence, not proof of causation.</p>
  `;
}

function renderIncidentExplanation() {
  const explanation = state.currentExplanation || {};
  return `
    <div class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
      <article class="rounded-md border border-slate-200 bg-white p-4">
        <div class="text-xs font-semibold uppercase tracking-wide text-slate-500">Summary</div>
        <p class="mt-2 text-base leading-7">${escapeHtml(explanation.summary || "No explanation generated yet.")}</p>
        <h3 class="subhead mt-5">Evidence Assessment</h3>
        <p class="mt-2 text-sm leading-6 text-slate-700">${escapeHtml(explanation.evidence_narrative || "")}</p>
        <h3 class="subhead mt-5">Timeline Narrative</h3>
        <p class="mt-2 whitespace-pre-wrap text-sm leading-6 text-slate-700">${escapeHtml(explanation.timeline_narrative || "")}</p>
      </article>
      <aside class="stack">
        <div class="rounded-md border border-slate-200 bg-slate-50 p-4">
          <div class="text-xs font-semibold uppercase tracking-wide text-slate-500">Model</div>
          <div class="mt-1 font-semibold">${escapeHtml(explanation.generation_model || "unknown")}</div>
        </div>
        ${renderChecks(explanation.suggested_actions || [])}
        ${renderListBlock("Uncertainty", explanation.uncertainty_notes || [])}
        ${renderListBlock("Guardrails", explanation.guardrail_violations || [])}
      </aside>
    </div>
  `;
}

function renderIncidentTrace() {
  const trace = state.currentTrace || {};
  const included = trace.included_events || [];
  return `
    <div class="grid gap-4 lg:grid-cols-[360px_minmax(0,1fr)]">
      <section class="rounded-md border border-slate-200 bg-white p-4">
        <h3 class="subhead">Prompt Boundary</h3>
        ${renderListBlock("Allowed", trace.prompt_contract?.allowed || [])}
        ${renderListBlock("Blocked", trace.prompt_contract?.blocked || [])}
        <div class="mt-4 rounded-md border border-slate-200 bg-slate-50 p-3 text-sm">
          <div class="font-semibold">Redaction</div>
          <div class="mt-1 text-slate-600">Raw logs sent: ${trace.redaction?.raw_logs_sent ? "yes" : "no"}</div>
          <div class="text-slate-600">Max evidence events: ${escapeHtml(trace.redaction?.max_events || 30)}</div>
        </div>
      </section>
      <section class="rounded-md border border-slate-200 bg-white p-4">
        <h3 class="subhead">Evidence Sent To AI</h3>
        <div class="mt-3 grid gap-2">
          ${included.length ? included.map((event) => `
            <button class="list-row text-left" type="button" data-event-id="${escapeAttr(event.event_id)}">
              <div class="min-w-0">
                <div class="flex flex-wrap items-center gap-2 text-xs text-slate-500">
                  <span>${escapeHtml(shortTime(event.timestamp))}</span>
                  <span class="status-badge ${severityBadgeClassByName(event.severity)}">${escapeHtml(event.severity)}</span>
                  ${event.supporting ? '<span class="status-badge ok">supporting</span>' : ""}
                  ${event.contradicting ? '<span class="status-badge warn">contradicting</span>' : ""}
                </div>
                <div class="mt-1 font-medium">${escapeHtml(event.service_id)}</div>
                <p class="mt-1 text-sm text-slate-600">${escapeHtml(event.summary)}</p>
              </div>
            </button>
          `).join("") : emptyInline("No evidence included.")}
        </div>
      </section>
    </div>
  `;
}

function renderIncidentChat() {
  return `
    <form id="chat-form" class="grid gap-3 md:grid-cols-[1fr_auto]">
      <input id="chat-question" class="field" autocomplete="off" placeholder="Ask: what evidence supports the top hypothesis?">
      <button class="btn-primary" type="submit">Ask AI</button>
    </form>
    <div id="chat-log" class="mt-4 grid gap-3"></div>
  `;
}

function wireIncidentPanel() {
  els.incidentPanel.querySelectorAll("[data-event-id]").forEach((node) => {
    node.addEventListener("click", () => openEvent(node.dataset.eventId));
  });
  els.incidentPanel.querySelectorAll("[data-copy]").forEach((node) => {
    node.addEventListener("click", () => navigator.clipboard?.writeText(node.dataset.copy || ""));
  });
  const chatForm = document.querySelector("#chat-form");
  if (chatForm) {
    chatForm.addEventListener("submit", async (event) => {
      event.preventDefault();
      const input = document.querySelector("#chat-question");
      const question = input.value.trim();
      if (!question) return;
      appendChat("user", question);
      input.value = "";
      const response = await fetch(`/api/incidents/${encodeURIComponent(state.selectedIncidentId)}/chat`, {
        method: "POST",
        headers: {"Content-Type": "application/json"},
        body: JSON.stringify({question}),
      });
      const payload = await response.json();
      appendChat("assistant", payload.answer || payload.detail || "No answer returned.");
    });
  }
}

function renderCollectors() {
  els.collectors.innerHTML = state.collectors.length
    ? state.collectors.map((collector) => `
      <article class="rounded-md border border-slate-200 bg-white p-4">
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0">
            <h3 class="truncate font-semibold">${escapeHtml(collector.source_type || "collector")}</h3>
            <p class="mt-1 truncate text-sm text-slate-500">${escapeHtml(collector.collector_id || "unknown")}</p>
          </div>
          <span class="status-badge ${collector.status === "running" ? "ok" : collector.error_count ? "danger" : "warn"}">${escapeHtml(collector.status || "unknown")}</span>
        </div>
        <div class="mt-4 grid grid-cols-3 gap-2 text-sm">
          ${miniMetric("Events", collector.events_emitted || 0)}
          ${miniMetric("Errors", collector.error_count || 0)}
          ${miniMetric("Queue", state.health?.queue_depth || 0)}
        </div>
        ${collector.last_error ? `<p class="mt-3 rounded-md bg-red-50 p-3 text-sm text-red-800">${escapeHtml(collector.last_error)}</p>` : ""}
      </article>
    `).join("")
    : emptyInline("No supervised collectors configured for this platform.");
}

function renderServices() {
  els.services.innerHTML = state.services.length
    ? state.services.map(renderServiceCard).join("")
    : emptyInline("No services discovered yet.");
  els.services.querySelectorAll("[data-service-id]").forEach((node) => {
    node.addEventListener("click", () => selectService(node.dataset.serviceId));
  });
}

function renderServiceCard(service) {
  return `
    <button class="service-card text-left" type="button" data-service-id="${escapeAttr(service.service_id)}">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <h3 class="truncate font-semibold">${escapeHtml(service.service_id)}</h3>
          <p class="mt-1 text-xs text-slate-500">${escapeHtml(service.last_event_at || "no recent event")}</p>
        </div>
        <span class="status-badge ${statusClass(service.status)}">${escapeHtml(service.status || "unknown")}</span>
      </div>
      <div class="mt-4 grid grid-cols-3 gap-2 text-sm">
        ${miniMetric("Events", service.event_count || 0)}
        ${miniMetric("Errors", service.error_count || 0)}
        ${miniMetric("Ratio", `${Math.round(Number(service.error_ratio || 0) * 100)}%`)}
      </div>
    </button>
  `;
}

async function selectService(serviceId) {
  state.selectedServiceId = serviceId;
  const payload = await getJson(`/api/services/${encodeURIComponent(serviceId)}?limit=80`);
  els.serviceDetailTitle.textContent = serviceId;
  els.serviceDetail.innerHTML = `
    <div class="grid gap-3">
      <div class="grid grid-cols-3 gap-2">
        ${miniMetric("Events", payload.service.event_count || 0)}
        ${miniMetric("Errors", payload.service.error_count || 0)}
        ${miniMetric("Incidents", payload.incidents.length)}
      </div>
      <h3 class="subhead">Recent Service Logs</h3>
      ${renderLogTable(payload.events || [], "No service logs in the last 24 hours.")}
    </div>
  `;
  els.serviceDetail.querySelectorAll("[data-event-id]").forEach((node) => node.addEventListener("click", () => openEvent(node.dataset.eventId)));
}

function renderLogs() {
  els.logs.innerHTML = renderLogTable(state.logs, "No logs match these filters.");
  els.logs.querySelectorAll("[data-event-id]").forEach((node) => {
    node.addEventListener("click", () => openEvent(node.dataset.eventId));
  });
}

function renderLogTable(events, emptyText) {
  if (!events.length) return `<div class="p-4">${emptyInline(emptyText)}</div>`;
  return `
    <div class="max-h-[640px] overflow-auto">
      <table class="min-w-full divide-y divide-slate-200 text-sm">
        <thead class="sticky top-0 bg-slate-50 text-left text-xs uppercase tracking-wide text-slate-500">
          <tr>
            <th class="px-3 py-2">Time</th>
            <th class="px-3 py-2">Service</th>
            <th class="px-3 py-2">Severity</th>
            <th class="px-3 py-2">Message</th>
            <th class="px-3 py-2">Source</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-slate-100 bg-white">
          ${events.map((event) => `
            <tr class="cursor-pointer hover:bg-slate-50" data-event-id="${escapeAttr(event.event_id)}">
              <td class="whitespace-nowrap px-3 py-2 text-slate-500">${escapeHtml(shortTime(event.timestamp))}</td>
              <td class="px-3 py-2 font-medium">${escapeHtml(event.service_id)}</td>
              <td class="px-3 py-2"><span class="status-badge ${severityBadgeClass(event.severity)}">${severityLabel(event.severity)}</span></td>
              <td class="max-w-xl px-3 py-2 text-slate-700">${escapeHtml(event.message)}</td>
              <td class="whitespace-nowrap px-3 py-2 text-slate-500">${escapeHtml(event.source_ref?.source_type || "unknown")}</td>
            </tr>
          `).join("")}
        </tbody>
      </table>
    </div>
  `;
}

async function openEvent(eventId) {
  const payload = await getJson(`/api/events/${encodeURIComponent(eventId)}`);
  const event = payload.event;
  els.drawer.hidden = false;
  els.drawerSubtitle.textContent = `${event.service_id} | ${severityLabel(event.severity)} | ${shortTime(event.timestamp)}`;
  els.eventDetail.innerHTML = `
    <div class="grid gap-3">
      ${statCard("Event ID", event.event_id)}
      ${statCard("Fingerprint", event.fingerprint)}
      ${statCard("Quality", Number(event.quality?.overall || 0).toFixed(2))}
      <section class="rounded-md border border-slate-200 bg-white p-4">
        <h3 class="subhead">Message</h3>
        <p class="mt-2 whitespace-pre-wrap text-sm leading-6 text-slate-700">${escapeHtml(event.message)}</p>
      </section>
      <section class="rounded-md border border-slate-200 bg-white p-4">
        <h3 class="subhead">Normalized Fields</h3>
        <pre class="code-block mt-3">${escapeHtml(JSON.stringify(event, null, 2))}</pre>
      </section>
    </div>
  `;
}

function closeDrawer() {
  els.drawer.hidden = true;
}

function renderLogServiceOptions() {
  const current = els.logService.value;
  els.logService.innerHTML = `<option value="">All services</option>${state.services.map((service) => `<option value="${escapeAttr(service.service_id)}">${escapeHtml(service.service_id)}</option>`).join("")}`;
  els.logService.value = current;
}

function renderAIConfig() {
  const status = state.aiStatus || {};
  els.aiEnabled.checked = status.enabled === true;
  els.aiBaseUrl.value = status.base_url || "http://127.0.0.1:11434";
  els.aiTokenEnv.value = status.token_env || "";
  els.aiModel.innerHTML = state.aiModels.map((model) => `<option value="${escapeAttr(model.name)}">${escapeHtml(model.name)} ${escapeHtml(model.size || "")}</option>`).join("");
  els.aiModel.value = status.model || "gemma4:e4b";
}

function renderAIStatus() {
  const status = state.aiStatus || {};
  const enabled = status.enabled === true;
  const available = status.available === true;
  els.aiPill.textContent = enabled ? (available ? "AI ready" : "AI offline") : "AI disabled";
  els.aiPill.className = `rounded-full border px-3 py-1 text-xs ${enabled && available ? "border-emerald-400 text-emerald-200" : enabled ? "border-amber-400 text-amber-200" : "border-slate-600 text-slate-300"}`;
  els.aiStatusGrid.innerHTML = [
    statCard("Provider", status.provider || "ollama"),
    statCard("Model", status.model || "not configured"),
    statCard("Server", status.base_url || "not configured"),
    statCard("State", status.reason || status.error || (available ? "available" : "not available")),
  ].join("");
}

function renderAIModels() {
  const installed = new Set(state.installedModels);
  els.aiModels.innerHTML = state.aiModels.length
    ? state.aiModels.map((model) => `
      <button class="list-row text-left" type="button" data-model="${escapeAttr(model.name)}">
        <div>
          <strong>${escapeHtml(model.name)}</strong>
          <div class="mt-1 text-xs text-slate-500">${escapeHtml(model.size || "unknown size")} | ${escapeHtml(model.context_window || "context unknown")} | ${escapeHtml(model.variant || "variant")}</div>
        </div>
        <span class="status-badge ${installed.has(model.name) ? "ok" : ""}">${installed.has(model.name) ? "installed" : "available"}</span>
      </button>
    `).join("")
    : emptyInline("No Gemma 4 registry entries loaded.");
  els.aiModels.querySelectorAll("[data-model]").forEach((node) => {
    node.addEventListener("click", () => {
      els.aiModel.value = node.dataset.model;
      setView("settings");
    });
  });
}

async function saveAIConfig() {
  const response = await fetch("/api/ai/config", {
    method: "POST",
    headers: {"Content-Type": "application/json"},
    body: JSON.stringify({
      enabled: els.aiEnabled.checked,
      provider: "ollama",
      base_url: els.aiBaseUrl.value,
      model: els.aiModel.value,
      token_env: els.aiTokenEnv.value,
    }),
  });
  const payload = await response.json();
  if (!response.ok) {
    alert(payload.detail || "Could not save AI config.");
    return;
  }
  state.aiStatus = payload;
  renderAIStatus();
}

async function controlCollectors(action) {
  const payload = await fetch(`/api/collectors/${action}`, {method: "POST"}).then((response) => response.json());
  state.collectors = payload.collectors || [];
  renderCollectors();
  await loadOperationalData();
}

async function resolveSelectedIncident() {
  if (!state.selectedIncidentId) return;
  const response = await fetch(`/api/incidents/${encodeURIComponent(state.selectedIncidentId)}/resolve`, {
    method: "POST",
    headers: {"Content-Type": "application/json"},
    body: JSON.stringify({feedback_type: "skipped"}),
  });
  if (response.ok) {
    state.selectedIncidentId = null;
    state.currentIncident = null;
    els.incidentTabs.hidden = true;
    els.resolveIncident.hidden = true;
    els.incidentDetail.innerHTML = `<div class="empty-state">Incident resolved.</div>`;
    els.incidentPanel.innerHTML = "";
    await loadOperationalData();
  }
}

function appendChat(role, text) {
  const log = document.querySelector("#chat-log");
  if (!log) return;
  const item = document.createElement("div");
  item.className = `chat-message ${role}`;
  item.textContent = text;
  log.appendChild(item);
}

function renderChecks(items) {
  if (!items.length) return emptyInline("No suggested checks.");
  return `
    <div class="rounded-md border border-slate-200 bg-white p-4">
      <h3 class="subhead">Suggested Checks</h3>
      <div class="mt-3 grid gap-2">
        ${items.map((item) => `
          <div class="rounded-md border border-slate-200 bg-slate-950 p-3 text-slate-100">
            <div class="flex items-start justify-between gap-3">
              <code class="min-w-0 whitespace-pre-wrap text-xs">${escapeHtml(item)}</code>
              <button class="rounded border border-slate-600 px-2 py-1 text-xs" type="button" data-copy="${escapeAttr(item)}">Copy</button>
            </div>
          </div>
        `).join("")}
      </div>
    </div>
  `;
}

function renderListBlock(title, items) {
  if (!items.length) return "";
  return `
    <div class="mt-4">
      <h3 class="subhead">${escapeHtml(title)}</h3>
      <ul class="mt-2 list-disc space-y-1 pl-5 text-sm text-slate-700">
        ${items.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}
      </ul>
    </div>
  `;
}

function renderRateChart(buckets) {
  if (!buckets.length) return emptyInline("No event buckets yet.");
  const max = Math.max(...buckets.map((item) => item.total), 1);
  return `
    <div class="flex h-48 items-end gap-1 rounded-md border border-slate-200 bg-slate-50 p-3">
      ${buckets.map((item) => `<div class="flex min-w-[6px] flex-1 flex-col items-center justify-end" title="${escapeAttr(item.timestamp)} ${escapeAttr(item.total)} events"><div class="w-full rounded-t bg-teal-700" style="height:${Math.max(4, (item.total / max) * 160)}px"></div></div>`).join("")}
    </div>
  `;
}

function statCard(label, value) {
  return `<div class="rounded-md border border-slate-200 bg-white p-3"><div class="text-xs font-medium uppercase tracking-wide text-slate-500">${escapeHtml(label)}</div><div class="mt-1 break-words text-lg font-semibold">${escapeHtml(value)}</div></div>`;
}

function miniMetric(label, value) {
  return `<div class="rounded-md bg-slate-50 p-2"><div class="text-xs text-slate-500">${escapeHtml(label)}</div><div class="font-semibold">${escapeHtml(value)}</div></div>`;
}

function emptyInline(text) {
  return `<div class="empty-state">${escapeHtml(text)}</div>`;
}

async function getJson(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url} returned ${response.status}`);
  return response.json();
}

function shortTime(value) {
  if (!value) return "unknown";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString();
}

function severityLabel(value) {
  return severityNames[Number(value)] || String(value || "unknown");
}

function severityTextClass(value) {
  if (Number(value) >= 4) return "text-red-700";
  if (Number(value) >= 3) return "text-orange-700";
  if (Number(value) >= 2) return "text-amber-700";
  return "text-emerald-700";
}

function severityDotClass(value) {
  if (Number(value) >= 4) return "critical";
  if (Number(value) >= 3) return "error";
  if (Number(value) >= 2) return "warn";
  return "ok";
}

function severityBadgeClass(value) {
  if (Number(value) >= 4) return "danger";
  if (Number(value) >= 2) return "warn";
  return "ok";
}

function severityBadgeClassByName(value) {
  if (value === "critical" || value === "error") return "danger";
  if (value === "warn") return "warn";
  return "ok";
}

function statusClass(status) {
  if (status === "critical") return "danger";
  if (status === "degraded" || status === "elevated") return "warn";
  if (status === "healthy") return "ok";
  return "";
}

function severityColor(value) {
  if (Number(value) >= 4) return "#b42318";
  if (Number(value) >= 3) return "#c2410c";
  if (Number(value) >= 2) return "#b54708";
  return "#0f766e";
}

function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"']/g, (char) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#039;",
  }[char]));
}

function escapeAttr(value) {
  return escapeHtml(value).replace(/`/g, "&#096;");
}

function wireDashboardClicks() {
  els.dashboard.querySelectorAll("[data-incident-id]").forEach((node) => {
    node.addEventListener("click", () => {
      setView("incidents");
      selectIncident(node.dataset.incidentId);
    });
  });
  els.dashboard.querySelectorAll("[data-service-id]").forEach((node) => {
    node.addEventListener("click", () => {
      setView("services");
      selectService(node.dataset.serviceId);
    });
  });
}

document.querySelectorAll("[data-view]").forEach((node) => node.addEventListener("click", () => setView(node.dataset.view)));
document.querySelectorAll("[data-tab]").forEach((node) => {
  node.addEventListener("click", () => {
    state.incidentTab = node.dataset.tab;
    renderIncidentTab();
  });
});
document.querySelectorAll("[data-close-drawer]").forEach((node) => node.addEventListener("click", closeDrawer));
document.querySelector("#refresh-data").addEventListener("click", loadOperationalData);
document.querySelector("#refresh-logs").addEventListener("click", loadLogs);
document.querySelector("#refresh-ai").addEventListener("click", loadAI);
document.querySelector("#start-collectors").addEventListener("click", () => controlCollectors("start"));
document.querySelector("#stop-collectors").addEventListener("click", () => controlCollectors("stop"));
els.resolveIncident.addEventListener("click", resolveSelectedIncident);
els.logFilters.addEventListener("submit", (event) => {
  event.preventDefault();
  loadLogs();
});
els.aiConfigForm.addEventListener("submit", (event) => {
  event.preventDefault();
  saveAIConfig();
});

setView("dashboard");
loadAll().catch((error) => {
  els.status.textContent = `UI load failed: ${error.message}`;
});
setInterval(() => {
  loadOperationalData().catch(() => {});
}, 5000);
