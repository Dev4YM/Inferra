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
    <Badge variant={tone} className={cn("gap-1", className)}>
      <Icon className="size-3" />
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
    <div className={cn("min-w-0", compact ? "space-y-1" : "space-y-1.5")}>
      <div className="flex items-center justify-between gap-3 text-xs">
        <span className="label-caps">{label}</span>
        <span className="font-data font-medium text-foreground">{percent}%</span>
      </div>
      <div className="h-1.5 overflow-hidden rounded-sm bg-panel-inset">
        <div className={cn("h-full rounded-sm transition-all duration-300", tone)} style={{ width: `${percent}%` }} />
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
      ? "text-critical"
      : tone === "warning"
        ? "text-warning"
        : tone === "success"
          ? "text-success"
          : "text-foreground";

  return (
    <div className="rounded-md border border-border bg-card p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="label-caps">{label}</p>
          <div className={cn("mt-1 font-data text-2xl font-semibold tracking-tight", toneClass)}>
            {typeof value === "string" ? formatDisplayValue(value) : value}
          </div>
        </div>
        <Icon className="size-4 shrink-0 text-muted-foreground" />
      </div>
      {detail ? <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{detail}</p> : null}
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
      <div className="rounded-sm border border-border bg-panel-inset p-2">
        <Server className="size-4 text-muted-foreground" />
      </div>
      <div className="min-w-0">
        <p className="truncate font-medium">{service}</p>
        <div className="mt-0.5 flex flex-wrap items-center gap-2 font-data text-xs text-muted-foreground">
          {runtime ? (
            <span className="inline-flex items-center gap-1">
              <RadioTower className="size-3" />
              {formatDisplayValue(runtime)}
            </span>
          ) : null}
          {latency ? (
            <span className="inline-flex items-center gap-1">
              <TimerReset className="size-3" />
              {latency}
            </span>
          ) : null}
        </div>
      </div>
    </div>
  );
}
