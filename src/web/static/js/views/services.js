import { state } from "../state.js";
import { getJson } from "../api.js";
import {
  emptyInline,
  escapeAttr,
  escapeHtml,
  miniMetric,
  renderLogTableRowsHtml,
  severityLabel,
  statusClass,
} from "../utils.js";

export function renderServices(els, ctx) {
  els.services.innerHTML = state.services.length
    ? state.services.map(renderServiceCard).join("")
    : emptyInline("No services discovered yet.");
  els.services.querySelectorAll("[data-service-id]").forEach((node) => {
    node.addEventListener("click", () => selectService(els, ctx, node.dataset.serviceId));
  });
}

function renderServiceCard(service) {
  return `
    <button class="service-card text-left" type="button" data-service-id="${escapeAttr(service.service_id)}">
      <div class="flex items-start justify-between gap-3">
        <div class="min-w-0">
          <h3 class="truncate font-semibold text-ink dark:text-slate-100">${escapeHtml(service.service_id)}</h3>
          <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">${escapeHtml(service.last_event_at || "no recent event")}</p>
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

export async function selectService(els, ctx, serviceId) {
  state.selectedServiceId = serviceId;
  const payload = await getJson(`/api/services/${encodeURIComponent(serviceId)}?limit=80`);
  els.serviceDetailTitle.textContent = serviceId;
  const inc = (payload.incidents || [])
    .map(
      (i) => `
    <button class="list-row mb-2" type="button" data-incident-id="${escapeAttr(i.incident_id)}">
      <span class="text-sm font-medium text-ink dark:text-slate-100">${escapeHtml(i.incident_id)}</span>
      <span class="status-badge ${i.severity >= 3 ? "danger" : "warn"}">${severityLabel(i.severity)}</span>
    </button>`,
    )
    .join("");
  els.serviceDetail.innerHTML = `
    <div class="grid gap-3">
      <div class="grid grid-cols-3 gap-2">
        ${miniMetric("Events", payload.service.event_count || 0)}
        ${miniMetric("Errors", payload.service.error_count || 0)}
        ${miniMetric("Incidents", payload.incidents.length)}
      </div>
      <h3 class="subhead">Active incidents</h3>
      ${inc || emptyInline("No active incidents for this service.")}
      <h3 class="subhead">Recent service logs</h3>
      ${renderLogTableStatic(payload.events || [], "No service logs in the last 24 hours.", ctx)}
    </div>
  `;
  els.serviceDetail.querySelectorAll("[data-event-id]").forEach((node) =>
    node.addEventListener("click", () => ctx.openEvent(node.dataset.eventId)),
  );
  els.serviceDetail.querySelectorAll("[data-incident-id]").forEach((node) =>
    node.addEventListener("click", () => {
      ctx.setView("incidents");
      ctx.selectIncident(node.dataset.incidentId);
    }),
  );
}

function renderLogTableStatic(events, emptyText, ctx) {
  if (!events.length) return `<div class="p-4">${emptyInline(emptyText)}</div>`;
  return `
    <div class="max-h-[360px] overflow-auto rounded-md border border-slate-200 dark:border-slate-600">
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
