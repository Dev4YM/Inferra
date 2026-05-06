import { getJson } from "./api.js";
import { escapeHtml, severityLabel, shortTime, statCard } from "./utils.js";

export function setupDrawer(els, closeFn) {
  els.drawer.querySelectorAll("[data-close-drawer]").forEach((node) => node.addEventListener("click", closeFn));
}

export async function openEvent(els, eventId) {
  const payload = await getJson(`/api/events/${encodeURIComponent(eventId)}`);
  const event = payload.event;
  els.drawer.hidden = false;
  els.drawerSubtitle.textContent = `${event.service_id} | ${severityLabel(event.severity)} | ${shortTime(event.timestamp)}`;
  els.eventDetail.innerHTML = `
    <div class="grid gap-3">
      ${statCard("Event ID", event.event_id)}
      ${statCard("Fingerprint", event.fingerprint)}
      ${statCard("Quality", Number(event.quality?.overall || 0).toFixed(2))}
      <section class="rounded-md border border-slate-200 bg-white p-4 dark:border-slate-700 dark:bg-slate-900">
        <h3 class="subhead">Message</h3>
        <p class="mt-2 whitespace-pre-wrap text-sm leading-6 text-slate-700 dark:text-slate-300">${escapeHtml(event.message)}</p>
      </section>
      <section class="rounded-md border border-slate-200 bg-white p-4 dark:border-slate-700 dark:bg-slate-900">
        <h3 class="subhead">Normalized Fields</h3>
        <pre class="code-block mt-3">${escapeHtml(JSON.stringify(event, null, 2))}</pre>
      </section>
    </div>
  `;
}
