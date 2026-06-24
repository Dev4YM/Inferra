import { useState } from "react";
import { Activity, AlertTriangle, Bot, RefreshCcw, ServerCog } from "lucide-react";
import { Link } from "react-router-dom";

import type { Mode } from "@/lib/experience";
import type { CollectorRow, OverviewResponse } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/layout/page-header";
import { CodeBlock, DataRow, FilterBar, FilterChip } from "@/components/layout/console-patterns";
import { EmptyState, ErrorState, LoadingState, MetricGridSkeleton } from "@/components/feedback/states";
import { formatDisplayValue, formatRiskTone, formatRelativeDate } from "@/lib/format";
import { shortTraceId } from "@/lib/observability";
import { isAdvancedMode } from "@/lib/experience";
import { useInferraRuntime } from "@/lib/inferra-runtime";
import { summarizeCollectorFleet } from "@/lib/collectors";
import { useApiQuery } from "@/lib/query";
import { EventRateBars, SeverityDistribution } from "@/components/inferra/charts";
import { IncidentCard } from "@/components/inferra/incident";
import { RuntimeStatusCard, ServiceHealthBadge, riskTone } from "@/components/inferra/health";

export type OverviewPageContentProps = {
  mode: Mode;
  data: OverviewResponse;
  collectorRows?: CollectorRow[];
  runtimeState?: "loading" | "online" | "degraded" | "auth_required" | "offline";
  onRefresh?: () => void;
  isRefreshing?: boolean;
};

