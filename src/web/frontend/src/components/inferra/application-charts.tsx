import { useId, type ReactNode } from "react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Pie,
  PieChart,
  RadialBar,
  RadialBarChart,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import type { EventRow } from "@/api";
import type { TimelinePoint } from "@/components/inferra/charts";
import { ResponsiveChartFrame } from "@/components/inferra/responsive-chart-frame";
import { cn } from "@/lib/utils";

const CHART = {
  accent: "var(--accent)",
  success: "var(--success)",
  warn: "var(--warning)",
  critical: "var(--critical)",
  muted: "color-mix(in srgb, var(--muted-foreground) 22%, transparent)",
  track: "color-mix(in srgb, var(--border) 65%, transparent)",
};

const CHART_H = 52;

function formatMemoryMb(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`;
  return `${Math.round(mb)} MB`;
}

function MiniTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: Array<{ name?: string; value?: number }>;
  label?: string;
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="rounded-sm border border-border bg-popover px-2 py-1 text-[10px] shadow-sm">
      {label ? <p className="font-data font-medium">{label}</p> : null}
      {payload.map((item) => (
        <p key={item.name} className="text-muted-foreground">
          {item.name}: <span className="font-data text-foreground">{item.value ?? 0}</span>
        </p>
      ))}
    </div>
  );
}

function ChartShell({
  label,
  value,
  subvalue,
  footer,
  children,
  className,
}: {
  label: string;
  value: string;
  subvalue?: string;
  footer?: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex h-full flex-col", className)}>
      <div className="flex items-baseline justify-between gap-2">
        <span className="label-caps text-[10px] text-muted-foreground">{label}</span>
        <div className="text-right">
          <span className="font-data text-sm font-semibold tabular-nums leading-none">{value}</span>
          {subvalue ? <p className="mt-0.5 font-data text-[10px] leading-none text-muted-foreground">{subvalue}</p> : null}
        </div>
      </div>
      <div className="relative mt-2" style={{ height: CHART_H }}>
        {children}
      </div>
      {footer ? <p className="mt-1.5 truncate text-[10px] text-muted-foreground">{footer}</p> : null}
    </div>
  );
}

export function bucketLogsToTimeline(logs: EventRow[], buckets = 10): TimelinePoint[] {
  if (!logs.length) return [];

  const parsed = logs
    .map((log) => ({
      log,
      time: log.timestamp ? Date.parse(log.timestamp) : Number.NaN,
    }))
    .filter((row) => Number.isFinite(row.time))
    .sort((left, right) => left.time - right.time);

  if (!parsed.length) {
    return [{ label: "recent", total: logs.length, error: 0, warn: 0, critical: 0 }];
  }

  const start = parsed[0].time;
  const end = parsed[parsed.length - 1].time;
  const span = Math.max(end - start, 60_000);
  const bucketMs = span / buckets;
  const slots: TimelinePoint[] = Array.from({ length: buckets }, (_, index) => ({
    label: `${index + 1}`,
    total: 0,
    error: 0,
    warn: 0,
    critical: 0,
  }));

  for (const { log, time } of parsed) {
    const index = Math.min(buckets - 1, Math.max(0, Math.floor((time - start) / bucketMs)));
    const slot = slots[index];
    slot.total += 1;
    const severity = String(log.severity_text ?? log.severity ?? "").toLowerCase();
    if (severity === "critical" || severity === "4") slot.critical = (slot.critical ?? 0) + 1;
    else if (severity === "error" || severity === "3") slot.error = (slot.error ?? 0) + 1;
    else if (severity === "warn" || severity === "warning" || severity === "2") slot.warn = (slot.warn ?? 0) + 1;
  }

  return slots;
}

export function AppCpuChart({ percent }: { percent: number | null | undefined }) {
  const value = Math.max(0, Math.min(100, percent ?? 0));
  const tone = value >= 85 ? CHART.critical : value >= 60 ? CHART.warn : CHART.accent;
  const data = [{ name: "cpu", value, fill: tone }];

  return (
    <ChartShell
      label="CPU"
      value={`${value.toFixed(1)}%`}
      footer={value < 1 ? "Process idle" : "Processor share"}
    >
      <ResponsiveChartFrame>
        {({ width, height }) => (
          <RadialBarChart
            width={width}
            height={height}
            data={data}
            innerRadius="58%"
            outerRadius="88%"
            startAngle={200}
            endAngle={-20}
            barSize={6}
            cx="50%"
            cy="72%"
          >
            <RadialBar background={{ fill: CHART.track }} dataKey="value" cornerRadius={4} />
            <Tooltip content={<MiniTooltip />} />
          </RadialBarChart>
        )}
      </ResponsiveChartFrame>
    </ChartShell>
  );
}

export function AppMemoryChart({
  memoryMb,
  virtualMb,
}: {
  memoryMb: number | null | undefined;
  virtualMb?: number | null;
}) {
  const rss = Math.max(0, memoryMb ?? 0);
  const virtual = Math.max(rss, virtualMb ?? rss);
  const rssPct = virtual > 0 ? Math.max(2, (rss / virtual) * 100) : 0;
  const data = [{ label: "mem", rss: rssPct, headroom: 100 - rssPct }];

  return (
    <ChartShell
      label="Memory"
      value={rss ? formatMemoryMb(rss) : "—"}
      subvalue={virtual > rss ? `${formatMemoryMb(virtual)} virtual` : "RSS"}
      footer="Resident set vs address space"
    >
      <ResponsiveChartFrame>
        {({ width, height }) => (
          <BarChart width={width} height={height} data={data} layout="vertical" margin={{ top: 14, right: 0, bottom: 14, left: 0 }}>
            <XAxis type="number" domain={[0, 100]} hide />
            <YAxis type="category" dataKey="label" hide />
            <Tooltip
              content={({ active }) =>
                active ? (
                  <div className="rounded-sm border border-border bg-popover px-2 py-1 text-[10px] shadow-sm">
                    <p>RSS: {formatMemoryMb(rss)}</p>
                    {virtual > rss ? <p>Virtual: {formatMemoryMb(virtual)}</p> : null}
                  </div>
                ) : null
              }
              cursor={false}
            />
            <Bar dataKey="rss" stackId="m" fill={CHART.accent} radius={[3, 0, 0, 3]} barSize={12} name="RSS" />
            <Bar dataKey="headroom" stackId="m" fill={CHART.track} radius={[0, 3, 3, 0]} barSize={12} name="Virtual" />
          </BarChart>
        )}
      </ResponsiveChartFrame>
    </ChartShell>
  );
}

export function AppErrorChart({
  errorCount,
  eventCount,
  errorRatio,
}: {
  errorCount: number;
  eventCount: number;
  errorRatio?: number | null;
}) {
  const errors = Math.max(0, errorCount);
  const total = Math.max(eventCount, errors);
  const clean = Math.max(0, total - errors);
  const rate = total > 0 ? Math.round((errorRatio ?? errors / total) * 100) : 0;
  const data =
    total > 0
      ? [
          { name: "Errors", value: errors, fill: CHART.critical },
          { name: "OK", value: clean, fill: CHART.success },
        ].filter((row) => row.value > 0)
      : [{ name: "Quiet", value: 1, fill: CHART.track }];

  return (
    <ChartShell
      label="Errors"
      value={total ? String(errors) : "0"}
      subvalue={total ? `${rate}% rate` : "none ingested"}
      footer={total ? `${total} attributed events` : "Waiting for log signals"}
    >
      <ResponsiveChartFrame>
        {({ width, height }) => (
          <PieChart width={width} height={height}>
            <Pie data={data} dataKey="value" nameKey="name" innerRadius={17} outerRadius={28} paddingAngle={total ? 3 : 0} stroke="var(--card)">
              {data.map((entry) => (
                <Cell key={entry.name} fill={entry.fill} />
              ))}
            </Pie>
            <Tooltip content={<MiniTooltip />} />
          </PieChart>
        )}
      </ResponsiveChartFrame>
      <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
        <span className={cn("font-data text-[10px] font-semibold uppercase tracking-wide", !total || errors === 0 ? "text-success" : "text-critical")}>
          {!total ? "ok" : errors === 0 ? "clean" : `${rate}%`}
        </span>
      </div>
    </ChartShell>
  );
}

export function AppActivityChart({
  points,
  mapped,
  eventCount,
  lastEventAt,
}: {
  points: TimelinePoint[];
  mapped: boolean;
  eventCount?: number;
  lastEventAt?: string | null;
}) {
  const gradientId = useId().replace(/:/g, "");
  const total = points.reduce((sum, point) => sum + point.total, 0);
  const hasSeries = total > 0;
  const data = hasSeries
    ? points
    : Array.from({ length: 12 }, (_, index) => ({
        label: String(index),
        total: 0,
        error: 0,
        warn: 0,
        critical: 0,
      }));
  const peak = Math.max(1, ...data.map((point) => point.total));

  const footer = lastEventAt
    ? `Last signal ${lastEventAt}`
    : !mapped
      ? "Map workspace service for timeline"
      : total === 0 && (eventCount ?? 0) > 0
        ? "Events exist outside retention window"
        : "No samples in retention window";

  return (
    <ChartShell
      label="Log activity"
      value={hasSeries ? `${total} sampled` : eventCount ? `${eventCount} total` : "0"}
      subvalue={hasSeries ? "recent window" : mapped ? "no samples" : "unmapped"}
      footer={footer}
    >
      <ResponsiveChartFrame>
        {({ width, height }) => (
          <AreaChart width={width} height={height} data={data} margin={{ top: 4, right: 2, left: -28, bottom: 0 }}>
            <defs>
              <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="var(--accent)" stopOpacity={0.4} />
                <stop offset="100%" stopColor="var(--accent)" stopOpacity={0.03} />
              </linearGradient>
            </defs>
            <CartesianGrid stroke={CHART.track} strokeDasharray="3 3" vertical={false} />
            <XAxis dataKey="label" hide />
            <YAxis domain={[0, peak]} hide />
            <Tooltip content={<MiniTooltip />} />
            {hasSeries ? (
              <>
                <Area type="monotone" dataKey="warn" stackId="sev" name="Warn" stroke="transparent" fill={CHART.warn} fillOpacity={0.12} />
                <Area type="monotone" dataKey="error" stackId="sev" name="Errors" stroke="transparent" fill={CHART.critical} fillOpacity={0.18} />
                <Area
                  type="monotone"
                  dataKey="total"
                  name="Events"
                  stroke={CHART.accent}
                  fill={`url(#${gradientId})`}
                  strokeWidth={1.5}
                  dot={false}
                  activeDot={{ r: 2.5, fill: CHART.accent }}
                />
              </>
            ) : (
              <Area
                type="monotone"
                dataKey="total"
                name="Events"
                stroke={CHART.track}
                fill="transparent"
                strokeWidth={1}
                strokeDasharray="4 4"
                dot={false}
              />
            )}
          </AreaChart>
        )}
      </ResponsiveChartFrame>
    </ChartShell>
  );
}
