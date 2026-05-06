import { state } from "./state.js";
import { getJson } from "./api.js";
import { setupDrawer, openEvent as openEventDrawer } from "./drawer.js";
import { initTheme, cycleTheme } from "./theme.js";
import { connectLiveSocket } from "./ws-client.js";
import { renderDashboard } from "./views/dashboard.js";
import { renderServices, selectService } from "./views/services.js";
import { renderCollectors, bindCollectorHeaderButtons } from "./views/collectors.js";
import { loadLogs, renderLogServiceOptions } from "./views/logs.js";
import {
  renderIncidents,
  selectIncident,
  refreshActiveIncidentTab,
  resolveSelectedIncident,
} from "./views/incidents.js";
import { loadAI, renderAIConfig, saveAIConfig } from "./views/settings.js";

function queryEls() {
  return {
    pageTitle: document.querySelector("#page-title"),
    status: document.querySelector("#status"),
    aiPill: document.querySelector("#ai-pill"),
    degradedBanner: document.querySelector("#degraded-banner"),
    dashboard: document.querySelector("#view-dashboard"),
    incidents: document.querySelector("#incidents"),
    collectorsHost: document.querySelector("#collectors"),
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
    aiModelsRegistry: document.querySelector("#ai-models"),
    aiConfigForm: document.querySelector("#ai-config-form"),
    aiEnabled: document.querySelector("#ai-enabled"),
    aiBaseUrl: document.querySelector("#ai-base-url"),
    aiModel: document.querySelector("#ai-model"),
    aiTokenEnv: document.querySelector("#ai-token-env"),
    aiAllowRemote: document.querySelector("#ai-allow-remote"),
    aiPullPanel: document.querySelector("#ai-pull-panel"),
    drawer: document.querySelector("#event-drawer"),
    drawerSubtitle: document.querySelector("#drawer-subtitle"),
    eventDetail: document.querySelector("#event-detail"),
    themeToggle: document.querySelector("#theme-toggle"),
    skipLink: document.querySelector("#skip-link"),
  };
}

const els = queryEls();

function reportUiError(context, error) {
  console.error(`[Inferra UI] ${context}`, error);
}

const navigate = (view) => setView(view);

async function loadOperationalDataOnce() {
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
  renderDashboard(els, ctx);
  renderIncidents(els, ctx);
  renderCollectors(ctx);
  renderServices(els, ctx);
  renderLogServiceOptions(els);
  // Do not refetch the open incident or re-render incident tabs here: live updates
  // use WS → selectIncident; re-rendering the panel on every poll wiped Chat and
  // duplicated API calls.
}

let _operationalInFlight = null;

async function loadOperationalData() {
  if (_operationalInFlight) {
    return _operationalInFlight;
  }
  _operationalInFlight = (async () => {
    try {
      await loadOperationalDataOnce();
    } finally {
      _operationalInFlight = null;
    }
  })();
  return _operationalInFlight;
}

async function loadAll() {
  els.status.textContent = "Loading core data…";
  try {
    await Promise.all([loadOperationalData(), loadLogs(els, ctx)]);
  } catch (error) {
    reportUiError("loadAll (core data + logs)", error);
    els.status.textContent = `UI load failed: ${error instanceof Error ? error.message : String(error)}`;
    throw error;
  }
  loadAI(els, navigate).catch((error) => {
    reportUiError("loadAI (background)", error);
    const pill = els.aiPill;
    if (pill) pill.textContent = "AI status unavailable";
  });
}

function renderShellStatus() {
  const health = state.health || {};
  const active = Number(health.active_incidents || 0);
  const queue = Number(health.queue_depth || 0);
  const collectorErrors = Number(health.collector_errors || 0);
  const stateText = collectorErrors ? "collector attention" : active ? "investigating" : "observing";
  els.status.textContent = `${stateText} | ${active} active incidents | queue ${queue} | ${state.services.length} services`;
  const banner = els.degradedBanner;
  if (!banner) return;
  const degraded = Boolean(health.degraded);
  const reasons = Array.isArray(health.degraded_reasons) ? health.degraded_reasons : [];
  const aiOff = health.ai_enabled && health.ai_available === false;
  if (!degraded && !aiOff) {
    banner.classList.add("hidden");
    banner.textContent = "";
    return;
  }
  const parts = [];
  if (aiOff) {
    parts.push("AI unavailable: using template explanations until the provider recovers.");
  }
  if (reasons.length) {
    parts.push(`Degraded: ${reasons.join(", ")}.`);
  }
  if (health.storage_writes_ok === false) {
    parts.push("Event storage writes are blocked; ingestion may drop events.");
  }
  banner.textContent = parts.join(" ");
  banner.classList.remove("hidden");
}

