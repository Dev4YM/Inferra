import { state } from "../state.js";
import { getJson } from "../api.js";
import {
  emptyInline,
  escapeAttr,
  escapeHtml,
  renderListBlock,
  severityBadgeClass,
  severityBadgeClassByName,
  severityColor,
  severityDotClass,
  severityLabel,
  shortTime,
  statCard,
} from "../utils.js";
import { renderIncidentLogTable } from "./logs.js";

export function renderIncidents(els, ctx) {
  els.incidents.innerHTML = state.incidents.length
    ? state.incidents.map(renderIncidentListItem).join("")
    : emptyInline("No active incidents.");
  els.incidents.querySelectorAll("[data-incident-id]").forEach((button) => {
    button.addEventListener("click", () => {
      ctx.setView("incidents");
      selectIncident(els, ctx, button.dataset.incidentId);
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
          <strong class="truncate text-ink dark:text-slate-100">${escapeHtml(incident.primary_service || "unknown")}</strong>
        </div>
        <div class="mt-1 text-xs text-slate-500 dark:text-slate-400">${escapeHtml(incident.event_count)} events | ${escapeHtml(incident.state)} | ${escapeHtml(incident.incident_id)}</div>
      </div>
      <span class="status-badge ${incident.severity >= 3 ? "danger" : "warn"}">${severityLabel(incident.severity)}</span>
    </button>
  `;
}

export async function selectIncident(els, ctx, incidentId, updateList = true, softRefresh = false) {
  state.selectedIncidentId = incidentId;
  if (updateList) renderIncidents(els, ctx);
  if (!softRefresh) {
    els.detailTitle.textContent = "Loading incident";
    els.detailMeta.textContent = incidentId;
    els.incidentDetail.innerHTML = `<div class="empty-state">Loading evidence...</div>`;
    els.incidentPanel.innerHTML = "";
    els.incidentTabs.hidden = false;
    els.resolveIncident.hidden = false;
  }

  const [detail, explanation, trace, chatData] = await Promise.all([
    getJson(`/api/incidents/${encodeURIComponent(incidentId)}`),
    getJson(`/api/incidents/${encodeURIComponent(incidentId)}/explanation`),
    getJson(`/api/incidents/${encodeURIComponent(incidentId)}/ai-trace`),
    getJson(`/api/incidents/${encodeURIComponent(incidentId)}/chat/messages`),
  ]);
  state.currentIncident = detail;
  state.currentExplanation = explanation.explanation;
  state.currentTrace = trace;
  state.chatMessages = chatData.messages || [];
  renderIncidentHeader(els);
  if (softRefresh && state.incidentTab === "chat") {
    return;
  }
  renderIncidentTab(els, ctx);
}

function renderIncidentHeader(els) {
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
    <div class="mt-4 rounded-md border border-slate-200 bg-slate-50 p-4 dark:border-slate-600 dark:bg-slate-900">
      <div class="text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">Most likely hypothesis</div>
      <div class="mt-1 text-base font-semibold text-ink dark:text-slate-100">${escapeHtml(top?.description || "No hypothesis generated yet.")}</div>
      <div class="mt-2 text-sm text-slate-600 dark:text-slate-400">${escapeHtml(top?.cause_type || "unknown")} | ${escapeHtml(top?.hypothesis_id || "no-id")}</div>
    </div>
  `;
}

/** Sync tab strip + panel from server for the active workbench tab (call on tab switch). */
export async function refreshActiveIncidentTab(els, ctx) {
  document.querySelectorAll(".tab").forEach((tab) => {
    const on = tab.dataset.tab === state.incidentTab;
    tab.classList.toggle("active", on);
    tab.setAttribute("aria-selected", on ? "true" : "false");
  });
  if (!state.selectedIncidentId || !state.currentIncident) {
    els.incidentPanel.innerHTML = "";
    return;
  }
  const id = state.selectedIncidentId;
  const base = `/api/incidents/${encodeURIComponent(id)}`;
  const tab = state.incidentTab;
  try {
    if (tab === "timeline" || tab === "logs" || tab === "graph") {
      state.currentIncident = await getJson(base);
    } else if (tab === "explanation") {
      const [detail, expl] = await Promise.all([getJson(base), getJson(`${base}/explanation`)]);
      state.currentIncident = detail;
      state.currentExplanation = expl.explanation;
    } else if (tab === "trace") {
      const [detail, trace] = await Promise.all([getJson(base), getJson(`${base}/ai-trace`)]);
      state.currentIncident = detail;
      state.currentTrace = trace;
    } else if (tab === "chat") {
      const [detail, chatData] = await Promise.all([getJson(base), getJson(`${base}/chat/messages`)]);
      state.currentIncident = detail;
      state.chatMessages = chatData.messages || [];
    }
    renderIncidentHeader(els);
    renderIncidentTab(els, ctx);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    console.error("refreshActiveIncidentTab", err);
    els.incidentPanel.innerHTML = `<div class="empty-state">Could not load ${escapeHtml(tab)}. ${escapeHtml(msg)}</div>`;
  }
}

export function renderIncidentTab(els, ctx) {
  document.querySelectorAll(".tab").forEach((tab) => {
    const on = tab.dataset.tab === state.incidentTab;
    tab.classList.toggle("active", on);
    tab.setAttribute("aria-selected", on ? "true" : "false");
  });
  if (!state.currentIncident) {
    els.incidentPanel.innerHTML = "";
    return;
  }
  const renderers = {
    timeline: renderIncidentTimeline,
    logs: () => renderIncidentLogs(),
    graph: renderIncidentGraph,
    explanation: renderIncidentExplanation,
    trace: renderIncidentTrace,
    chat: renderIncidentChat,
  };
  els.incidentPanel.innerHTML = renderers[state.incidentTab]();
  wireIncidentPanel(els, ctx);
}

function renderIncidentTimeline() {
  const events = state.currentIncident.events || [];
  const top = state.currentIncident.hypotheses?.[0] || {};
  const supporting = new Set(top.supporting_events || []);
  const contradicting = new Set(top.contradicting_events || []);
  return `
    <div class="grid gap-3">
      ${events.length
        ? events
            .map(
              (event) => `
        <button class="timeline-row" type="button" data-event-id="${escapeAttr(event.event_id)}">
          <div class="timeline-pin ${severityDotClass(event.severity)}"></div>
          <div class="min-w-0">
            <div class="flex flex-wrap items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
              <span>${escapeHtml(shortTime(event.timestamp))}</span>
              <span class="status-badge ${severityBadgeClass(event.severity)}">${severityLabel(event.severity)}</span>
              ${supporting.has(event.event_id) ? '<span class="status-badge ok">supporting</span>' : ""}
              ${contradicting.has(event.event_id) ? '<span class="status-badge warn">contradicting</span>' : ""}
            </div>
            <div class="mt-1 font-medium text-ink dark:text-slate-100">${escapeHtml(event.service_id)}</div>
            <p class="mt-1 text-sm text-slate-600 dark:text-slate-400">${escapeHtml(event.message)}</p>
          </div>
        </button>`,
            )
            .join("")
        : emptyInline("No incident events linked yet.")}
    </div>
  `;
}

function renderIncidentLogs() {
  return renderIncidentLogTable(state.currentIncident.events || [], "No logs linked to this incident.");
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
  events.forEach((event, index) =>
    positions.set(event.event_id, { x: 120 + (index % 4) * 230, y: 56 + Math.floor(index / 4) * rowHeight }),
  );
  const edgeSvg = edges
    .map((edge) => {
      const source = positions.get(edge.source_event_id);
      const target = positions.get(edge.target_event_id);
      if (!source || !target) return "";
      return `<line x1="${source.x}" y1="${source.y}" x2="${target.x}" y2="${target.y}" stroke="#94a3b8" stroke-width="${Math.max(1, Number(edge.weight || 0.4) * 4)}" stroke-linecap="round"><title>${escapeHtml(edge.edge_type || "correlation")} ${escapeHtml(edge.evidence || "")}</title></line>`;
    })
    .join("");
  const nodeSvg = events
    .map((event) => {
      const pos = positions.get(event.event_id);
      return `
      <g class="graph-node" data-event-id="${escapeAttr(event.event_id)}" transform="translate(${pos.x}, ${pos.y})">
        <circle r="16" fill="${severityColor(event.severity)}"></circle>
        <text x="26" y="-3" fill="#172033" font-size="12" font-weight="700">${escapeHtml(event.service_id.slice(0, 24))}</text>
        <text x="26" y="14" fill="#667085" font-size="11">${escapeHtml(severityLabel(event.severity))}</text>
        <title>${escapeHtml(event.message)}</title>
      </g>`;
    })
    .join("");
  return `
    <div class="overflow-auto rounded-md border border-slate-200 bg-white dark:border-slate-600 dark:bg-slate-900">
      <svg viewBox="0 0 ${width} ${height}" width="100%" height="${Math.min(620, height)}" role="img" aria-label="Incident inference graph" class="text-ink">
        ${edgeSvg}
        ${nodeSvg}
      </svg>
    </div>
    <p class="mt-3 text-sm text-slate-600 dark:text-slate-400">Edges are correlation or plausible sequence evidence, not proof of causation.</p>
  `;
}

function renderIncidentExplanation() {
  const explanation = state.currentExplanation || {};
  return `
    <div class="grid gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
      <article class="rounded-md border border-slate-200 bg-white p-4 dark:border-slate-600 dark:bg-slate-900">
        <div class="text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">Summary</div>
        <p class="mt-2 text-base leading-7 text-ink dark:text-slate-100">${escapeHtml(explanation.summary || "No explanation generated yet.")}</p>
        <h3 class="subhead mt-5">Evidence Assessment</h3>
        <p class="mt-2 text-sm leading-6 text-slate-700 dark:text-slate-300">${escapeHtml(explanation.evidence_narrative || "")}</p>
        <h3 class="subhead mt-5">Timeline Narrative</h3>
        <p class="mt-2 whitespace-pre-wrap text-sm leading-6 text-slate-700 dark:text-slate-300">${escapeHtml(explanation.timeline_narrative || "")}</p>
      </article>
      <aside class="stack">
        <div class="rounded-md border border-slate-200 bg-slate-50 p-4 dark:border-slate-600 dark:bg-slate-800">
          <div class="text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">Model</div>
          <div class="mt-1 font-semibold text-ink dark:text-slate-100">${escapeHtml(explanation.generation_model || "unknown")}</div>
          <div class="mt-2 text-xs text-slate-600 dark:text-slate-400">Quality: ${escapeHtml(explanation.quality || "ok")}</div>
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
  const audit = trace.prompt_audit || {};
  const included = trace.included_events || [];
  const systemPrompt = audit.sanitized_system_prompt || "";
  const userPrompt = audit.sanitized_user_prompt || "";
  return `
    <div class="grid gap-4 lg:grid-cols-[360px_minmax(0,1fr)]">
      <section class="rounded-md border border-slate-200 bg-white p-4 dark:border-slate-600 dark:bg-slate-900">
        <h3 class="subhead">Prompt Boundary</h3>
        ${renderListBlock("Allowed", trace.prompt_contract?.allowed || [])}
        ${renderListBlock("Blocked", trace.prompt_contract?.blocked || [])}
        <div class="mt-4 rounded-md border border-slate-200 bg-slate-50 p-3 text-sm dark:border-slate-600 dark:bg-slate-800">
          <div class="font-semibold text-ink dark:text-slate-100">Redaction</div>
          <div class="mt-1 text-slate-600 dark:text-slate-400">Raw logs sent: ${trace.redaction?.raw_logs_sent ? "yes" : "no"}</div>
          <div class="text-slate-600 dark:text-slate-400">Max evidence events: ${escapeHtml(trace.redaction?.max_events || 30)}</div>
        </div>
        <div class="mt-4 rounded-md border border-slate-200 bg-slate-50 p-3 text-sm dark:border-slate-600 dark:bg-slate-800">
          <div class="font-semibold text-ink dark:text-slate-100">Sanitized prompts (audit)</div>
          <p class="mt-2 text-xs uppercase tracking-wide text-slate-500 dark:text-slate-400">System</p>
          <pre class="code-block mt-1 max-h-48 overflow-auto text-xs">${escapeHtml(systemPrompt || "(no trace stored yet)")}</pre>
          <p class="mt-3 text-xs uppercase tracking-wide text-slate-500 dark:text-slate-400">User</p>
          <pre class="code-block mt-1 max-h-64 overflow-auto text-xs">${escapeHtml(userPrompt || "(no trace stored yet)")}</pre>
        </div>
      </section>
      <section class="rounded-md border border-slate-200 bg-white p-4 dark:border-slate-600 dark:bg-slate-900">
        <h3 class="subhead">Evidence Sent To AI</h3>
        <div class="mt-3 grid gap-2">
          ${included.length
            ? included
                .map(
                  (event) => `
            <button class="list-row text-left" type="button" data-event-id="${escapeAttr(event.event_id)}">
              <div class="min-w-0">
                <div class="flex flex-wrap items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
                  <span>${escapeHtml(shortTime(event.timestamp))}</span>
                  <span class="status-badge ${severityBadgeClassByName(event.severity)}">${escapeHtml(event.severity)}</span>
                  ${event.supporting ? '<span class="status-badge ok">supporting</span>' : ""}
                  ${event.contradicting ? '<span class="status-badge warn">contradicting</span>' : ""}
                </div>
                <div class="mt-1 font-medium text-ink dark:text-slate-100">${escapeHtml(event.service_id)}</div>
                <p class="mt-1 text-sm text-slate-600 dark:text-slate-400">${escapeHtml(event.summary)}</p>
              </div>
            </button>`,
                )
                .join("")
            : emptyInline("No evidence included.")}
        </div>
      </section>
    </div>
  `;
}

function renderIncidentChat() {
  const msgs = state.chatMessages || [];
  const history = msgs.length
    ? msgs
        .map((m) => {
          const cls = m.role === "assistant" ? "assistant" : "user";
          return `<div class="chat-message ${cls}">${escapeHtml(m.content)}</div>`;
        })
        .join("")
    : "";
  return `
    <form id="chat-form" class="grid gap-3 md:grid-cols-[1fr_auto]">
      <input id="chat-question" class="field" autocomplete="off" placeholder="Ask: what evidence supports the top hypothesis?" aria-label="Chat question">
      <button class="btn-primary" type="submit">Ask AI</button>
    </form>
    <div id="chat-log" class="mt-4 grid gap-3" role="log" aria-live="polite" aria-relevant="additions">${history}</div>
  `;
}

function renderChecks(items) {
  if (!items.length) return emptyInline("No suggested checks.");
  return `
    <div class="rounded-md border border-slate-200 bg-white p-4 dark:border-slate-600 dark:bg-slate-900">
      <h3 class="subhead">Suggested Checks</h3>
      <div class="mt-3 grid gap-2">
        ${items.map(
          (item) => `
          <div class="rounded-md border border-slate-200 bg-slate-950 p-3 text-slate-100 dark:border-slate-600">
            <div class="flex items-start justify-between gap-3">
              <code class="min-w-0 whitespace-pre-wrap text-xs">${escapeHtml(item)}</code>
              <button class="rounded border border-slate-600 px-2 py-1 text-xs" type="button" data-copy="${escapeAttr(item)}">Copy</button>
            </div>
          </div>`,
        ).join("")}
      </div>
    </div>
  `;
}

function wireIncidentPanel(els, ctx) {
  els.incidentPanel.querySelectorAll("[data-event-id]").forEach((node) => {
    node.addEventListener("click", () => ctx.openEvent(node.dataset.eventId));
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
      const message = appendChat("assistant", "");
      message.classList.add("streaming-assistant");
      await streamChatAnswer(question, message);
      message.classList.remove("streaming-assistant");
      const refreshed = await getJson(`/api/incidents/${encodeURIComponent(state.selectedIncidentId)}/chat/messages`);
      state.chatMessages = refreshed.messages || [];
      renderIncidentTab(els, ctx);
    });
  }
}

function appendChat(role, text) {
  const log = document.querySelector("#chat-log");
  if (!log) return null;
  const item = document.createElement("div");
  item.className = `chat-message ${role}`;
  item.textContent = text;
  log.appendChild(item);
  return item;
}

function streamChatAnswer(question, target) {
  return new Promise((resolve) => {
    state.chatDedicatedSocketActive = true;
    const scheme = window.location.protocol === "https:" ? "wss" : "ws";
    const socket = new WebSocket(
      `${scheme}://${window.location.host}/api/incidents/${encodeURIComponent(state.selectedIncidentId)}/chat/stream`,
    );
    let text = "";
    socket.addEventListener("open", () => socket.send(JSON.stringify({ question })));
    socket.addEventListener("message", (event) => {
      const payload = JSON.parse(event.data);
      if (payload.type === "token") {
        text += payload.content || "";
        if (target) target.textContent = text;
      } else if (payload.type === "done") {
        socket.close();
        resolve();
      } else if (payload.type === "error") {
        if (target) target.textContent = payload.error || "AI provider unavailable.";
        socket.close();
        resolve();
      }
    });
    socket.addEventListener("close", () => {
      state.chatDedicatedSocketActive = false;
      resolve();
    });
  });
}

export async function resolveSelectedIncident(els, ctx) {
  if (!state.selectedIncidentId) return;
  const response = await fetch(`/api/incidents/${encodeURIComponent(state.selectedIncidentId)}/resolve`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ feedback_type: "skipped" }),
  });
  if (response.ok) {
    state.selectedIncidentId = null;
    state.currentIncident = null;
    els.incidentTabs.hidden = true;
    els.resolveIncident.hidden = true;
    els.incidentDetail.innerHTML = `<div class="empty-state">Incident resolved.</div>`;
    els.incidentPanel.innerHTML = "";
    await ctx.loadOperationalData();
  }
}

export function appendHubChatToken(token) {
  const streaming = document.querySelector(".streaming-assistant");
  if (streaming && !state.chatDedicatedSocketActive) {
    streaming.textContent += token || "";
    return;
  }
  const log = document.querySelector("#chat-log");
  if (!log || state.chatDedicatedSocketActive) return;
  let hub = log.querySelector(".chat-message.hub-stream");
  if (!hub) {
    hub = document.createElement("div");
    hub.className = "chat-message assistant hub-stream";
    hub.setAttribute("aria-label", "Live stream");
    log.appendChild(hub);
  }
  hub.textContent += token || "";
}
