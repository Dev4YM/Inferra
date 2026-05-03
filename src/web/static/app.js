const state = {
  selectedIncidentId: null,
  incidents: [],
};

const els = {
  status: document.querySelector("#status"),
  aiPill: document.querySelector("#ai-pill"),
  aiStatus: document.querySelector("#ai-status"),
  aiModels: document.querySelector("#ai-models"),
  aiConfigForm: document.querySelector("#ai-config-form"),
  aiEnabled: document.querySelector("#ai-enabled"),
  aiBaseUrl: document.querySelector("#ai-base-url"),
  aiModel: document.querySelector("#ai-model"),
  aiTokenEnv: document.querySelector("#ai-token-env"),
  incidents: document.querySelector("#incidents"),
  collectors: document.querySelector("#collectors"),
  events: document.querySelector("#events"),
  detailTitle: document.querySelector("#detail-title"),
  detailMeta: document.querySelector("#detail-meta"),
  incidentDetail: document.querySelector("#incident-detail"),
  explanation: document.querySelector("#explanation"),
  chatForm: document.querySelector("#chat-form"),
  chatQuestion: document.querySelector("#chat-question"),
  chatLog: document.querySelector("#chat-log"),
};

async function loadAll() {
  await Promise.all([loadAI(), loadOperationalData()]);
}

async function loadOperationalData() {
  const [health, incidents, events] = await Promise.all([
    getJson("/api/health"),
    getJson("/api/incidents"),
    getJson("/api/events?limit=40"),
    getJson("/api/collectors"),
  ]);
  els.status.textContent = `${health.status} | ${health.active_incidents} active`;
  state.incidents = incidents.incidents;
  renderIncidents();
  renderEvents(events.events);
  renderCollectors(collectors.collectors);
  if (state.selectedIncidentId && state.incidents.some((item) => item.incident_id === state.selectedIncidentId)) {
    await selectIncident(state.selectedIncidentId, false);
  }
}

async function loadAI() {
  const [status, models] = await Promise.all([getJson("/api/ai/status"), getJson("/api/ai/models")]);
  renderAIConfig(status, models.registry);
  renderAIStatus(status);
  renderAIModels(models.registry, models.installed || []);
}

function renderAIConfig(status, registry) {
  els.aiEnabled.checked = status.enabled === true;
  els.aiBaseUrl.value = status.base_url || "http://127.0.0.1:11434";
  els.aiTokenEnv.value = status.token_env || "";
  const currentOptions = Array.from(els.aiModel.options).map((option) => option.value).join("\n");
  const nextOptions = registry.map((model) => model.name).join("\n");
  if (currentOptions !== nextOptions) {
    els.aiModel.innerHTML = registry.map((model) => `
      <option value="${escapeHtml(model.name)}">${escapeHtml(model.name)} ${escapeHtml(model.size)}</option>
    `).join("");
  }
  els.aiModel.value = status.model || "gemma4:e4b";
}

function renderAIStatus(status) {
  const enabled = status.enabled === true;
  const available = status.available === true;
  els.aiPill.textContent = enabled ? (available ? "AI ready" : "AI offline") : "AI disabled";
  els.aiPill.className = `status-pill ${enabled && available ? "ok" : enabled ? "warn" : ""}`;
  els.aiStatus.innerHTML = [
    stat("Provider", status.provider || "ollama"),
    stat("Model", status.model || "not configured"),
    stat("Server", status.base_url || "not configured"),
    stat("State", status.reason || status.error || (available ? "available" : "not available")),
  ].join("");
}

function renderAIModels(registry, installed) {
  const installedSet = new Set(installed);
  const preferred = registry.filter((model) => ["gemma4:e2b", "gemma4:e4b", "gemma4:26b", "gemma4:31b"].includes(model.name));
  els.aiModels.innerHTML = preferred.map((model) => `
    <div class="model ${installedSet.has(model.name) ? "installed" : ""}">
      <strong>${escapeHtml(model.name)}</strong>
      <span>${escapeHtml(model.size)} | ${escapeHtml(model.context_window)} | ${escapeHtml(model.variant)}</span>
    </div>
  `).join("");
}

function renderIncidents() {
  els.incidents.innerHTML =
    state.incidents.length === 0
      ? '<p class="muted">No active incidents.</p>'
      : state.incidents.map(renderIncident).join("");
  els.incidents.querySelectorAll("[data-incident-id]").forEach((button) => {
    button.addEventListener("click", () => selectIncident(button.dataset.incidentId));
  });
}

function renderCollectors(collectors) {
  els.collectors.innerHTML =
    collectors.length === 0
      ? '<p class="muted">No supervised collectors configured for this platform.</p>'
      : collectors.map((collector) => `
        <div class="item event">
          <strong>${escapeHtml(collector.source_type)}</strong>
          <span>${escapeHtml(collector.status)} | ${collector.events_emitted} events | ${collector.error_count} errors</span>
          <div class="muted">${escapeHtml(collector.collector_id)}</div>
          ${collector.last_error ? `<div class="muted">${escapeHtml(collector.last_error)}</div>` : ""}
        </div>
      `).join("");
}

async function controlCollectors(action) {
  const payload = await fetch(`/api/collectors/${action}`, {method: "POST"}).then((response) => response.json());
  renderCollectors(payload.collectors || []);
}