export function OverviewPageContent({
  mode,
  data,
  collectorRows = [],
  runtimeState = "online",
  onRefresh,
  isRefreshing = false,
}: OverviewPageContentProps) {
  const [quickFilter, setQuickFilter] = useState<"all" | "active" | "degraded">("all");

  const { quick_analysis: quick, dashboard, workspace_projects: projects, experience, runtime: runtimeContext } = data;
  const readiness = data.readiness ?? legacyReadiness(data);
  const health = dashboard.health ?? {};
  const incidents = dashboard.incidents ?? [];
  const services = dashboard.services ?? [];
  const riskyServices = services.filter((item) => ["critical", "degraded", "elevated"].includes(item.status));
  const activeIncidents = incidents.filter((incident) => incident.state !== "resolved");
  const visibleIncidents = quickFilter === "active" ? activeIncidents : incidents;
  const visibleServices = quickFilter === "degraded" ? riskyServices : services;
  const eventRate = normalizeEventRate(dashboard.event_rate);
  const severityCounts = normalizeSeverityCounts(dashboard.severity_counts);
  const fleet = summarizeCollectorFleet(collectorRows);
  const activeCollectorErrors = collectorRows.filter((collector) => collector.status === "error" || (collector.error_count ?? 0) > 0);
  const collectorErrorCount = health.collector_errors ?? 0;
  const collectorsUnderpowered = fleet.supported > 0 && fleet.running < fleet.supported;
  const platformDegraded =
    Boolean(health.degraded) ||
    runtimeState === "degraded" ||
    runtimeState === "offline" ||
    collectorsUnderpowered ||
    collectorErrorCount > 0;
  const platformLabel = platformDegraded
    ? collectorsUnderpowered
      ? "degraded"
      : health.status ?? "degraded"
    : health.status ?? quick.risk_level;
  const platformDetail = collectorsUnderpowered
    ? `${fleet.idle} of ${fleet.supported} collectors idle - ${fleet.idleCollectors
        .slice(0, 3)
        .map((collector) => collector.collector_id)
        .join(", ")}`
    : collectorErrorCount
      ? `${collectorErrorCount} collector errors`
      : fleet.unsupported
        ? `Collectors nominal - ${fleet.unsupported} unsupported on this host`
        : "Collectors nominal";
  const aiState = health.ai_enabled
    ? health.ai_available
      ? { label: "Ready", variant: "success" as const }
      : { label: "Degraded", variant: "warning" as const }
    : { label: "Off", variant: "secondary" as const };

  return (
    <div className="space-y-5">
      <PageHeader
        title="Overview"
        subtitle="Current runtime situation and where to look next."
        mode={quick.mode}
        actions={
          onRefresh ? (
            <Button variant="outline" size="sm" onClick={onRefresh}>
              <RefreshCcw className={`size-4 ${isRefreshing ? "animate-spin" : ""}`} />
              Refresh
            </Button>
          ) : null
        }
      />

      <Card>
        <CardContent className="space-y-3 p-4">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={formatRiskTone(quick.risk_level)}>Risk {formatDisplayValue(quick.risk_level)}</Badge>
            <Badge variant={aiState.variant}>{aiState.label}</Badge>
            <Badge variant="outline">{formatDisplayValue(experience.ai_role)}</Badge>
            {health.degraded || platformDegraded ? <Badge variant="warning">Platform degraded</Badge> : null}
            {collectorsUnderpowered ? <Badge variant="warning">{fleet.idle} collectors idle</Badge> : null}
            {fleet.unsupported ? <Badge variant="outline">{fleet.unsupported} unsupported on this host</Badge> : null}
          </div>
          <h2 className="text-xl font-semibold tracking-tight">{quick.headline}</h2>
          <div className="flex flex-wrap gap-x-4 gap-y-1 font-data text-xs text-muted-foreground">
            <span>{projects.length} projects</span>
            <span>{runtimeContext.containers?.length ?? 0} containers</span>
            <span>{quick.process_sample_size} processes sampled</span>
            <span>queue {health.queue_depth ?? 0}</span>
          </div>
        </CardContent>
      </Card>

      <section className="grid gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(320px,0.8fr)]">
        <Card>
          <CardHeader className="space-y-3">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <CardTitle>Operational readiness</CardTitle>
              <div className="flex items-center gap-2">
                <Badge variant={readinessBadgeVariant(readiness.status)}>{formatDisplayValue(readiness.status)}</Badge>
                <Badge variant="outline">Score {readiness.score}</Badge>
              </div>
            </div>
            <div className="space-y-2">
              <div className="h-2 overflow-hidden rounded bg-panel-inset">
                <div className="h-full rounded bg-accent transition-[width]" style={{ width: `${Math.max(0, Math.min(100, readiness.score))}%` }} />
              </div>
              <p className="text-base font-medium">{readiness.headline}</p>
              <p className="text-sm text-muted-foreground">{readiness.summary}</p>
            </div>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              <ReadinessMetric label="Services" value={String(readiness.services_observed)} />
              <ReadinessMetric label="Mapped" value={String(readiness.mapped_services)} />
              <ReadinessMetric label="Unmapped" value={String(readiness.unmapped_services)} />
              <ReadinessMetric label="Runtime apps" value={String(readiness.runtime_apps_detected)} />
            </div>
            {readiness.blockers.length ? (
              <div className="space-y-2">
                {readiness.blockers.slice(0, 4).map((blocker) => (
                  <div key={blocker.id} className="rounded-md border border-border bg-panel-inset px-3 py-3">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge variant={riskTone(blocker.severity)}>{formatDisplayValue(blocker.severity)}</Badge>
                      <span className="text-sm font-medium">{blocker.title}</span>
                    </div>
                    <p className="mt-2 text-sm text-muted-foreground">{blocker.detail}</p>
                  </div>
                ))}
              </div>
            ) : (
              <Alert>
                <div className="min-w-0">
                  <AlertTitle>Core setup layers are in place</AlertTitle>
                  <AlertDescription>
                    Inferra is receiving evidence and has enough ownership context to make the dashboard actionable.
                  </AlertDescription>
                </div>
              </Alert>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Next actions</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            {readiness.next_actions.length ? (
              readiness.next_actions.map((action) => (
                <div key={action.id} className="space-y-3 rounded-md border border-border bg-panel-inset p-4">
                  <div>
                    <p className="font-medium">{action.title}</p>
                    <p className="mt-1 text-sm text-muted-foreground">{action.detail}</p>
                  </div>
                  {action.command ? <CodeBlock>{action.command}</CodeBlock> : null}
                  {action.href ? (
                    <Button asChild variant="outline" size="sm">
                      <Link to={action.href}>Open {formatDisplayValue(action.category)}</Link>
                    </Button>
                  ) : null}
                </div>
              ))
            ) : (
              <p className="text-sm text-muted-foreground">No immediate setup blockers. Use Systems and Incidents to investigate live behavior.</p>
            )}
            {readiness.strengths.length ? (
              <div className="space-y-2">
                <p className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">Already working</p>
                <div className="space-y-2">
                  {readiness.strengths.slice(0, 4).map((item) => (
                    <p key={item} className="rounded-md border border-border/70 px-3 py-2 text-sm text-muted-foreground">
                      {item}
                    </p>
                  ))}
                </div>
              </div>
            ) : null}
          </CardContent>
        </Card>
      </section>

      <div className="dashboard-grid">
        <RuntimeStatusCard
          icon={Activity}
          label="Platform"
          value={platformLabel}
          tone={platformDegraded ? "warning" : riskTone(health.status ?? quick.risk_level)}
          detail={platformDetail}
        />
        <RuntimeStatusCard
          icon={AlertTriangle}
          label="Incidents"
          value={String(health.active_incidents ?? activeIncidents.length)}
          tone={(health.active_incidents ?? activeIncidents.length) ? "warning" : "success"}
          detail={`${activeIncidents.length} open`}
        />
        <RuntimeStatusCard
          icon={ServerCog}
          label="Services"
          value={String(services.length)}
          tone={riskyServices.length ? "warning" : "success"}
          detail={`${riskyServices.length} need attention`}
        />
        <RuntimeStatusCard
          icon={Bot}
          label="AI"
          value={aiState.label}
          tone={aiState.variant === "warning" ? "warning" : aiState.variant === "success" ? "success" : "secondary"}
          detail={health.ai_reason ?? "Read-only investigation"}
        />
      </div>

      {collectorErrorCount || activeCollectorErrors.length ? (
        <Alert variant="warning">
          <AlertTriangle className="size-4" />
          <div className="min-w-0 space-y-2">
            <AlertTitle>Collector errors affecting health</AlertTitle>
            <AlertDescription>
              {activeCollectorErrors.slice(0, 3).map((collector) => (
                <span key={collector.collector_id} className="block font-data text-xs">
                  {collector.collector_id}: {collector.last_error ?? collector.error_hint ?? `${collector.error_count ?? 0} errors`}
                </span>
              ))}
            </AlertDescription>
            <Button asChild variant="outline" size="sm">
              <Link to="/control">Diagnostics</Link>
            </Button>
          </div>
        </Alert>
      ) : null}

      <FilterBar>
        <FilterChip active={quickFilter === "all"} onClick={() => setQuickFilter("all")}>
          All
        </FilterChip>
        <FilterChip active={quickFilter === "active"} onClick={() => setQuickFilter("active")}>
          Open incidents ({activeIncidents.length})
        </FilterChip>
        <FilterChip active={quickFilter === "degraded"} onClick={() => setQuickFilter("degraded")}>
          Degraded services ({riskyServices.length})
        </FilterChip>
      </FilterBar>

      <section className="grid gap-4 xl:grid-cols-[minmax(0,1.4fr)_minmax(320px,0.8fr)]">
        <Card>
          <CardHeader className="flex-row items-center justify-between space-y-0">
            <CardTitle>Incidents</CardTitle>
            <Button variant="ghost" size="sm" asChild>
              <Link to="/incidents">View all</Link>
            </Button>
          </CardHeader>
          <CardContent>
            {visibleIncidents.length ? (
              <div className="grid gap-2 lg:grid-cols-2">
                {visibleIncidents.slice(0, 6).map((incident) => (
                  <IncidentCard key={incident.incident_id} incident={incident} />
                ))}
              </div>
            ) : (
              <p className="text-sm text-muted-foreground">No incidents in this filter.</p>
            )}
          </CardContent>
        </Card>

        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Severity</CardTitle>
            </CardHeader>
            <CardContent>
              <SeverityDistribution counts={severityCounts} />
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>Event rate</CardTitle>
            </CardHeader>
            <CardContent>
              <EventRateBars points={eventRate} />
            </CardContent>
          </Card>
        </div>
      </section>

      <Card>
        <CardHeader className="flex-row items-center justify-between space-y-0">
          <CardTitle>Services</CardTitle>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/systems">Runtime inventory</Link>
          </Button>
        </CardHeader>
        <CardContent className="divide-y divide-border">
          {visibleServices.slice(0, 8).map((service) => (
            <Link
              key={service.service_id}
              to={`/systems/${service.service_id}`}
              className="flex items-center justify-between gap-3 py-2.5 first:pt-0 last:pb-0 hover:opacity-90"
            >
              <div className="min-w-0">
                <p className="font-medium">{service.service_id}</p>
                <p className="font-data text-xs text-muted-foreground">
                  {service.event_count ?? 0} events · {service.error_count ?? 0} errors
                  {service.latest_trace_summary
                    ? ` · trace ${shortTraceId(service.latest_trace_summary.trace_id)}`
                    : ""}
                </p>
              </div>
              <ServiceHealthBadge status={service.status} />
            </Link>
          ))}
          {!visibleServices.length ? <p className="text-sm text-muted-foreground">No services match this filter.</p> : null}
        </CardContent>
      </Card>

      {isAdvancedMode(mode) ? (
        <section className="content-grid">
          <Card>
            <CardHeader>
              <CardTitle>CLI quick reference</CardTitle>
            </CardHeader>
            <CardContent className="grid gap-3 sm:grid-cols-2">
              <div>
                <p className="text-sm font-medium">Setup</p>
                <CodeBlock>inferra setup</CodeBlock>
              </div>
              <div>
                <p className="text-sm font-medium">Storage</p>
                <CodeBlock>inferra init-db</CodeBlock>
              </div>
              <div>
                <p className="text-sm font-medium">Serve UI</p>
                <CodeBlock>inferra serve</CodeBlock>
              </div>
              <div>
                <p className="text-sm font-medium">Collectors</p>
                <CodeBlock>inferra collectors status</CodeBlock>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Platform</CardTitle>
            </CardHeader>
            <CardContent>
              <DataRow label="Status" value={formatDisplayValue(health.status ?? "unknown")} mono />
              <DataRow label="Queue" value={String(health.queue_depth ?? 0)} mono />
              <DataRow label="Storage writes" value={health.storage_writes_ok ? "ok" : "failed"} mono />
              <DataRow
                label="AI"
                value={health.ai_enabled ? (health.ai_available ? "available" : "degraded") : "disabled"}
                mono
              />
              {health.degraded_reasons?.map((reason, index) => (
                <DataRow key={index} label="Degraded" value={reason} />
              ))}
            </CardContent>
          </Card>
        </section>
      ) : null}
    </div>
  );
}

export function OverviewPage({ mode }: { mode: Mode }) {
  const inferraRuntime = useInferraRuntime();
  const overview = useApiQuery<OverviewResponse>("/api/overview", { staleTime: 15_000 });
  const collectors = useApiQuery<{ collectors: CollectorRow[]; queue_depth: number }>("/api/collectors", { staleTime: 15_000 });

  if (overview.isLoading && !overview.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Overview" subtitle="Current runtime situation and where to look next." mode={mode} />
        <MetricGridSkeleton />
        <LoadingState title="Loading snapshot" />
      </div>
    );
  }

  if (overview.errorMessage && !overview.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Overview" subtitle="Current runtime situation and where to look next." mode={mode} />
        <ErrorState description={overview.errorMessage} onRetry={() => void overview.reload()} />
      </div>
    );
  }

  if (!overview.data) {
    return <EmptyState title="No overview available" description="Inferra has not produced a snapshot yet." />;
  }

  return (
    <OverviewPageContent
      mode={mode}
      data={overview.data}
      collectorRows={collectors.data?.collectors ?? []}
      runtimeState={inferraRuntime.state}
      onRefresh={() => void overview.reload({ silent: true })}
      isRefreshing={overview.isRefreshing}
    />
  );
}

