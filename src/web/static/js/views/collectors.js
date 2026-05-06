import { state } from "../state.js";
import { getJson } from "../api.js";
import { emptyInline, escapeAttr, escapeHtml, miniMetric } from "../utils.js";

async function controlCollectors(action, ctx) {
  const payload = await fetch(`/api/collectors/${action}`, { method: "POST" }).then((r) => r.json());
  state.collectors = payload.collectors || [];
  renderCollectors(ctx);
  await ctx.loadOperationalData();
}

async function controlOneCollector(collectorId, action, ctx) {
  const path = action === "start" ? "start" : "stop";
  const q = new URLSearchParams({ collector_id: collectorId });
  const res = await fetch(`/api/collectors/one/${path}?${q.toString()}`, { method: "POST" });
  const payload = await res.json();
  if (!res.ok) {
    window.alert(payload.detail || "Collector control failed");
    return;
  }
  state.collectors = payload.collectors || [];
  renderCollectors(ctx);
  await ctx.loadOperationalData();
}

export function renderCollectors(ctx) {
  const host = document.querySelector("#collectors");
  if (!host) return;
  host.innerHTML = state.collectors.length
    ? state.collectors
        .map(
          (collector) => `
      <article class="rounded-md border border-slate-200 bg-white p-4 dark:border-slate-600 dark:bg-slate-900">
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div class="min-w-0">
            <h3 class="truncate font-semibold text-ink dark:text-slate-100">${escapeHtml(collector.source_type || "collector")}</h3>
            <p class="mt-1 truncate text-sm text-slate-500 dark:text-slate-400">${escapeHtml(collector.collector_id || "unknown")}</p>
          </div>
          <span class="status-badge ${collector.status === "running" ? "ok" : collector.error_count ? "danger" : "warn"}">${escapeHtml(collector.status || "unknown")}</span>
        </div>
        <div class="mt-4 grid grid-cols-3 gap-2 text-sm">
          ${miniMetric("Events", collector.events_emitted || 0)}
          ${miniMetric("Errors", collector.error_count || 0)}
          ${miniMetric("Lag s", collector.lag_seconds ?? "—")}
        </div>
        <div class="mt-3 flex flex-wrap gap-2">
          <button class="btn-secondary" type="button" data-collector-start="${escapeAttr(collector.collector_id)}">Start</button>
          <button class="btn-secondary" type="button" data-collector-stop="${escapeAttr(collector.collector_id)}">Stop</button>
        </div>
        ${collector.last_error ? `<p class="mt-3 rounded-md bg-red-50 p-3 text-sm text-red-800 dark:bg-red-950 dark:text-red-200">${escapeHtml(collector.last_error)}</p>` : ""}
      </article>`,
        )
        .join("")
    : emptyInline("No supervised collectors configured for this platform.");
  host.querySelectorAll("[data-collector-start]").forEach((btn) =>
    btn.addEventListener("click", () => controlOneCollector(btn.getAttribute("data-collector-start"), "start", ctx)),
  );
  host.querySelectorAll("[data-collector-stop]").forEach((btn) =>
    btn.addEventListener("click", () => controlOneCollector(btn.getAttribute("data-collector-stop"), "stop", ctx)),
  );
}

export function bindCollectorHeaderButtons(ctx) {
  document.querySelector("#start-collectors")?.addEventListener("click", () => controlCollectors("start", ctx));
  document.querySelector("#stop-collectors")?.addEventListener("click", () => controlCollectors("stop", ctx));
}
