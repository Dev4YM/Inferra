import type { ReactNode } from "react";
import { Box, ChevronRight, Container, Database, HardDrive, Link2, Radio, Server, Settings2, TriangleAlert } from "lucide-react";
import { Link } from "react-router-dom";

import type { EventRow, ServiceRow } from "@/api";
import {
  AppActivityChart,
  AppCpuChart,
  AppErrorChart,
  AppMemoryChart,
  bucketLogsToTimeline,
} from "@/components/inferra/application-charts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { ServiceHealthBadge } from "@/components/inferra/health";
import { TraceSummaryInline } from "@/components/inferra/trace-summary";
import type { AttentionItem, InventoryApplication, InventoryDataStore, SystemsInventory } from "@/lib/systems-inventory";
import { formatHostCpuPercent } from "@/lib/systems-inventory";
import { formatDisplayValue, formatRelativeDate } from "@/lib/format";
import { useApiQuery } from "@/lib/query";
import { cn } from "@/lib/utils";

export function AttentionStrip({ items }: { items: AttentionItem[] }) {
  if (!items.length) {
    return (
      <Card className="border-emerald-400/25 bg-emerald-500/5">
        <CardContent className="flex items-center gap-3 p-4 text-sm">
          <div className="rounded-xl border border-emerald-400/30 bg-emerald-500/10 p-2">
            <Server className="size-4 text-emerald-600" />
          </div>
          <div>
            <p className="font-medium">Nothing needs you right now</p>
            <p className="text-muted-foreground">No degraded services, container faults, or unmapped runtimes flagged.</p>
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className="border-border bg-card/70">
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-2 text-base">
          <TriangleAlert className="size-4 text-warning" />
          What needs you
        </CardTitle>
        <CardDescription>Actionable runtime signals — not raw event totals.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-2">
        {items.slice(0, 8).map((item) => (
          <div
            key={item.id}
            className={cn(
              "flex flex-wrap items-center justify-between gap-3 rounded-md border px-4 py-3 text-sm",
              item.tone === "destructive"
                ? "border-rose-400/30 bg-rose-500/5"
                : item.tone === "warning"
                  ? "border-amber-400/30 bg-amber-500/5"
                  : "border-border bg-panel-inset",
            )}
          >
            <div className="min-w-0">
              <p className="font-medium">{item.title}</p>
              <p className="mt-1 text-muted-foreground">{item.detail}</p>
            </div>
            {item.href ? (
              <Button variant="outline" size="sm" asChild>
                <Link to={item.href}>
                  Inspect
                  <ChevronRight className="size-4" />
                </Link>
              </Button>
            ) : null}
          </div>
        ))}
        {items.length > 8 ? <p className="text-xs text-muted-foreground">+{items.length - 8} more signals in the inventory below.</p> : null}
      </CardContent>
    </Card>
  );
}

export function InventorySection({
  title,
  description,
  icon: Icon,
  count,
  emptyLabel,
  children,
}: {
  title: string;
  description: string;
  icon: typeof Server;
  count: number;
  emptyLabel: string;
  children: ReactNode;
}) {
  return (
    <Card>
      <CardHeader>
        <div className="flex items-start justify-between gap-3">
          <div>
            <CardTitle className="flex items-center gap-2 text-base">
              <Icon className="size-4 text-primary" />
              {title}
            </CardTitle>
            <CardDescription>{description}</CardDescription>
          </div>
          <Badge variant="outline">{count}</Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-2">
        {count ? children : <p className="text-sm text-muted-foreground">{emptyLabel}</p>}
      </CardContent>
    </Card>
  );
}

export function ServerInventoryCard({ inventory }: { inventory: SystemsInventory }) {
  const host = inventory.hostService;
  const topProcess = inventory.topProcesses[0];

  return (
    <div className="rounded-md border border-border bg-panel-inset p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <p className="font-medium">{inventory.hostname}</p>
          <p className="mt-1 text-sm text-muted-foreground">
            {inventory.topProcesses.length} watched processes · {inventory.containers.length} containers
          </p>
        </div>
        {host ? <ServiceHealthBadge status={host.status} /> : <Badge variant="outline">No host events</Badge>}
      </div>
      <div className="mt-4 grid gap-3 sm:grid-cols-3">
        <Metric label="Host signals" value={host ? String(host.event_count ?? 0) : "—"} hint={host ? `last ${formatRelativeDate(host.last_event_at)}` : "Enable host_metrics collector"} />
        <Metric
          label="Top CPU process"
          value={topProcess ? topProcess.name : "—"}
          hint={topProcess ? `${formatHostCpuPercent(topProcess.cpu_percent)} · ${Math.round(topProcess.memory_mb)} MB` : "Process scan idle"}
        />
        <Metric label="Host errors" value={host ? String(host.error_count ?? 0) : "—"} hint={host && host.error_count ? "Review resource pressure events" : "No host errors"} />
      </div>
      {host ? (
        <div className="mt-4">
          <Button variant="outline" size="sm" asChild>
            <Link to={`/systems/${encodeURIComponent(host.service_id)}`}>Open host evidence</Link>
          </Button>
        </div>
      ) : null}
    </div>
  );
}

export function ApplicationInventoryRow({ entry }: { entry: InventoryApplication }) {
  const { app, service, mappedServiceId } = entry;
  const title = app.display_name ?? app.name;
  const detailHref = service
    ? `/systems/${encodeURIComponent(service.service_id)}`
    : `/workspace/apps?name=${encodeURIComponent(app.name)}`;

  const logsQuery = useApiQuery<{ logs: EventRow[] }>(
    service ? `/api/logs?service=${encodeURIComponent(service.service_id)}&limit=64` : null,
    { staleTime: 30_000, refetchInterval: 30_000 },
  );
  const activityPoints = bucketLogsToTimeline(logsQuery.data?.logs ?? []);
  const lastSignal = service?.last_event_at ? formatRelativeDate(service.last_event_at) : null;
  const attention =
    service && ["critical", "degraded", "elevated"].includes(String(service.status).toLowerCase());
  const pathLabel = app.project_path?.split(/[/\\]/).slice(-2).join("/") ?? "";

  return (
    <div
      className={cn(
        "group flex h-full flex-col overflow-hidden rounded-md border bg-card shadow-sm transition-shadow hover:shadow-md",
        attention ? "border-warning/45" : "border-border",
      )}
    >
      <div className={cn("h-px w-full", attention ? "bg-warning" : "bg-border")} />

      <div className="flex flex-1 flex-col p-3">
        <div className="flex items-start gap-2.5">
          <div className="flex size-9 shrink-0 items-center justify-center rounded-sm border border-border bg-panel-inset">
            <Radio className="size-4 text-accent" />
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-start justify-between gap-2">
              <div className="min-w-0">
                <p className="truncate text-sm font-semibold leading-tight">{title}</p>
                <div className="mt-1 flex flex-wrap items-center gap-1">
                  {app.framework ? (
                    <Badge variant="outline" className="h-5 border-border/80 px-1.5 text-[10px] font-normal">
                      {formatDisplayValue(app.framework)}
                    </Badge>
                  ) : null}
                  <span className="font-data text-[10px] text-muted-foreground">
                    {formatDisplayValue(app.language ?? app.runtime)}
                    {app.process_kind ? ` · ${formatDisplayValue(app.process_kind)}` : ""}
                  </span>
                </div>
              </div>
              {service ? (
                <ServiceHealthBadge status={service.status} />
              ) : (
                <Badge variant="outline" className="shrink-0 text-[10px] font-normal">
                  Observed
                </Badge>
              )}
            </div>
            <p className="mt-1.5 font-data text-[10px] text-muted-foreground" title={app.project_path ?? undefined}>
              {app.pid ? `pid ${app.pid}` : "pid —"}
              {pathLabel ? ` · ${pathLabel}` : ""}
            </p>
          </div>
        </div>

        <div className="mt-3 grid grid-cols-2 gap-1.5">
          <div className="rounded-sm border border-border/70 bg-panel-inset/60 p-2">
            <AppCpuChart percent={app.resources?.cpu_percent} />
          </div>
          <div className="rounded-sm border border-border/70 bg-panel-inset/60 p-2">
            <AppMemoryChart memoryMb={app.resources?.memory_mb} virtualMb={app.resources?.virtual_memory_mb} />
          </div>
          <div className="rounded-sm border border-border/70 bg-panel-inset/60 p-2">
            <AppErrorChart
              errorCount={service?.error_count ?? 0}
              eventCount={service?.event_count ?? 0}
              errorRatio={service?.error_ratio}
            />
          </div>
          <div className="rounded-sm border border-border/70 bg-panel-inset/60 p-2">
            <AppActivityChart
              points={activityPoints}
              mapped={Boolean(mappedServiceId)}
              eventCount={service?.event_count}
              lastEventAt={lastSignal}
            />
          </div>
        </div>

        <div className="mt-2.5 flex items-center justify-between gap-2 border-t border-border/80 pt-2.5">
          <Button variant="outline" size="sm" className="h-7 gap-1 px-2.5 text-xs" asChild>
            <Link to={detailHref}>
              Inspect
              <ChevronRight className="size-3.5 opacity-60" />
            </Link>
          </Button>
          <span className="truncate font-data text-[10px] text-muted-foreground">
            {mappedServiceId ? mappedServiceId : "unmapped"}
          </span>
        </div>
      </div>
    </div>
  );
}

export function DataStoreInventoryRow({ store }: { store: InventoryDataStore }) {
  const href = store.service ? `/systems/${encodeURIComponent(store.service.service_id)}` : undefined;

  return (
    <div className="rounded-md border border-border bg-panel-inset p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <div className="flex flex-wrap items-center gap-2">
            <p className="font-medium">{store.label}</p>
            <Badge variant="outline">Data store</Badge>
            {store.monitored ? <Badge variant="success">Log signals</Badge> : <Badge variant="warning">Process only</Badge>}
          </div>
          <p className="mt-1 text-sm text-muted-foreground">{store.hint ?? "Inferra does not query this database directly."}</p>
        </div>
        {store.service ? <ServiceHealthBadge status={store.service.status} /> : null}
      </div>
      {store.service ? (
        <div className="mt-3 grid gap-3 sm:grid-cols-3">
          <Metric label="Errors" value={String(store.service.error_count ?? 0)} hint={`${store.service.event_count ?? 0} events`} />
          <Metric label="Error rate" value={`${Math.round((store.service.error_ratio ?? 0) * 100)}%`} hint="From attributed logs/events" />
          <Metric label="Last signal" value={formatRelativeDate(store.service.last_event_at)} hint="Connection/query errors appear here when ingested" />
        </div>
      ) : null}
      <div className="mt-3 flex flex-wrap gap-2">
        {href ? (
          <Button variant="outline" size="sm" asChild>
            <Link to={href}>Inspect evidence</Link>
          </Button>
        ) : (
          <Button variant="outline" size="sm" asChild>
            <Link to="/workspace">
              <Link2 className="size-4" />
              Wire monitoring
            </Link>
          </Button>
        )}
        <Button variant="ghost" size="sm" asChild>
          <Link to="/control">Collector setup</Link>
        </Button>
      </div>
    </div>
  );
}

export function ContainerInventoryRow({
  container,
}: {
  container: { name: string; image: string; state: string };
}) {
  const tone = container.state.toLowerCase() === "running" ? "success" : "warning";
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-border bg-panel-inset px-4 py-3 text-sm">
      <div className="min-w-0">
        <p className="font-medium">{container.name}</p>
        <p className="truncate text-muted-foreground">{container.image}</p>
      </div>
      <Badge variant={tone}>{formatDisplayValue(container.state)}</Badge>
    </div>
  );
}