function normalizeEventRate(value: unknown) {
  if (!Array.isArray(value)) {
    if (value && typeof value === "object") {
      const point = value as Record<string, unknown>;
      return [{
        label: "now",
        total: numberValue(point.events ?? point.total),
        warn: numberValue(point.warn),
        error: numberValue(point.error),
        critical: numberValue(point.critical),
      }];
    }
    return [];
  }
  return value
    .filter((point): point is Record<string, unknown> => Boolean(point && typeof point === "object" && !Array.isArray(point)))
    .map((point) => ({
      label: formatRelativeDate(typeof point.timestamp === "string" ? point.timestamp : undefined),
      total: numberValue(point.total),
      warn: numberValue(point.warn),
      error: numberValue(point.error),
      critical: numberValue(point.critical),
    }));
}

function normalizeSeverityCounts(value: unknown): Record<string, number> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const counts: Record<string, number> = {};
  for (const [key, raw] of Object.entries(value)) {
    const normalizedKey = severityKey(key);
    counts[normalizedKey] = (counts[normalizedKey] ?? 0) + numberValue(raw);
  }
  return counts;
}

function severityKey(value: string): string {
  switch (value.toLowerCase()) {
    case "0":
      return "debug";
    case "1":
      return "info";
    case "2":
      return "warn";
    case "3":
      return "error";
    case "4":
      return "critical";
    default:
      return value.toLowerCase();
  }
}