async function refreshDataForView(view) {
  try {
    if (view === "logs") {
      await loadLogs(els, ctx);
    } else if (view === "settings") {
      await loadAI(els, navigate);
    } else if (view === "dashboard" || view === "incidents" || view === "services" || view === "collectors") {
      await loadOperationalData();
    }
  } catch (error) {
    reportUiError(`refreshDataForView(${view})`, error);
  }
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
  const activeNav = document.querySelector(`.nav-item[data-view="${view}"]`);
  if (activeNav) activeNav.focus();
  void refreshDataForView(view);
}

function switchIncidentWorkbenchTab(tab) {
  state.incidentTab = tab;
  void refreshActiveIncidentTab(els, ctx);
}

function closeDrawer() {
  els.drawer.hidden = true;
}

async function openEvent(eventId) {
  await openEventDrawer(els, eventId);
}

const ctx = {
  setView,
  selectIncident: (id, u, soft) => selectIncident(els, ctx, id, u, soft),
  selectService: (id) => selectService(els, ctx, id),
  openEvent,
  loadOperationalData,
  loadAI: () => loadAI(els, navigate),
};

function wireTabsKeyboard() {
  const tablist = document.querySelector("#incident-tabs");
  if (!tablist) return;
  tablist.addEventListener("keydown", (e) => {
    const tabs = [...tablist.querySelectorAll(".tab")];
    const i = tabs.indexOf(document.activeElement);
    if (i < 0) return;
    if (e.key === "ArrowRight") {
      e.preventDefault();
      const n = tabs[(i + 1) % tabs.length];
      n.focus();
      n.click();
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      const n = tabs[(i - 1 + tabs.length) % tabs.length];
      n.focus();
      n.click();
    }
  });
}

document.querySelectorAll("[data-view]").forEach((node) =>
  node.addEventListener("click", () => setView(node.dataset.view)),
);
window.addEventListener("inferra:navigate", (e) => {
  const v = e.detail && e.detail.view;
  if (v) setView(v);
});
document.querySelectorAll("[data-tab]").forEach((node) => {
  node.addEventListener("click", () => switchIncidentWorkbenchTab(node.dataset.tab));
});
setupDrawer(els, closeDrawer);
document.querySelector("#refresh-data")?.addEventListener("click", loadOperationalData);
document.querySelector("#refresh-logs")?.addEventListener("click", () => loadLogs(els, ctx));
document.querySelector("#refresh-ai")?.addEventListener("click", () => loadAI(els, navigate));
bindCollectorHeaderButtons(ctx);
els.resolveIncident.addEventListener("click", () => resolveSelectedIncident(els, ctx));
els.logFilters.addEventListener("submit", (event) => {
  event.preventDefault();
  loadLogs(els, ctx);
});
els.aiConfigForm.addEventListener("submit", (event) => {
  event.preventDefault();
  saveAIConfig(els);
});
els.aiModel.addEventListener("change", () => renderAIConfig(els, navigate));

initTheme(els.themeToggle);
els.themeToggle?.addEventListener("click", () => cycleTheme(els.themeToggle));

setView("dashboard");
loadAll().catch((error) => {
  reportUiError("loadAll (unhandled)", error);
  els.status.textContent = `UI load failed: ${error instanceof Error ? error.message : String(error)}`;
});
connectLiveSocket(ctx);
setInterval(() => {
  loadOperationalData().catch((error) => reportUiError("periodic loadOperationalData", error));
}, 60000);

wireTabsKeyboard();

els.skipLink?.addEventListener("click", (e) => {
  e.preventDefault();
  document.querySelector("#main-content")?.focus();
});
