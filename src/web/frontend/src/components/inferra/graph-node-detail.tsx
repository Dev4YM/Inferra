import type { ReactNode } from "react";
import { Activity, ArrowUpRight, Box, GitBranch, X } from "lucide-react";
import { Link } from "react-router-dom";

import type { AnomalyStatus, EventRow, IncidentRow, ServiceDetailResponse, ServiceRow, WorkspaceRuntimeApp } from "@/api";
import { bucketLogsToTimeline } from "@/components/inferra/application-charts";
import { EventRateBars } from "@/components/inferra/charts";
import { ServiceHealthBadge, SeverityIndicator } from "@/components/inferra/health";
import { IncidentCard } from "@/components/inferra/incident";
import { TraceSummaryInline } from "@/components/inferra/trace-summary";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ErrorState, LoadingState } from "@/components/feedback/states";
import { formatDisplayValue, formatRelativeDate, formatRiskTone, summarizeEvent } from "@/lib/format";
import { useApiQuery } from "@/lib/query";
import { cn } from "@/lib/utils";

import type { GraphSelection } from "@/components/inferra/correlation-graph";

type GraphNodeDetailProps = {
  selection: GraphSelection;
  services: ServiceRow[];
  incidents: IncidentRow[];
  runtimeApps: WorkspaceRuntimeApp[];
  onClose: () => void;
  className?: string;
};

export function GraphNodeDetail({ selection, services, incidents, runtimeApps, onClose, className }: GraphNodeDetailProps) {
  if (!selection) return null;

  return (
    <aside
      className={cn(
        "flex h-full min-h-0 w-full flex-col overflow-hidden border-border bg-card lg:border-l",
        className,
      )}
    >
      <div className="flex items-start justify-between gap-3 border-b border-border px-4 py-3">
        <div className="min-w-0">
          <p className="label-caps text-[10px] text-muted-foreground">{formatDisplayValue(selection.kind)}</p>
          <h2 className="truncate text-sm font-semibold">{selection.label}</h2>
        </div>
        <Button variant="ghost" size="icon" className="size-8 shrink-0" onClick={onClose} aria-label="Close details">
          <X className="size-4" />
        </Button>
      </div>
      <div className="flex-1 overflow-y-auto px-4 py-4">
        {selection.kind === "service" ? (
          <ServiceGraphDetail serviceId={selection.id} service={services.find((row) => row.service_id === selection.id)} />
        ) : null}
        {selection.kind === "incident" ? (
          <IncidentGraphDetail incident={incidents.find((row) => row.incident_id === selection.id)} />
        ) : null}
        {selection.kind === "app" ? (
          <AppGraphDetail app={runtimeApps.find((row) => row.name === selection.id)} />
        ) : null}
      </div>
    </aside>
  );
}

function ServiceGraphDetail({ serviceId, service }: { serviceId: string; service?: ServiceRow }) {
  const detail = useApiQuery<ServiceDetailResponse>(`/api/services/${encodeURIComponent(serviceId)}`, { deps: [serviceId] });
  const anomaly = useApiQuery<AnomalyStatus>(`/api/anomaly/${encodeURIComponent(serviceId)}/status`, { deps: [serviceId] });
  const logs = useApiQuery<{ logs: EventRow[] }>(`/api/logs?service=${encodeURIComponent(serviceId)}&limit=64`, {
    deps: [serviceId],
    staleTime: 20_000,
  });

  const row = detail.data?.service ?? service;
  const timeline = bucketLogsToTimeline(detail.data?.events ?? logs.data?.logs ?? [], 12);
  const linkedIncidents = detail.data?.incidents ?? row?.active_incidents ?? [];

  if (detail.isLoading && !row) return <LoadingState title="Loading service" />;
  if (detail.errorMessage && !row) return <ErrorState description={detail.errorMessage} onRetry={() => void detail.reload()} />;
  if (!row) return <p className="text-sm text-muted-foreground">Service not found in the current snapshot.</p>;

  return (
    <div className="space-y-4">
      <div className="rounded-md border border-border bg-panel-inset p-4">
        <div className="flex flex-wrap items-center gap-2">
          <ServiceHealthBadge status={row.status} />
          {anomaly.data ? <Badge variant={formatRiskTone(anomaly.data.status)}>{formatDisplayValue(anomaly.data.status)}</Badge> : null}
        </div>
        <dl className="mt-4 grid grid-cols-2 gap-3 text-sm">
          <Metric label="Events" value={String(row.event_count ?? 0)} />
          <Metric label="Errors" value={String(row.error_count ?? 0)} />
          <Metric label="Error ratio" value={`${Math.round((row.error_ratio ?? 0) * 100)}%`} />
          <Metric label="Last event" value={formatRelativeDate(row.last_event_at)} compact />
        </dl>
      </div>

      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm">Activity</CardTitle>
        </CardHeader>
        <CardContent>
          {timeline.length ? <EventRateBars points={timeline} /> : <p className="text-sm text-muted-foreground">No recent event buckets.</p>}
        </CardContent>
      </Card>

      {linkedIncidents.length ? (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm">Linked incidents</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {linkedIncidents.slice(0, 3).map((incident) => (
              <IncidentCard key={incident.incident_id} incident={incident} />
            ))}
          </CardContent>
        </Card>
      ) : null}

      {(detail.data?.events.length ?? logs.data?.logs.length) ? (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm">Latest signals</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            {(detail.data?.events ?? logs.data?.logs ?? []).slice(0, 5).map((event, index) => (
              <div key={event.event_id ?? index} className="rounded-sm border border-border bg-panel-inset p-3 text-sm">
                <div className="flex items-center justify-between gap-2">
                  <SeverityIndicator value={event.severity} />
                  <span className="font-data text-[10px] text-muted-foreground">{formatRelativeDate(event.timestamp)}</span>
                </div>
                <p className="mt-2 text-muted-foreground">{summarizeEvent(event)}</p>
              </div>
            ))}
          </CardContent>
        </Card>
      ) : null}

      {row.latest_trace_summary ? (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm">Latest trace</CardTitle>
          </CardHeader>
          <CardContent>
            <TraceSummaryInline summary={row.latest_trace_summary} context={{ from: "service", serviceId }} showMessage />
          </CardContent>
        </Card>
      ) : null}

      <Button variant="outline" size="sm" className="w-full" asChild>
        <Link to={`/systems/${encodeURIComponent(serviceId)}`}>
          Open in Systems
          <ArrowUpRight className="size-4" />
        </Link>
      </Button>
    </div>
  );
}