function numberValue(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function legacyReadiness(data: OverviewResponse) {
  const services = data.dashboard.services ?? [];
  const incidents = data.dashboard.incidents ?? [];
  const unmappedApprox = Math.max(0, services.length - Math.min(data.workspace_projects.length, services.length));
  const status = services.length === 0 ? "cold" : data.workspace_projects.length === 0 ? "partial" : "ready";
  return {
    status,
    score: status === "ready" ? 70 : status === "partial" ? 45 : 20,
    headline:
      status === "ready"
        ? "Inferra has baseline context, but restart the runtime to unlock the richer readiness model."
        : "This runtime is still serving an older overview payload, so readiness guidance is limited until the backend restarts.",
    summary:
      status === "ready"
        ? "A compatibility fallback is active while the running backend catches up with the latest overview contract."
        : "The dashboard can still show baseline state, but the new setup guidance requires the updated Rust runtime.",
    services_observed: services.length,
    mapped_services: Math.min(data.workspace_projects.length, services.length),
    unmapped_services: unmappedApprox,
    runtime_apps_detected: 0,
    blockers:
      status === "ready"
        ? []
        : [
            {
              id: "legacy-overview-runtime",
              category: "runtime",
              severity: "warning",
              title: "Backend restart still pending",
              detail: "The running runtime has not picked up the latest overview readiness payload yet.",
            },
          ],
    next_actions: [
      {
        id: "legacy-runtime-restart",
        category: "control",
        title: "Restart the running runtime",
        detail: "Reload the deployed Rust runtime so the dashboard can receive the new readiness assessment.",
        href: "/control",
        command: "inferra runtime restart",
      },
    ],
    strengths: incidents.length ? [`${incidents.length} incident(s) are already available in the current snapshot.`] : [],
  };
}

function ReadinessMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border border-border bg-panel-inset px-3 py-3">
      <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">{label}</p>
      <p className="mt-2 text-2xl font-semibold">{value}</p>
    </div>
  );
}

function readinessBadgeVariant(status: string) {
  switch (status) {
    case "ready":
      return "success" as const;
    case "blocked":
      return "destructive" as const;
    case "cold":
    case "partial":
      return "warning" as const;
    default:
      return "secondary" as const;
  }
}
