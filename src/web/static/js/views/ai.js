import { state } from "../state.js";
import { escapeAttr, escapeHtml } from "../utils.js";

export function renderAIPullPanel(els, reloadAI) {
  if (!els.aiPullPanel) return;
  const model = els.aiModel.value || state.aiStatus?.model || "gemma4:e4b";
  els.aiPullPanel.innerHTML = `
    <div class="rounded-md border border-slate-200 bg-slate-50 p-3 dark:border-slate-600 dark:bg-slate-800">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="min-w-0">
          <div class="text-xs font-semibold uppercase tracking-wide text-slate-500 dark:text-slate-400">Model Pull</div>
          <div id="ai-pull-label" class="mt-1 truncate text-sm text-slate-700 dark:text-slate-200">${escapeHtml(model)}</div>
        </div>
        <button id="ai-pull-model" class="btn-secondary" type="button">Pull</button>
      </div>
      <div class="progress-shell mt-3">
        <div id="ai-pull-bar" class="progress-fill" style="width:0%"></div>
      </div>
      <div id="ai-pull-status" class="mt-2 text-sm text-slate-600 dark:text-slate-400">Idle</div>
    </div>
  `;
  document.querySelector("#ai-pull-model")?.addEventListener("click", () => startAIPull(els, els.aiModel.value || model, reloadAI));
}

export function renderAIModels(els, navigate) {
  const installed = new Set(state.installedModels);
  els.aiModelsRegistry.innerHTML = state.aiModels.length
    ? state.aiModels
        .map(
          (model) => `
      <button class="list-row text-left" type="button" data-model="${escapeAttr(model.name)}">
        <div>
          <strong class="text-ink dark:text-slate-100">${escapeHtml(model.name)}</strong>
          <div class="mt-1 text-xs text-slate-500 dark:text-slate-400">${escapeHtml(model.size || "unknown size")} | ${escapeHtml(model.context_window || "context unknown")} | ${escapeHtml(model.variant || "variant")}</div>
        </div>
        <span class="status-badge ${installed.has(model.name) ? "ok" : ""}">${installed.has(model.name) ? "installed" : "available"}</span>
      </button>`,
        )
        .join("")
    : `<div class="empty-state">No Gemma registry entries loaded.</div>`;
  els.aiModelsRegistry.querySelectorAll("[data-model]").forEach((node) => {
    node.addEventListener("click", () => {
      els.aiModel.value = node.dataset.model;
      navigate("settings");
    });
  });
}

function startAIPull(els, model, reloadAI) {
  const bar = document.querySelector("#ai-pull-bar");
  const statusEl = document.querySelector("#ai-pull-status");
  const button = document.querySelector("#ai-pull-model");
  if (button) button.disabled = true;
  const scheme = window.location.protocol === "https:" ? "wss" : "ws";
  const socket = new WebSocket(`${scheme}://${window.location.host}/api/ai/pull`);
  socket.addEventListener("open", () => socket.send(JSON.stringify({ model })));
  socket.addEventListener("message", (event) => {
    const payload = JSON.parse(event.data);
    if (payload.type === "progress") {
      const percent = Number(payload.percent || 0);
      if (bar && payload.percent != null) bar.style.width = `${Math.max(0, Math.min(100, percent))}%`;
      if (statusEl) {
        statusEl.textContent = `${payload.status || "pulling"} ${payload.percent != null ? `${percent.toFixed(1)}%` : ""}`;
      }
    } else if (payload.type === "done") {
      if (bar) bar.style.width = "100%";
      if (statusEl) statusEl.textContent = "Complete";
      if (button) button.disabled = false;
      socket.close();
      reloadAI().catch(() => {});
    } else if (payload.type === "error") {
      if (statusEl) statusEl.textContent = payload.error || "Pull failed";
      if (button) button.disabled = false;
      socket.close();
    }
  });
  socket.addEventListener("close", () => {
    if (button) button.disabled = false;
  });
}