function IncidentGraphDetail({ incident }: { incident?: IncidentRow }) {
  if (!incident) return <p className="text-sm text-muted-foreground">Incident not found in the active set.</p>;

  return (
    <div className="space-y-4">
      <IncidentCard incident={incident} />
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm">Affected services</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-2">
          {[incident.primary_service, ...(incident.affected_services ?? [])]
            .filter(Boolean)
            .map((service) => (
              <Badge key={service} variant="outline">
                {service}
              </Badge>
            ))}
        </CardContent>
      </Card>
      <Button variant="outline" size="sm" className="w-full" asChild>
        <Link to={`/incidents/${encodeURIComponent(incident.incident_id)}`}>
          Open incident workspace
          <ArrowUpRight className="size-4" />
        </Link>
      </Button>
    </div>
  );
}

function AppGraphDetail({ app }: { app?: WorkspaceRuntimeApp }) {
  if (!app) return <p className="text-sm text-muted-foreground">Application not found in the workspace scan.</p>;

  return (
    <div className="space-y-4">
      <div className="rounded-md border border-border bg-panel-inset p-4">
        <div className="flex items-center gap-2">
          <Box className="size-4 text-accent" />
          <span className="text-sm font-semibold">{app.display_name || app.name}</span>
        </div>
        <dl className="mt-4 space-y-2 text-sm">
          <div className="flex justify-between gap-3">
            <dt className="text-muted-foreground">Runtime</dt>
            <dd className="font-data">{formatDisplayValue(app.runtime)}</dd>
          </div>
          {app.language ? (
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">Language</dt>
              <dd className="font-data">{formatDisplayValue(app.language)}</dd>
            </div>
          ) : null}
          {app.framework ? (
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">Framework</dt>
              <dd className="font-data">{formatDisplayValue(app.framework)}</dd>
            </div>
          ) : null}
          {app.status ? (
            <div className="flex justify-between gap-3">
              <dt className="text-muted-foreground">State</dt>
              <dd className="font-data">{formatDisplayValue(app.status)}</dd>
            </div>
          ) : null}
        </dl>
      </div>

      {app.resources ? (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm">Resources</CardTitle>
          </CardHeader>
          <CardContent className="grid grid-cols-2 gap-3 text-sm">
            <Metric label="CPU" value={`${Math.round(app.resources.cpu_percent ?? 0)}%`} icon={<Activity className="size-3.5" />} />
            <Metric label="Memory" value={`${Math.round(app.resources.memory_mb ?? 0)} MB`} icon={<GitBranch className="size-3.5" />} />
          </CardContent>
        </Card>
      ) : null}

      {app.latest_trace_summary ? (
        <Card>
          <CardHeader className="pb-2">
            <CardTitle className="text-sm">Latest trace</CardTitle>
          </CardHeader>
          <CardContent>
            <TraceSummaryInline summary={app.latest_trace_summary} context={{ from: "workspace", appName: app.name }} showMessage />
          </CardContent>
        </Card>
      ) : null}

      <Button variant="outline" size="sm" className="w-full" asChild>
        <Link to="/systems">
          Open Systems inventory
          <ArrowUpRight className="size-4" />
        </Link>
      </Button>
    </div>
  );
}

function Metric({
  label,
  value,
  compact,
  icon,
}: {
  label: string;
  value: string;
  compact?: boolean;
  icon?: ReactNode;
}) {
  return (
    <div>
      <dt className="inline-flex items-center gap-1 text-[10px] uppercase tracking-wide text-muted-foreground">
        {icon}
        {label}
      </dt>
      <dd className={cn("mt-1 font-data font-semibold text-foreground", compact ? "text-xs" : "text-sm")}>{value}</dd>
    </div>
  );
}