function renderIncident(incident) {
  const selected = incident.incident_id === state.selectedIncidentId ? " selected" : "";
  return `<button class="item incident${selected}" type="button" data-incident-id="${escapeHtml(incident.incident_id)}">
    <span>
      <strong>${escapeHtml(incident.primary_service || "unknown")}</strong>
      <span class="muted">severity ${incident.severity} | ${incident.event_count} events | ${escapeHtml(incident.state)}</span>
    </span>
    <span class="badge">${escapeHtml(incident.incident_id)}</span>
  </button>`;
}

async function selectIncident(incidentId, updateList = true) {
  state.selectedIncidentId = incidentId;
  if (updateList) {
    renderIncidents();
  }
  els.incidentDetail.innerHTML = '<p class="muted">Loading incident...</p>';
  els.explanation.innerHTML = "";
  els.chatLog.innerHTML = "";
  const detail = await getJson(`/api/incidents/${encodeURIComponent(incidentId)}`);
  const explanation = await getJson(`/api/incidents/${encodeURIComponent(incidentId)}/explanation`);
  renderIncidentDetail(detail);
  renderExplanation(explanation.explanation);
  els.chatForm.hidden = false;
}

function renderIncidentDetail(detail) {
  const incident = detail.incident;
  els.detailTitle.textContent = incident.primary_service || "Unknown service";
  els.detailMeta.textContent = `${incident.state} | severity ${incident.severity}`;
  els.incidentDetail.innerHTML = `
    <div class="metrics">
      ${stat("Events", incident.event_count)}
      ${stat("Affected", incident.affected_services.join(", ") || "unknown")}
      ${stat("Window", `${incident.time_range_start} -> ${incident.time_range_end}`)}
    </div>
    <h2>Hypotheses</h2>
    ${detail.hypotheses.length === 0 ? '<p class="muted">No hypotheses yet.</p>' : detail.hypotheses.map(renderHypothesis).join("")}
  `;
}

function renderHypothesis(hypothesis) {
  return `<div class="hypothesis">
    <strong>${escapeHtml(hypothesis.cause_type)}</strong>
    <p>${escapeHtml(hypothesis.description)}</p>
    <div class="muted">score ${Number(hypothesis.total_score || 0).toFixed(2)} | ${escapeHtml(hypothesis.confidence_label || "unknown")}</div>
  </div>`;
}

function renderExplanation(explanation) {
  els.explanation.innerHTML = `
    <h2>Explanation</h2>
    <div class="explanation">
      <strong>${escapeHtml(explanation.summary || "No summary")}</strong>
      <p>${escapeHtml(explanation.primary_hypothesis_text || "")}</p>
      <p>${escapeHtml(explanation.evidence_narrative || "")}</p>
      ${renderList("Suggested Checks", explanation.suggested_actions)}
      ${renderList("Uncertainty", explanation.uncertainty_notes)}
      <div class="muted">model ${escapeHtml(explanation.generation_model || "unknown")}</div>
    </div>
  `;
}

async function askIncident(question) {
  const incidentId = state.selectedIncidentId;
  if (!incidentId) {
    return;
  }
  appendChat("user", question);
  els.chatQuestion.value = "";
  const response = await fetch(`/api/incidents/${encodeURIComponent(incidentId)}/chat`, {
    method: "POST",
    headers: {"Content-Type": "application/json"},
    body: JSON.stringify({question}),
  });
  const payload = await response.json();
  appendChat("assistant", payload.answer || payload.detail || "No answer returned.");
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
    appendChat("assistant", payload.detail || "Could not save AI config.");
    return;
  }
  renderAIStatus(payload);
}

function appendChat(role, text) {
  const item = document.createElement("div");
  item.className = `chat-message ${role}`;
  item.textContent = text;
  els.chatLog.appendChild(item);
}

function renderEvents(events) {
  els.events.innerHTML =
    events.length === 0
      ? '<p class="muted">No events stored yet.</p>'
      : events.map(renderEvent).join("");
}

function renderEvent(event) {
  return `<div class="item event">
    <strong>${escapeHtml(event.service_id)}</strong>
    <span>${escapeHtml(event.message)}</span>
    <div class="muted">severity ${event.severity} | ${escapeHtml(event.timestamp)}</div>
  </div>`;
}

function stat(label, value) {
  return `<div class="stat"><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong></div>`;
}

function renderList(title, items) {
  if (!Array.isArray(items) || items.length === 0) {
    return "";
  }
  return `<h3>${escapeHtml(title)}</h3><ul>${items.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}</ul>`;
}

async function getJson(url) {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`${url} returned ${response.status}`);
  }
  return response.json();
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (char) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#039;",
  }[char]));
}

document.querySelector("#refresh-ai").addEventListener("click", loadAI);
document.querySelector("#refresh-data").addEventListener("click", loadOperationalData);
document.querySelector("#start-collectors").addEventListener("click", () => controlCollectors("start"));
document.querySelector("#stop-collectors").addEventListener("click", () => controlCollectors("stop"));
els.aiConfigForm.addEventListener("submit", (event) => {
  event.preventDefault();
  saveAIConfig();
});
els.chatForm.addEventListener("submit", (event) => {
  event.preventDefault();
  const question = els.chatQuestion.value.trim();
  if (question) {
    askIncident(question);
  }
});

loadAll();
setInterval(loadOperationalData, 5000);
