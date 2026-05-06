import { state } from "./state.js";
import { appendHubChatToken } from "./views/incidents.js";

export function connectLiveSocket(ctx) {
  const proto = window.location.protocol === "https:" ? "wss" : "ws";
  const ws = new WebSocket(`${proto}://${window.location.host}/ws`);
  let debounceTimer = null;
  let incidentDetailTimer = null;
  const scheduleRefresh = () => {
    if (debounceTimer) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      debounceTimer = null;
      ctx.loadOperationalData().catch((error) => {
        console.error("[Inferra UI] WebSocket refresh loadOperationalData", error);
      });
    }, 650);
  };
  const scheduleSelectedIncidentReload = (incidentId) => {
    if (!incidentId) return;
    if (incidentDetailTimer) clearTimeout(incidentDetailTimer);
    incidentDetailTimer = setTimeout(() => {
      incidentDetailTimer = null;
      if (state.selectedIncidentId !== incidentId) return;
      ctx.selectIncident(incidentId, false, true).catch((error) => {
        console.error("[Inferra UI] WebSocket selectIncident (soft)", error);
      });
    }, 450);
  };
  ws.addEventListener("message", (event) => {
    let msg = {};
    try {
      msg = JSON.parse(event.data);
    } catch {
      return;
    }
    const t = msg.type;
    if (
      t === "event_count" ||
      t === "incident_created" ||
      t === "incident_updated" ||
      t === "incident_resolved" ||
      t === "collector_health" ||
      t === "explanation_ready" ||
      t === "baseline_status"
    ) {
      scheduleRefresh();
      if (
        state.selectedIncidentId &&
        msg.incident_id === state.selectedIncidentId &&
        (t === "explanation_ready" || t === "incident_updated")
      ) {
        scheduleSelectedIncidentReload(msg.incident_id);
      }
    }
    if (t === "ai_stream_token") {
      const iid = msg.incident_id;
      if (
        state.view === "incidents" &&
        state.incidentTab === "chat" &&
        iid &&
        iid === state.selectedIncidentId
      ) {
        appendHubChatToken(msg.token || "");
      }
    }
  });
  ws.addEventListener("close", () => {
    setTimeout(() => connectLiveSocket(ctx), 2000);
  });
}
