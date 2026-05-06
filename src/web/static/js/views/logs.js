import { state } from "../state.js";
import { getJson } from "../api.js";
import { emptyInline, escapeAttr, escapeHtml, renderLogTableRowsHtml } from "../utils.js";

const ROW_HEIGHT = 38;
const BUFFER = 10;

function detachLogVirtual() {
  if (typeof state.logVirtualCleanup === "function") {
    state.logVirtualCleanup();
    state.logVirtualCleanup = null;
  }
}

export async function loadLogs(els, ctx) {
  const params = new URLSearchParams();
  const search = els.logSearch.value.trim();
  const service = els.logService.value;
  const severity = els.logSeverity.value;
  params.set("limit", els.logLimit.value || "500");
  if (search) params.set("search", search);
  if (service) params.set("service", service);
  if (severity) params.set("severity", severity);
  const payload = await getJson(`/api/logs?${params.toString()}`);
  state.logs = payload.logs || [];
  renderLogs(els, ctx);
}

export function renderLogs(els, ctx) {
  detachLogVirtual();
  const events = state.logs;
  if (!events.length) {
    els.logs.innerHTML = `<div class="p-4">${emptyInline("No logs match these filters.")}</div>`;
    return;
  }
  if (events.length <= 500) {
    els.logs.innerHTML = renderFullTable(events);
    wireLogRows(els, ctx);
    return;
  }
  els.logs.innerHTML = `
    <div id="log-scroll-host" class="rounded-md border border-slate-200 dark:border-slate-600" style="height:560px;overflow:auto" role="region" aria-label="Virtualized log list">
      <div id="log-top-spacer"></div>
      <table class="min-w-full text-sm" style="table-layout:fixed;width:100%">
        <thead class="bg-slate-50 text-left text-xs uppercase tracking-wide text-slate-500 dark:bg-slate-800 dark:text-slate-400">
          <tr>
            <th class="px-3 py-2" style="width:140px">Time</th>
            <th class="px-3 py-2" style="width:120px">Service</th>
            <th class="px-3 py-2" style="width:100px">Severity</th>
            <th class="px-3 py-2">Message</th>
            <th class="px-3 py-2" style="width:120px">Source</th>
          </tr>
        </thead>
        <tbody id="log-virtual-tbody" class="divide-y divide-slate-100 bg-white dark:divide-slate-800 dark:bg-slate-900"></tbody>
      </table>
      <div id="log-bottom-spacer"></div>
    </div>`;
  const host = els.logs.querySelector("#log-scroll-host");
  const topSpacer = els.logs.querySelector("#log-top-spacer");
  const botSpacer = els.logs.querySelector("#log-bottom-spacer");
  const tbody = els.logs.querySelector("#log-virtual-tbody");

  function sliceVisible() {
    const scrollTop = host.scrollTop;
    const viewH = host.clientHeight;
    const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - BUFFER);
    const end = Math.min(events.length, Math.ceil((scrollTop + viewH) / ROW_HEIGHT) + BUFFER);
    topSpacer.style.height = `${start * ROW_HEIGHT}px`;
    botSpacer.style.height = `${(events.length - end) * ROW_HEIGHT}px`;
    const chunk = events.slice(start, end);
    tbody.innerHTML = renderLogTableRowsHtml(chunk);
    tbody.querySelectorAll("[data-event-id]").forEach((row, idx) => {
      const ev = chunk[idx];
      row.addEventListener("click", () => ctx.openEvent(ev.event_id));
    });
  }

  host.addEventListener("scroll", sliceVisible, { passive: true });
  sliceVisible();
  state.logVirtualCleanup = () => {
    host.removeEventListener("scroll", sliceVisible);
  };
}

function renderFullTable(events) {
  return `
    <div class="max-h-[640px] overflow-auto">
      <table class="min-w-full divide-y divide-slate-200 text-sm dark:divide-slate-700">
        <thead class="sticky top-0 bg-slate-50 text-left text-xs uppercase tracking-wide text-slate-500 dark:bg-slate-800 dark:text-slate-400">
          <tr>
            <th class="px-3 py-2">Time</th>
            <th class="px-3 py-2">Service</th>
            <th class="px-3 py-2">Severity</th>
            <th class="px-3 py-2">Message</th>
            <th class="px-3 py-2">Source</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-slate-100 bg-white dark:divide-slate-800 dark:bg-slate-900">${renderLogTableRowsHtml(events)}</tbody>
      </table>
    </div>`;
}

function wireLogRows(els, ctx) {
  els.logs.querySelectorAll("[data-event-id]").forEach((node) => {
    node.addEventListener("click", () => ctx.openEvent(node.dataset.eventId));
  });
}

export function renderLogServiceOptions(els) {
  const current = els.logService.value;
  els.logService.innerHTML = `<option value="">All services</option>${state.services
    .map((service) => `<option value="${escapeAttr(service.service_id)}">${escapeHtml(service.service_id)}</option>`)
    .join("")}`;
  els.logService.value = current;
}

export function renderIncidentLogTable(events, emptyText) {
  if (!events.length) return emptyInline(emptyText);
  return `
    <div class="max-h-[520px] overflow-auto rounded-md border border-slate-200 dark:border-slate-600">
      <table class="min-w-full divide-y divide-slate-200 text-sm dark:divide-slate-700">
        <thead class="sticky top-0 bg-slate-50 text-left text-xs uppercase tracking-wide text-slate-500 dark:bg-slate-800 dark:text-slate-400">
          <tr>
            <th class="px-3 py-2">Time</th><th class="px-3 py-2">Service</th><th class="px-3 py-2">Severity</th>
            <th class="px-3 py-2">Message</th><th class="px-3 py-2">Source</th>
          </tr>
        </thead>
        <tbody class="divide-y divide-slate-100 bg-white dark:divide-slate-800 dark:bg-slate-900">${renderLogTableRowsHtml(events)}</tbody>
      </table>
    </div>`;
}
