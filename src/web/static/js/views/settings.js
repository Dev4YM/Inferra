import { state } from "../state.js";
import { getJson } from "../api.js";
import { escapeAttr, escapeHtml, statCard } from "../utils.js";
import { renderAIModels, renderAIPullPanel } from "./ai.js";

export async function loadAI(els, navigate) {
  const [status, models] = await Promise.all([getJson("/api/ai/status"), getJson("/api/ai/models")]);
  state.aiStatus = status;
  state.aiModels = models.registry || [];
  state.installedModels = models.installed || [];
  renderAIConfig(els, navigate);
  renderAIStatus(els);
  renderAIModels(els, navigate);
}

export function renderAIConfig(els, navigate) {
  const status = state.aiStatus || {};
  els.aiEnabled.checked = status.enabled === true;
  els.aiBaseUrl.value = status.base_url || "http://127.0.0.1:11434";
  els.aiTokenEnv.value = status.token_env || "";
  els.aiAllowRemote.checked = status.allow_remote === true;
  els.aiModel.innerHTML = state.aiModels
    .map((model) => `<option value="${escapeAttr(model.name)}">${escapeHtml(model.name)} ${escapeHtml(model.size || "")}</option>`)
    .join("");
  const preferred = status.model || (state.aiModels[0] && state.aiModels[0].name) || "gemma4:e4b";
  if ([...els.aiModel.options].some((o) => o.value === preferred)) {
    els.aiModel.value = preferred;
  }
  renderAIPullPanel(els, () => loadAI(els, navigate));
}

export function renderAIStatus(els) {
  const status = state.aiStatus || {};
  const enabled = status.enabled === true;
  const available = status.available === true;
  els.aiPill.textContent = enabled ? (available ? "AI ready" : "AI offline") : "AI disabled";
  els.aiPill.className = `rounded-full border px-3 py-1 text-xs ${
    enabled && available ? "border-emerald-400 text-emerald-200" : enabled ? "border-amber-400 text-amber-200" : "border-slate-600 text-slate-300"
  }`;
  els.aiStatusGrid.innerHTML = [
    statCard("Provider", status.provider || "ollama"),
    statCard("Model", status.model || "not configured"),
    statCard("Server", status.base_url || "not configured"),
    statCard("State", status.reason || status.error || (available ? "available" : "not available")),
  ].join("");
}

export async function saveAIConfig(els) {
  const response = await fetch("/api/ai/config", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      enabled: els.aiEnabled.checked,
      provider: "ollama",
      base_url: els.aiBaseUrl.value,
      model: els.aiModel.value,
      token_env: els.aiTokenEnv.value,
      allow_remote: els.aiAllowRemote.checked,
    }),
  });
  const payload = await response.json();
  if (!response.ok) {
    window.alert(payload.detail || "Could not save AI config.");
    return;
  }
  state.aiStatus = payload;
  renderAIStatus(els);
}
