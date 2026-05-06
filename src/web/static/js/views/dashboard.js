import { state } from "../state.js";
import { drawEventRateChart, drawSeverityHistogram, mountChartResize, chartLegendHtml } from "../chart-canvas.js";
import {
  emptyInline,
  escapeAttr,
  escapeHtml,
  miniMetric,
  severityLabel,
  severityTextClass,
  statCard,
  statusClass,
} from "../utils.js";

let chartCleanups = [];

function clearChartCleanups() {
  chartCleanups.forEach((fn) => fn());
  chartCleanups = [];
}

export function renderDashboard(els, ctx) {
  clearChartCleanups();
  const data = state.dashboard || { incidents: [], services: [], event_rate: [], severity_counts: {} };
  const active = data.incidents || [];
  const services = data.services || [];
  const counts = data.severity_counts || {};
  const health = data.health || {};
  els.dashboard.innerHTML = `
    <div class="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.65fr)]">
      <section class="surface">
        <div class="section-head">
          <div>
            <h2>Health</h2>
            <p>Runtime and ingestion snapshot.</p>
          </div>
          <span class="${active.length ? "status-badge warn" : "status-badge ok"}">${active.length ? "Needs review" : "All quiet"}</span>
        </div>
        <div class="grid gap-3 md:grid-cols-4">
          ${statCard("Active incidents", active.length)}
          ${statCard("Queue depth", health.queue_depth ?? 0)}
          ${statCard("Collector errors", health.collector_errors ?? 0)}
          ${statCard("Dedup suppressed", data.dedup?.total_suppressed ?? 0)}
        </div>
        <div class="mt-4 grid gap-4 lg:grid-cols-2">
          <div>
            <h3 class="subhead">Severity histogram</h3>
            <canvas id="dash-severity-chart" class="mt-2 h-40 w-full rounded-md border border-slate-200 dark:border-slate-600" width="400" height="160" role="img" aria-label="Severity histogram"></canvas>
          </div>
          <div>
            <h3 class="subhead">Noise filter</h3>
            <div class="mt-2 grid gap-2 text-sm text-slate-600 dark:text-slate-400">
              <div class="flex justify-between"><span>Filtered</span><strong>${escapeHtml(data.noise?.total_filtered ?? 0)}</strong></div>
              <div class="flex justify-between"><span>Blocklist hits</span><strong>${escapeHtml(data.noise?.blocklist_hits ?? 0)}</strong></div>
              <div class="flex justify-between"><span>Adaptive demotions</span><strong>${escapeHtml(data.noise?.adaptive_demotions ?? 0)}</strong></div>
            </div>
          </div>
        </div>
      </section>
      <section class="surface">
        <div class="section-head">
          <div>
            <h2>Event rate</h2>
            <p>Last minute buckets (canvas).</p>
          </div>
        </div>
        <canvas id="dash-rate-chart" class="h-48 w-full rounded-md border border-slate-200 dark:border-slate-600" width="600" height="192" role="img" aria-label="Event rate chart"></canvas>
        ${chartLegendHtml(data.event_rate || [])}
      </section>
    </div>
    <div class="mt-4 grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
      <section class="surface">
        <div class="section-head">
          <div>
            <h2>Active incidents</h2>
            <p>Open workload.</p>
          </div>
        </div>
        <div class="grid gap-3 lg:grid-cols-2">
          ${active.length ? active.map(renderIncidentSummaryCard).join("") : emptyInline("No active incidents. Inferra is observing.")}
        </div>
      </section>
      <section class="surface">
        <div class="section-head">
          <div>
            <h2>Collector health</h2>
            <p>Supervised tasks.</p>
          </div>
        </div>
        <div class="stack">
          ${(state.collectors || []).length ? (state.collectors || []).map(renderCollectorRow).join("") : emptyInline("No collectors configured.")}
        </div>
      </section>
    </div>
    <section class="surface mt-4">
      <div class="section-head">
        <div>
          <h2>Service grid</h2>
          <p>Derived health from events and incidents.</p>
        </div>
      </div>
      <div class="service-grid">${services.length ? services.map(renderServiceCard).join("") : emptyInline("No service events stored yet.")}</div>
    </section>
  `;
  requestAnimationFrame(() => {
    const rateEl = document.querySelector("#dash-rate-chart");
    const sevEl = document.querySelector("#dash-severity-chart");
    if (rateEl) {
      const redraw = () => drawEventRateChart(rateEl, data.event_rate || []);
      redraw();
      chartCleanups.push(mountChartResize(rateEl, redraw));
    }
    if (sevEl) {
      const redrawS = () => drawSeverityHistogram(sevEl, counts);
      redrawS();
      chartCleanups.push(mountChartResize(sevEl, redrawS));
    }
  });
  wireDashboardClicks(els, ctx);
}

function renderCollectorRow(collector) {
  return `<div class="flex flex-wrap items-center justify-between gap-2 rounded-md border border-slate-200 px-3 py-2 text-sm dark:border-slate-600">
    <span class="font-medium text-ink dark:text-slate-100">${escapeHtml(collector.source_type || "collector")}</span>
    <span class="status-badge ${collector.status === "running" ? "ok" : collector.error_count ? "danger" : "warn"}">${escapeHtml(collector.status || "unknown")}</span>
  </div>`;
}

function renderIncidentSummaryCard(incident) {
  return `
    <button class="incident-card text-left" type="button" data-incident-id="${escapeAttr(incident.incident_id)}">
      <div class="flex items-start justify-between gap-3">
        <div>
          <div class="text-xs font-semibold uppercase tracking-wide ${severityTextClass(incident.severity)}">${severityLabel(incident.severity)}</div>
          <h3 class="mt-1 text-base font-semibold text-ink dark:text-slate-100">${escapeHtml(incident.primary_service || "unknown service")}</h3>
        </div>
        <span class="status-badge ${incident.severity >= 3 ? "danger" : "warn"}">${escapeHtml(incident.state)}</span>
      </div>
      <p class="mt-3 text-sm text-slate-600 dark:text-slate-400">${escapeHtml(incident.event_count)} events across ${escapeHtml((incident.affected_services || []).join(", ") || "unknown")}</p>
      <div class="mt-4 text-xs text-slate-500">${escapeHtml(incident.time_range_start)} to ${escapeHtml(incident.time_range_end)}</div>
    </button>
  `;
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

function wireDashboardClicks(els, ctx) {
  els.dashboard.querySelectorAll("[data-incident-id]").forEach((node) => {
    node.addEventListener("click", () => {
      ctx.setView("incidents");
      ctx.selectIncident(node.dataset.incidentId);
    });
  });
  els.dashboard.querySelectorAll("[data-service-id]").forEach((node) => {
    node.addEventListener("click", () => {
      ctx.setView("services");
      ctx.selectService(node.dataset.serviceId);
    });
  });
}
