import { emptyInline, escapeHtml } from "./utils.js";

function readCssVar(name, fallback) {
  const raw = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return raw || fallback;
}

export function drawEventRateChart(canvas, buckets) {
  const ctx = canvas.getContext("2d");
  if (!ctx || !buckets.length) return;
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || 400;
  const h = canvas.clientHeight || 192;
  canvas.width = Math.floor(w * dpr);
  canvas.height = Math.floor(h * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const pad = 8;
  const max = Math.max(...buckets.map((b) => b.total), 1);
  const barW = Math.max(2, (w - pad * 2) / buckets.length - 1);
  const baseY = h - pad;
  const chartH = h - pad * 2;
  buckets.forEach((item, i) => {
    const bh = Math.max(2, (item.total / max) * chartH);
    const x = pad + i * (barW + 1);
    const y = baseY - bh;
    ctx.fillStyle = readCssVar("--chart-bar", "#0f766e");
    ctx.fillRect(x, y, barW, bh);
    const sev =
      (Number(item.warn || 0) + Number(item.error || 0) + Number(item.critical || 0)) / Math.max(item.total, 1);
    if (sev > 0.2) {
      ctx.fillStyle = readCssVar("--chart-warn", "#b54708");
      ctx.fillRect(x, y + bh * (1 - sev), barW, bh * sev);
    }
  });
  if (!reduced) {
    ctx.strokeStyle = readCssVar("--chart-grid", "#e2e8f0");
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(pad, pad);
    ctx.lineTo(pad, baseY);
    ctx.lineTo(w - pad, baseY);
    ctx.stroke();
  }
}

export function drawSeverityHistogram(canvas, counts) {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const order = ["debug", "info", "warn", "error", "critical"];
  const values = order.map((k) => Number(counts[k] || 0));
  const max = Math.max(...values, 1);
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || 320;
  const h = canvas.clientHeight || 160;
  canvas.width = Math.floor(w * dpr);
  canvas.height = Math.floor(h * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const colors = ["#94a3b8", "#0f766e", "#b54708", "#c2410c", "#b42318"];
  const barW = (w - 40) / order.length - 6;
  let x = 20;
  values.forEach((v, i) => {
    const bh = Math.max(4, (v / max) * (h - 36));
    const y = h - 24 - bh;
    ctx.fillStyle = colors[i];
    ctx.fillRect(x, y, barW, bh);
    ctx.fillStyle = readCssVar("--chart-label", "#475569");
    ctx.font = "11px system-ui,sans-serif";
    ctx.fillText(order[i].slice(0, 3), x, h - 8);
    x += barW + 6;
  });
}

export function mountChartResize(canvas, redraw) {
  if (!canvas || typeof ResizeObserver === "undefined") return () => {};
  const ro = new ResizeObserver(() => redraw());
  ro.observe(canvas);
  return () => ro.disconnect();
}

export function chartLegendHtml(buckets) {
  if (!buckets.length) return emptyInline("No event buckets yet.");
  const last = buckets.slice(-8);
  return `<div class="mt-2 flex flex-wrap gap-2 text-xs text-slate-600 dark:text-slate-400">${last
    .map(
      (item) =>
        `<span class="rounded border border-slate-200 px-2 py-1 dark:border-slate-600" title="${escapeHtml(item.timestamp)}">${escapeHtml(item.timestamp)}: ${escapeHtml(item.total)}</span>`,
    )
    .join("")}</div>`;
}
