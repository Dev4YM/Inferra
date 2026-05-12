import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { Card, CardContent } from "@/components/ui/card";
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
  error: "#f43f5e",
  warn: "var(--warning)",
  info: "var(--primary)",
  debug: "#94a3b8",
};

export function SeverityDistribution({ counts }: { counts: Record<string, number> }) {
  const data = ["critical", "error", "warn", "info", "debug"]
    .map((key) => ({ key, name: formatDisplayValue(key), value: counts[key] ?? 0 }))
    .filter((item) => item.value > 0);

  if (!data.length) {
    return <p className="text-sm text-muted-foreground">No severity data available yet.</p>;
  }

  return (
    <div className="grid gap-4 md:grid-cols-[180px_minmax(0,1fr)]">
      <div className="h-44">
        <ResponsiveContainer width="100%" height="100%">
          <PieChart>
            <Pie data={data} dataKey="value" nameKey="name" innerRadius={50} outerRadius={78} paddingAngle={3}>
              {data.map((entry) => (
                <Cell key={entry.key} fill={CHART_COLORS[entry.key as keyof typeof CHART_COLORS] ?? "var(--primary)"} />
              ))}
            </Pie>
            <Tooltip content={<ChartTooltip />} />
          </PieChart>
        </ResponsiveContainer>
      </div>
      <div className="space-y-2">
        {data.map((item) => (
          <div key={item.key} className="flex items-center justify-between gap-3 rounded-xl border border-border/60 bg-background/35 px-3 py-2 text-sm">
            <span className="inline-flex items-center gap-2 text-muted-foreground">
              <span
                className="size-2.5 rounded-full"
                style={{ background: CHART_COLORS[item.key as keyof typeof CHART_COLORS] ?? "var(--primary)" }}
              />
              {item.name}
            </span>
            <span className="font-medium">{item.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function EventRateBars({ points }: { points: TimelinePoint[] }) {
  const data = points.slice(-32);

  if (!data.length) {
    return <p className="text-sm text-muted-foreground">No recent event rate data available.</p>;
  }

  return (
    <div className="h-64">
      <ResponsiveContainer width="100%" height="100%">
        <AreaChart data={data} margin={{ left: -18, right: 12, top: 8, bottom: 0 }}>
          <defs>
            <linearGradient id="eventsFill" x1="0" x2="0" y1="0" y2="1">
              <stop offset="5%" stopColor="var(--primary)" stopOpacity={0.28} />
              <stop offset="95%" stopColor="var(--primary)" stopOpacity={0.02} />
            </linearGradient>
          </defs>
          <CartesianGrid stroke="var(--border)" strokeDasharray="4 4" vertical={false} />
          <XAxis dataKey="label" tickLine={false} axisLine={false} tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} minTickGap={24} />
          <YAxis tickLine={false} axisLine={false} tick={{ fontSize: 11, fill: "var(--muted-foreground)" }} allowDecimals={false} />
          <Tooltip content={<ChartTooltip />} />
          <Area type="monotone" dataKey="total" name="Events" stroke="var(--primary)" strokeWidth={2.5} fill="url(#eventsFill)" />
          <Area type="monotone" dataKey="critical" name="Critical" stroke="var(--critical)" fill="transparent" strokeWidth={2} />
          <Area type="monotone" dataKey="error" name="Errors" stroke="#f43f5e" fill="transparent" strokeWidth={2} />
          <Area type="monotone" dataKey="warn" name="Warnings" stroke="var(--warning)" fill="transparent" strokeWidth={2} />
        </AreaChart>
      </ResponsiveContainer>
    </div>
  );
}

export function Sparkline({ values, tone = "primary" }: { values: number[]; tone?: "primary" | "warning" | "critical" | "success" }) {
  const color = tone === "critical" ? "var(--critical)" : tone === "warning" ? "var(--warning)" : tone === "success" ? "var(--success)" : "var(--primary)";
  const data = values.slice(-12).map((value, index) => ({ index: String(index + 1), value }));

  return (
    <div className="h-12">
      <ResponsiveContainer width="100%" height="100%">
        <BarChart data={data} margin={{ top: 3, right: 0, bottom: 0, left: 0 }}>
          <Bar dataKey="value" radius={[5, 5, 2, 2]} fill={color} opacity={0.72} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  );
}

function ChartTooltip({ active, payload, label }: { active?: boolean; payload?: Array<{ name?: string; value?: number }>; label?: string }) {
  if (!active || !payload?.length) return null;
  return (
    <Card className="rounded-xl border-border/80 bg-popover/95 shadow-lg">
      <CardContent className="space-y-1 p-3 text-xs">
        {label ? <p className="font-medium text-foreground">{label}</p> : null}
        {payload.map((item) => (
          <p key={item.name} className="text-muted-foreground">
            {item.name}: <span className="font-medium text-foreground">{item.value ?? 0}</span>
          </p>
        ))}
      </CardContent>
    </Card>
  );
}
