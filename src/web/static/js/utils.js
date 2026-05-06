export const severityNames = ["debug", "info", "warn", "error", "critical"];

export function shortTime(value) {
  if (!value) return "unknown";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString();
}

export function severityLabel(value) {
  return severityNames[Number(value)] || String(value || "unknown");
}

export function severityTextClass(value) {
  if (Number(value) >= 4) return "text-red-700";
  if (Number(value) >= 3) return "text-orange-700";
  if (Number(value) >= 2) return "text-amber-700";
  return "text-emerald-700";
}

export function severityDotClass(value) {
  if (Number(value) >= 4) return "critical";
  if (Number(value) >= 3) return "error";
  if (Number(value) >= 2) return "warn";
  return "ok";
}

export function severityBadgeClass(value) {
  if (Number(value) >= 4) return "danger";
  if (Number(value) >= 2) return "warn";
  return "ok";
}

export function severityBadgeClassByName(value) {
  if (value === "critical" || value === "error") return "danger";
  if (value === "warn") return "warn";
  return "ok";
}

export function statusClass(status) {
  if (status === "critical") return "danger";
  if (status === "degraded" || status === "elevated") return "warn";
  if (status === "healthy") return "ok";
  return "";
}

export function severityColor(value) {
  if (Number(value) >= 4) return "#b42318";
  if (Number(value) >= 3) return "#c2410c";
  if (Number(value) >= 2) return "#b54708";
  return "#0f766e";
}

export function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"']/g, (char) =>
    ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#039;",
    })[char],
  );
}

export function escapeAttr(value) {
  return escapeHtml(value).replace(/`/g, "&#096;");
}

export function statCard(label, value) {
  return `<div class="rounded-md border border-slate-200 bg-white p-3 dark:border-slate-700 dark:bg-slate-900"><div class="text-xs font-medium uppercase tracking-wide text-slate-500 dark:text-slate-400">${escapeHtml(label)}</div><div class="mt-1 break-words text-lg font-semibold text-ink dark:text-slate-100">${escapeHtml(value)}</div></div>`;
}

export function miniMetric(label, value) {
  return `<div class="rounded-md bg-slate-50 p-2 dark:bg-slate-800"><div class="text-xs text-slate-500 dark:text-slate-400">${escapeHtml(label)}</div><div class="font-semibold text-ink dark:text-slate-100">${escapeHtml(value)}</div></div>`;
}

export function emptyInline(text) {
  return `<div class="empty-state">${escapeHtml(text)}</div>`;
}

export function renderListBlock(title, items) {
  if (!items.length) return "";
  return `
    <div class="mt-4">
      <h3 class="subhead">${escapeHtml(title)}</h3>
      <ul class="mt-2 list-disc space-y-1 pl-5 text-sm text-slate-700 dark:text-slate-300">
        ${items.map((item) => `<li>${escapeHtml(item)}</li>`).join("")}
      </ul>
    </div>
  `;
}

export function renderLogTableRowsHtml(events) {
  return events
    .map(
      (event) => `
    <tr class="cursor-pointer hover:bg-slate-50 dark:hover:bg-slate-800" data-event-id="${escapeAttr(event.event_id)}">
      <td class="whitespace-nowrap px-3 py-2 text-slate-500 dark:text-slate-400">${escapeHtml(shortTime(event.timestamp))}</td>
      <td class="px-3 py-2 font-medium text-ink dark:text-slate-100">${escapeHtml(event.service_id)}</td>
      <td class="px-3 py-2"><span class="status-badge ${severityBadgeClass(event.severity)}">${severityLabel(event.severity)}</span></td>
      <td class="max-w-xl px-3 py-2 text-slate-700 dark:text-slate-300">${escapeHtml(event.message)}</td>
      <td class="whitespace-nowrap px-3 py-2 text-slate-500 dark:text-slate-400">${escapeHtml(event.source_ref?.source_type || "unknown")}</td>
    </tr>`,
    )
    .join("");
}