export function ObservedServiceRow({ service }: { service: ServiceRow }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-border bg-panel-inset px-4 py-3 text-sm">
      <div className="min-w-0">
        <Link className="font-medium" to={`/systems/${encodeURIComponent(service.service_id)}`}>
          {service.service_id}
        </Link>
        <p className="text-muted-foreground">
          {service.event_count ?? 0} events · {service.error_count ?? 0} errors · last {formatRelativeDate(service.last_event_at)}
        </p>
      </div>
      <div className="flex items-center gap-2">
        <ServiceHealthBadge status={service.status} />
        <TraceSummaryInline summary={service.latest_trace_summary} context={{ from: "service", serviceId: service.service_id }} emptyLabel="" />
      </div>
    </div>
  );
}

export function DeveloperServiceRegistry({ services }: { services: ServiceRow[] }) {
  if (!services.length) return null;

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <Settings2 className="size-4" />
          Raw service registry
        </CardTitle>
        <CardDescription>Internal `service_id` rows as stored in Inferra — useful for API alignment and debugging.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-2">
        {services.map((service) => (
          <ObservedServiceRow key={service.service_id} service={service} />
        ))}
      </CardContent>
    </Card>
  );
}

function Metric({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-xl border border-border/50 bg-background/20 p-3">
      <p className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">{label}</p>
      <p className="mt-1 truncate font-medium">{value}</p>
      {hint ? <p className="mt-1 text-xs text-muted-foreground">{hint}</p> : null}
    </div>
  );
}

export function InventorySummaryStrip({ inventory }: { inventory: SystemsInventory }) {
  return (
    <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <SummaryPill icon={HardDrive} label="Server" value={inventory.hostname} detail={inventory.hostService ? formatDisplayValue(inventory.hostService.status) : "No host row"} />
      <SummaryPill icon={Box} label="Applications" value={String(inventory.applications.length)} detail="Live workspace runtimes" />
      <SummaryPill icon={Database} label="Data stores" value={String(inventory.dataStores.length)} detail={`${inventory.dataStores.filter((store) => store.monitored).length} with log signals`} />
      <SummaryPill icon={Container} label="Containers" value={String(inventory.containers.length)} detail={`${inventory.unmappedServices.length} unmapped services`} />
    </div>
  );
}

function SummaryPill({
  icon: Icon,
  label,
  value,
  detail,
}: {
  icon: typeof Server;
  label: string;
  value: string;
  detail: string;
}) {
  return (
    <div className="rounded-md border border-border bg-card/65 p-4">
      <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">
        <Icon className="size-3.5" />
        {label}
      </div>
      <p className="mt-2 text-xl font-semibold tracking-tight">{value}</p>
      <p className="mt-1 text-sm text-muted-foreground">{detail}</p>
    </div>
  );
}
