import type { EventRow, InvestigationOutput } from "@/api";

export function formatSeverity(value: number | string | null | undefined): string {
  if (typeof value === "number") {
    return ["debug", "info", "warn", "error", "critical"][Math.max(0, Math.min(4, value))] ?? "info";
  }
  return String(value ?? "unknown").toLowerCase();
}

export function formatDisplayValue(value: string | number | boolean | null | undefined): string {
  if (value === null || value === undefined || value === "") return "Unknown";
  if (typeof value === "boolean") return value ? "Enabled" : "Disabled";
  if (typeof value === "number") return String(value);
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .split(" ")
    .map((part) => {
      if (!part) return part;
      if (/^[A-Z0-9.]+$/.test(part)) return part;
      if (/^(ai|api|llm|cpu|gpu|id|os|ui|url)$/i.test(part)) return part.toUpperCase();
      return `${part.charAt(0).toUpperCase()}${part.slice(1).toLowerCase()}`;
    })
    .join(" ");
}

export function formatSeverityLabel(value: number | string | null | undefined): string {
  return formatDisplayValue(formatSeverity(value));
}

export function formatModeLabel(value: string | null | undefined): string {
  return formatDisplayValue(value);
}

export function formatRiskTone(value: string | null | undefined): "success" | "warning" | "destructive" | "secondary" {
  switch ((value ?? "").toLowerCase()) {
    case "low":
      return "success";
    case "medium":
      return "warning";
    case "high":
    case "critical":
      return "destructive";
    default:
      return "secondary";
  }
}

export function formatRelativeDate(value: string | null | undefined): string {
  if (!value) return "Unknown";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

export function summarizeEvent(event: EventRow): string {
  const message = String(event.summary ?? event.message ?? "").replace(/\s+/g, " ").trim();
  if (message.length <= 140) return message || "No summary";
  return `${message.slice(0, 137)}...`;
}

export function investigationHasSignal(output: InvestigationOutput | null | undefined): boolean {
  if (!output) return false;
  return Boolean(
    output.headline.trim() ||
      output.what_happened.length ||
      output.why_it_matters.length ||
      output.likely_causes.length ||
      output.evidence.length ||
      output.next_steps.length,
  );
}
