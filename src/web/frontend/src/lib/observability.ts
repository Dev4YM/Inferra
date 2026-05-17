import type { EventRow } from "@/api";

export function normalizeTraceId(value: string | null | undefined): string {
  const raw = String(value ?? "").trim();
  if (!raw) return "";

  const traceparentMatch = raw.match(/^[0-9a-fA-F]{2}-([0-9a-fA-F]{32})-[0-9a-fA-F]{16}-[0-9a-fA-F]{2}$/);
  if (traceparentMatch) {
    return traceparentMatch[1].toLowerCase();
  }

  return raw.replace(/[^0-9a-fA-F]/g, "").toLowerCase();
}

export function hasValidTraceId(value: string | null | undefined): boolean {
  return normalizeTraceId(value).length === 32;
}

export function shortTraceId(value: string | null | undefined): string {
  const traceId = normalizeTraceId(value);
  if (!traceId) return "unknown";
  if (traceId.length <= 16) return traceId;
  return `${traceId.slice(0, 8)}...${traceId.slice(-8)}`;
}

export function buildTracePath(
  traceId: string,
  query?: Record<string, string | number | undefined>,
): string {
  const normalized = normalizeTraceId(traceId);
  const base = `/traces/${encodeURIComponent(normalized || traceId)}`;
  if (!query) return base;

  const sp = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === "") continue;
    sp.set(key, String(value));
  }
  const qs = sp.toString();
  return qs ? `${base}?${qs}` : base;
}

export function buildEvidencePath(query?: Record<string, string | number | undefined>): string {
  const base = "/evidence";
  if (!query) return base;

  const sp = new URLSearchParams();
  for (const [key, value] of Object.entries(query)) {
    if (value === undefined || value === "") continue;
    sp.set(key, String(value));
  }
  const qs = sp.toString();
  return qs ? `${base}?${qs}` : base;
}

export function eventMatchesSearch(event: EventRow, search: string): boolean {
  const needle = search.trim().toLowerCase();
  if (!needle) return true;

  const haystacks = [
    event.event_id,
    event.message,
    event.summary,
    event.service_id,
    event.trace_id,
    event.span_id,
    event.signal_kind,
    event.deployment_environment,
    event.severity_text,
    event.source_ref?.source_type,
    ...(event.tags ?? []),
  ]
    .filter((value): value is string => typeof value === "string" && value.length > 0)
    .map((value) => value.toLowerCase());

  return haystacks.some((value) => value.includes(needle));
}
