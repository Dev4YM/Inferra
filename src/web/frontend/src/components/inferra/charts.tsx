import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Pie,
  PieChart,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { ResponsiveChartFrame } from "@/components/inferra/responsive-chart-frame";
import { formatDisplayValue } from "@/lib/format";

export type TimelinePoint = {
  label: string;
  total: number;
  warn?: number;
  error?: number;
  critical?: number;
};

const CHART_COLORS = {
  critical: "var(--critical)",
  error: "#9a3412",
  warn: "var(--warning)",
  info: "var(--muted-foreground)",
  debug: "var(--border)",
  accent: "var(--accent)",
};

export function SeverityDistribution({ counts }: { counts: Record<string, number> }) {
  const data = ["critical", "error", "warn", "info", "debug"]
    .map((key) => ({ key, name: formatDisplayValue(key), value: counts[key] ?? 0 }))
    .filter((item) => item.value > 0);

  if (!data.length) {
    return <p className="text-sm text-muted-foreground">No severity data in the current window.</p>;
  }

  return (
    <div className="grid gap-4 md:grid-cols-[160px_minmax(0,1fr)]">
      <ResponsiveChartFrame className="h-40">
        {({ width, height }) => (
          <PieChart width={width} height={height}>
            <Pie data={data} dataKey="value" nameKey="name" innerRadius={44} outerRadius={68} paddingAngle={2} stroke="var(--card)">
              {data.map((entry) => (
                <Cell key={entry.key} fill={CHART_COLORS[entry.key as keyof typeof CHART_COLORS] ?? CHART_COLORS.info} />
              ))}
            </Pie>
            <Tooltip content={<ChartTooltip />} />
          </PieChart>
        )}
      </ResponsiveChartFrame>
      <div className="space-y-1">
        {data.map((item) => (
          <div key={item.key} className="flex items-center justify-between gap-3 border-b border-border py-1.5 text-sm last:border-b-0">
            <span className="inline-flex items-center gap-2 text-muted-foreground">
              <span
                className="size-2 rounded-sm"
                style={{ background: CHART_COLORS[item.key as keyof typeof CHART_COLORS] ?? CHART_COLORS.info }}
              />
              {item.name}
            </span>
            <span className="font-data font-medium">{item.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function EventRateBars({ points }: { points: TimelinePoint[] }) {
  const data = points.slice(-32);

  if (!data.length) {
    return <p className="text-sm text-muted-foreground">No event-rate buckets in the current window.</p>;
  }

  return (
    <ResponsiveChartFrame className="h-56">
      {({ width, height }) => (
        <AreaChart width={width} height={height} data={data} margin={{ left: -16, right: 8, top: 4, bottom: 0 }}>
          <CartesianGrid stroke="var(--border)" strokeDasharray="2 4" vertical={false} />
          <XAxis dataKey="label" tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: "var(--muted-foreground)", fontFamily: "var(--font-mono)" }} minTickGap={28} />
          <YAxis tickLine={false} axisLine={false} tick={{ fontSize: 10, fill: "var(--muted-foreground)", fontFamily: "var(--font-mono)" }} allowDecimals={false} width={28} />
          <Tooltip content={<ChartTooltip />} />
          <Area type="monotone" dataKey="total" name="Events" stroke={CHART_COLORS.accent} strokeWidth={1.5} fill="color-mix(in srgb, var(--accent) 12%, transparent)" />
          <Area type="monotone" dataKey="critical" name="Critical" stroke={CHART_COLORS.critical} fill="transparent" strokeWidth={1.25} dot={false} />
          <Area type="monotone" dataKey="error" name="Errors" stroke={CHART_COLORS.error} fill="transparent" strokeWidth={1.25} dot={false} />
          <Area type="monotone" dataKey="warn" name="Warnings" stroke={CHART_COLORS.warn} fill="transparent" strokeWidth={1.25} dot={false} />
        </AreaChart>
      )}
    </ResponsiveChartFrame>
  );
}

export function Sparkline({ values, tone = "primary" }: { values: number[]; tone?: "primary" | "warning" | "critical" | "success" }) {
  const color =
    tone === "critical"
      ? CHART_COLORS.critical
      : tone === "warning"
        ? CHART_COLORS.warn
        : tone === "success"
          ? "var(--success)"
          : CHART_COLORS.accent;
  const data = values.slice(-12).map((value, index) => ({ index: String(index + 1), value }));

  return (
    <ResponsiveChartFrame className="h-10">
      {({ width, height }) => (
        <BarChart width={width} height={height} data={data} margin={{ top: 2, right: 0, bottom: 0, left: 0 }}>
          <Bar dataKey="value" radius={[1, 1, 0, 0]} fill={color} opacity={0.85} />
        </BarChart>
      )}
    </ResponsiveChartFrame>
  );
}

function ChartTooltip({ active, payload, label }: { active?: boolean; payload?: Array<{ name?: string; value?: number }>; label?: string }) {
  if (!active || !payload?.length) return null;
  return (
    <div className="rounded-sm border border-border bg-popover px-2.5 py-2 text-xs shadow-sm">
      {label ? <p className="mb-1 font-data font-medium text-foreground">{label}</p> : null}
      {payload.map((item) => (
        <p key={item.name} className="text-muted-foreground">
          {item.name}: <span className="font-data font-medium text-foreground">{item.value ?? 0}</span>
        </p>
      ))}
    </div>
  );
}
