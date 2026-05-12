import type { ComponentType, ReactNode } from "react";
import { Activity, AlertCircle, CheckCircle2, CircleAlert, Info, RadioTower, Server, TimerReset } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";
import { formatDisplayValue, formatRiskTone, formatSeverity, formatSeverityLabel } from "@/lib/format";

export type Tone = "success" | "warning" | "destructive" | "secondary" | "info";

export function riskTone(value: string | number | null | undefined): Tone {
  const text = typeof value === "number" ? formatSeverity(value) : String(value ?? "").toLowerCase();
  if (["critical", "error", "high", "degraded", "down", "failed"].includes(text)) return "destructive";
  if (["warn", "warning", "medium", "elevated", "investigating", "starting"].includes(text)) return "warning";
  if (["ok", "healthy", "low", "success", "running", "active", "stable"].includes(text)) return "success";
  if (["info", "unknown"].includes(text)) return "info";
  return formatRiskTone(text) === "secondary" ? "secondary" : formatRiskTone(text);
}

export function SeverityIndicator({
  value,
  label,
  className,
}: {
  value: string | number | null | undefined;
  label?: string;
  className?: string;
}) {
  const severity = typeof value === "number" ? formatSeverityLabel(value) : formatDisplayValue(value ?? "unknown");
  const tone = riskTone(value);
  const Icon = tone === "destructive" ? CircleAlert : tone === "warning" ? AlertCircle : tone === "success" ? CheckCircle2 : Info;
  return (
    <Badge variant={tone} className={cn("gap-1.5", className)}>
      <Icon className="size-3.5" />
      {label ? formatDisplayValue(label) : severity}
    </Badge>
  );
}

export function ServiceHealthBadge({ status }: { status: string | null | undefined }) {
  return <SeverityIndicator value={status ?? "unknown"} label={formatDisplayValue(status ?? "unknown")} />;
}

export function ConfidenceMeter({
  value,
  label = "confidence",
  compact = false,
}: {
  value?: number | null;
  label?: string;
  compact?: boolean;
}) {
  const percent = Math.max(0, Math.min(100, Math.round((value ?? 0) * 100)));
  const tone = percent >= 75 ? "bg-success" : percent >= 45 ? "bg-warning" : "bg-muted-foreground";
  return (
    <div className={cn("min-w-0", compact ? "space-y-1" : "space-y-2")}>
      <div className="flex items-center justify-between gap-3 text-xs">
        <span className="font-semibold uppercase tracking-[0.16em] text-muted-foreground">{label}</span>
        <span className="font-medium text-foreground">{percent}%</span>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-secondary">
        <div className={cn("h-full rounded-full transition-all duration-300", tone)} style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}

export function RuntimeStatusCard({
  icon: Icon = Activity,
  label,
  value,
  detail,
  tone = "info",
}: {
  icon?: ComponentType<{ className?: string }>;
  label: string;
  value: ReactNode;
  detail?: ReactNode;
  tone?: Tone;
}) {
  const toneClass =
    tone === "destructive"
      ? "text-critical bg-rose-500/10 border-rose-400/25"
      : tone === "warning"
        ? "text-warning bg-amber-500/10 border-amber-400/25"
        : tone === "success"
          ? "text-success bg-emerald-500/10 border-emerald-400/25"
          : "text-primary bg-sky-500/10 border-sky-400/25";

  return (
    <div className="rounded-2xl border border-border/70 bg-card/65 p-4 shadow-sm">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">{label}</p>
          <div className="mt-2 text-2xl font-semibold tracking-tight">{typeof value === "string" ? formatDisplayValue(value) : value}</div>
        </div>
        <div className={cn("rounded-xl border p-2.5", toneClass)}>
          <Icon className="size-4" />
        </div>
      </div>
      {detail ? <p className="mt-3 text-sm leading-6 text-muted-foreground">{detail}</p> : null}
    </div>
  );
}

export function RuntimeIdentity({
  service,
  runtime,
  latency,
}: {
  service: string;
  runtime?: string | null;
  latency?: string | null;
}) {
  return (
    <div className="flex min-w-0 items-center gap-3">
      <div className="rounded-xl border border-border/70 bg-secondary/65 p-2.5">
        <Server className="size-4 text-primary" />
      </div>
      <div className="min-w-0">
        <p className="truncate font-medium">{service}</p>
        <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          {runtime ? (
            <span className="inline-flex items-center gap-1">
              <RadioTower className="size-3.5" />
              {formatDisplayValue(runtime)}
            </span>
          ) : null}
          {latency ? (
            <span className="inline-flex items-center gap-1">
              <TimerReset className="size-3.5" />
              {latency}
            </span>
          ) : null}
        </div>
      </div>
    </div>
  );
}
