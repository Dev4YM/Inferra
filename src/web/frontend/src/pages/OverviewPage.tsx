import { useState } from "react";
import { Activity, AlertTriangle, Bot, Boxes, Cpu, FolderGit2, RefreshCcw, ServerCog, Sparkles } from "lucide-react";
import { Link } from "react-router-dom";

import type { Mode } from "@/lib/experience";
import type { CollectorRow, OverviewResponse } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState, MetricGridSkeleton } from "@/components/feedback/states";
import { formatDisplayValue, formatRiskTone, formatSeverity, formatSeverityLabel, formatRelativeDate } from "@/lib/format";
import { shortTraceId } from "@/lib/observability";
import { useApiQuery } from "@/lib/query";
import { EventRateBars, SeverityDistribution } from "@/components/inferra/charts";
import { IncidentCard } from "@/components/inferra/incident";
import { RuntimeStatusCard, ServiceHealthBadge, riskTone } from "@/components/inferra/health";
import { TraceSummaryInline } from "@/components/inferra/trace-summary";

export function OverviewPage({ mode }: { mode: Mode }) {
  const overview = useApiQuery<OverviewResponse>("/api/overview");
  const collectors = useApiQuery<{ collectors: CollectorRow[]; queue_depth: number }>("/api/collectors", { staleTime: 15_000 });
  const [quickFilter, setQuickFilter] = useState<"all" | "active" | "degraded">("all");

  if (overview.isLoading && !overview.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Overview" subtitle="What changed, what matters, and what to inspect next." mode={mode} />
        <MetricGridSkeleton />
        <LoadingState title="Loading observability snapshot" />
      </div>
    );
  }

  if (overview.errorMessage && !overview.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Overview" subtitle="What changed, what matters, and what to inspect next." mode={mode} />
        <ErrorState description={overview.errorMessage} onRetry={() => void overview.reload()} />
      </div>
    );
  }

  if (!overview.data) {
    return <EmptyState title="No overview available" description="Inferra has not produced a snapshot yet." />;
  }

  const { quick_analysis: quick, dashboard, workspace_projects: projects, experience } = overview.data;
  const health = dashboard.health ?? {};
  const incidents = dashboard.incidents ?? [];
  const services = dashboard.services ?? [];
  const riskyServices = services.filter((item) => ["critical", "degraded", "elevated"].includes(item.status));
  const activeIncidents = incidents.filter((incident) => incident.state !== "resolved");
  const traceLinkedIncidents = incidents.filter((incident) => Boolean(incident.latest_trace_summary));
  const traceLinkedServices = services.filter((service) => Boolean(service.latest_trace_summary));
  const visibleIncidents = quickFilter === "active" ? activeIncidents : incidents;
  const visibleServices = quickFilter === "degraded" ? riskyServices : services;
  const eventRate = normalizeEventRate(dashboard.event_rate);
  const severityCounts = normalizeSeverityCounts(dashboard.severity_counts);
  const collectorRows = collectors.data?.collectors ?? [];
  const collectorsWithErrors = collectorRows.filter((collector) => (collector.error_count ?? 0) > 0 || Boolean(collector.last_error));
  const activeCollectorErrors = collectorsWithErrors.filter((collector) => collector.status === "error");
  const collectorErrorCount = health.collector_errors ?? 0;
  const collectorErrorDetail = collectorErrorCount
    ? `${health.queue_depth ?? 0} queued events, ${collectorErrorCount} active collector errors across ${Math.max(activeCollectorErrors.length, 1)} collector${Math.max(activeCollectorErrors.length, 1) === 1 ? "" : "s"}`
    : `${health.queue_depth ?? 0} queued events, ${collectorsWithErrors.length} collectors with error history`;
  const aiState = health.ai_enabled
    ? health.ai_available
      ? { label: "AI ready", variant: "success" as const }
      : { label: "AI degraded", variant: "warning" as const }
    : { label: "AI disabled", variant: "secondary" as const };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Overview"
        subtitle="What is happening, what changed, and what to inspect next."
        mode={quick.mode}
        actions={
          <Button variant="outline" size="sm" onClick={() => void overview.reload({ silent: true })}>
            <RefreshCcw className={`size-4 ${overview.isRefreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      <div className="dashboard-grid">
        <RuntimeStatusCard
          icon={Activity}
          label="System health"
          value={health.status ?? quick.risk_level}
          tone={riskTone(health.status ?? quick.risk_level)}
          detail={collectorErrorDetail}
        />
        <RuntimeStatusCard
          icon={AlertTriangle}
          label="Active incidents"
          value={String(health.active_incidents ?? activeIncidents.length)}
          tone={(health.active_incidents ?? activeIncidents.length) ? "warning" : "success"}
          detail={`${traceLinkedIncidents.length} include a latest trace jump.`}
        />
        <RuntimeStatusCard
          icon={ServerCog}
          label="Monitored services"
          value={String(services.length)}
          tone={riskyServices.length ? "warning" : "success"}
          detail={`${traceLinkedServices.length} have latest trace context.`}
        />
        <RuntimeStatusCard
          icon={Bot}
          label="AI investigator"
          value={aiState.label}
          tone={aiState.variant === "warning" ? "warning" : aiState.variant === "success" ? "success" : "secondary"}
          detail={health.ai_reason ?? "Evidence-backed summaries and suggested checks."}
        />
      </div>

      {collectorErrorCount || activeCollectorErrors.length ? (
        <Alert variant="warning">
          <AlertTriangle className="size-4" />
          <div className="min-w-0 space-y-3">
            <div>
              <AlertTitle>Collector errors are degrading health</AlertTitle>
              <AlertDescription>
                {activeCollectorErrors.length ? (
                  activeCollectorErrors.slice(0, 3).map((collector) => (
                    <span key={collector.collector_id} className="block">
                      {collector.collector_id}: {collector.last_error ?? collector.error_hint ?? `${collector.error_count ?? 0} errors`}
                    </span>
                  ))
                ) : (
                  <span className="block">Open collector diagnostics to load the failing collector rows and share the current report.</span>
                )}
              </AlertDescription>
            </div>
            <Button asChild variant="outline" size="sm">
              <Link to="/control">Open collector diagnostics</Link>
            </Button>
          </div>
        </Alert>
      ) : null}

      <Card className="overflow-hidden">
        <CardContent className="grid gap-6 p-6 lg:grid-cols-[1.5fr_1fr]">
          <div className="space-y-4">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={formatRiskTone(quick.risk_level)}>Risk {formatDisplayValue(quick.risk_level)}</Badge>
              <Badge variant={aiState.variant}>
                <Bot className="size-3.5" />
                {aiState.label}
              </Badge>
              <Badge variant="outline">{formatDisplayValue(experience.ai_role)}</Badge>
            </div>
            <div>
              <p className="text-xs font-semibold uppercase tracking-[0.28em] text-primary/80">Quick analysis</p>
              <h2 className="mt-2 text-2xl font-semibold tracking-tight md:text-3xl">{quick.headline}</h2>
              <p className="mt-3 max-w-3xl text-sm leading-7 text-muted-foreground">
                Inferra keeps the system read-only and evidence-backed. This dashboard is optimized for smooth triage, safe
                inspection, and clear escalation.
              </p>
            </div>
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <QuickStat icon={FolderGit2} label="Workspace projects" value={String(projects.length)} note="Mapped to local runtime context" />
            <QuickStat icon={Boxes} label="Containers" value={String(overview.data.runtime.containers?.length ?? 0)} note="Observed from local runtime" />
            <QuickStat icon={Cpu} label="Process sample" value={String(quick.process_sample_size)} note="Local process context" />
            <QuickStat icon={Sparkles} label="Safe actions" value={experience.suggest_safe_actions ? "Suggest" : "Disabled"} note="Never executed automatically" />
          </div>
        </CardContent>
      </Card>

      <div className="flex flex-wrap gap-2">
        {[
          ["all", "All signals"],
          ["active", "Active incidents"],
          ["degraded", "Degraded services"],
        ].map(([value, label]) => (
          <Button
            key={value}
            type="button"
            variant={quickFilter === value ? "default" : "outline"}
            size="sm"
            aria-pressed={quickFilter === value}
            onClick={() => setQuickFilter(value as typeof quickFilter)}
          >
            {label}
          </Button>
        ))}
      </div>

      <section className="grid gap-4 xl:grid-cols-[minmax(0,1.45fr)_minmax(360px,0.85fr)]">
        <Card>
          <CardHeader>
            <CardTitle>Active incidents</CardTitle>
          </CardHeader>
          <CardContent>
            {visibleIncidents.length ? (
              <div className="grid gap-3 lg:grid-cols-2">
                {visibleIncidents.slice(0, 6).map((incident) => (
                  <IncidentCard key={incident.incident_id} incident={incident} />
                ))}
              </div>
            ) : (
              <EmptyState title="No active incidents" description="Inferra is quiet right now. Keep collectors running and refresh when needed." />
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Severity distribution</CardTitle>
          </CardHeader>
          <CardContent>
            <SeverityDistribution counts={severityCounts} />
          </CardContent>
        </Card>
      </section>

      <section className="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(360px,0.9fr)]">
        <Card>
          <CardHeader>
            <CardTitle>Recent anomaly timeline</CardTitle>
          </CardHeader>
          <CardContent>
            <EventRateBars points={eventRate} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Monitored services</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {visibleServices.slice(0, 7).map((service) => (
              <Link
                key={service.service_id}
                to={`/systems/${service.service_id}`}
                className="flex items-center justify-between gap-3 rounded-2xl border border-border/60 bg-background/35 p-4 text-foreground hover:bg-secondary/45 hover:opacity-100"
              >
                <div className="min-w-0">
                  <p className="truncate font-medium">{service.service_id}</p>
                  <p className="text-sm text-muted-foreground">
                    {service.event_count ?? 0} events, {service.error_count ?? 0} errors
                  </p>
                  {service.latest_trace_summary ? (
                    <p className="mt-1 text-xs text-muted-foreground">
                      Latest trace {shortTraceId(service.latest_trace_summary.trace_id)} · {service.latest_trace_summary.event_count} rows ·{" "}
                      {formatRelativeDate(service.latest_trace_summary.last_seen_at)}
                    </p>
                  ) : null}
                </div>
                <ServiceHealthBadge status={service.status} />
              </Link>
            ))}
            {!visibleServices.length ? <p className="text-sm text-muted-foreground">No monitored services reported yet.</p> : null}
          </CardContent>
        </Card>
      </section>

      <section className="grid gap-4 xl:grid-cols-[minmax(0,1.5fr)_minmax(360px,1fr)]">
        <Card>
          <CardHeader>
            <CardTitle>First-run path</CardTitle>
          </CardHeader>
          <CardContent className="grid gap-3 md:grid-cols-2">
            <GuideCard title="Write the local config" command="inferra setup" />
            <GuideCard title="Initialize local storage" command="inferra init-db" />
            <GuideCard title="Start the dashboard" command="inferra serve" />
            <GuideCard title="Inspect collector/runtime state" command="inferra collectors status" />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Platform health</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <HealthRow label="Status" value={formatDisplayValue(health.status ?? "unknown")} />
            <HealthRow label="Queue depth" value={String(health.queue_depth ?? 0)} />
            <HealthRow label="Storage writes" value={String(health.storage_writes_ok ?? false)} />
            <HealthRow
              label="AI status"
              value={
                health.ai_enabled
                  ? (health.ai_available ? "Available" : "Degraded")
                  : "Disabled"
              }
            />
            {health.degraded_reasons?.length ? (
              <Alert variant="warning">
                <AlertTriangle className="size-4" />
                <div className="min-w-0">
                  <AlertTitle>Degraded reasons</AlertTitle>
                  <AlertDescription>
                    {health.degraded_reasons.map((reason, index) => (
                      <span key={index} className="block">
                        - {reason}
                      </span>
                    ))}
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}
          </CardContent>
        </Card>
      </section>

      <div className="content-grid">
        <Card>
          <CardHeader>
            <CardTitle>Top concern</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            {incidents.length ? (
              incidents.slice(0, 3).map((incident) => (
                <div key={incident.incident_id} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                  <div className="flex flex-wrap items-center justify-between gap-3">
                    <div>
                      <p className="font-medium">{incident.primary_service || "unknown service"}</p>
                      <p className="text-sm text-muted-foreground">{incident.incident_id}</p>
                    </div>
                    <div className="flex items-center gap-2">
                      <Badge variant={formatRiskTone(formatSeverity(incident.severity))}>Sev {formatSeverityLabel(incident.severity)}</Badge>
                      <Link className="text-sm font-medium" to={`/incidents/${incident.incident_id}`}>
                        Open incident
                      </Link>
                    </div>
                  </div>
                  <p className="mt-3 text-sm text-muted-foreground">
                    Updated {formatRelativeDate(incident.updated_at)} with {incident.event_count ?? 0} correlated events.
                  </p>
                  {incident.latest_trace_summary ? (
                    <TraceSummaryInline
                      summary={incident.latest_trace_summary}
                      context={{ from: "incident", incidentId: incident.incident_id }}
                      className="mt-3"
                      emptyLabel="—"
                    />
                  ) : null}
                </div>
              ))
            ) : (
              <EmptyState
                title="No active incidents"
                description="Inferra is quiet right now. Keep collectors running and refresh when new evidence arrives."
                action={<Button onClick={() => void overview.reload({ silent: true })}>Refresh snapshot</Button>}
              />
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Services needing attention</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {riskyServices.length ? (
              riskyServices.slice(0, 6).map((service) => (
                <div key={service.service_id} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                  <div className="flex items-center justify-between gap-2">
                    <div>
                      <p className="font-medium">{service.service_id}</p>
                      <p className="text-sm text-muted-foreground">{service.event_count ?? 0} events observed</p>
                    </div>
                    <Badge variant={formatRiskTone(service.status)}>{formatDisplayValue(service.status)}</Badge>
                  </div>
                  {service.latest_trace_summary ? (
                    <TraceSummaryInline
                      summary={service.latest_trace_summary}
                      context={{ from: "service", serviceId: service.service_id }}
                      className="mt-3"
                      emptyLabel="—"
                    />
                  ) : null}
                </div>
              ))
            ) : (
              <p className="text-sm text-muted-foreground">No degraded services detected.</p>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function QuickStat({
  icon: Icon,
  label,
  value,
  note,
}: {
  icon: typeof Activity;
  label: string;
  value: string;
  note: string;
}) {
  return (
    <Card className="border-border/70 bg-background/35">
      <CardContent className="flex items-center gap-4 p-4">
        <div className="rounded-2xl border border-border/70 bg-secondary/70 p-3">
          <Icon className="size-5 text-primary" />
        </div>
        <div>
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-muted-foreground">{label}</p>
          <p className="mt-1 text-2xl font-semibold">{value}</p>
          <p className="text-xs text-muted-foreground">{note}</p>
        </div>
      </CardContent>
    </Card>
  );
}

function GuideCard({ title, command }: { title: string; command: string }) {
  return (
    <div className="rounded-2xl border border-border/60 bg-background/35 p-4">
      <p className="font-medium">{title}</p>
      <pre className="mt-3 overflow-x-auto rounded-xl border border-border/70 bg-background/75 p-3 text-xs text-primary">
        <code>{command}</code>
      </pre>
    </div>
  );
}

function HealthRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-2 rounded-xl border border-border/60 bg-background/30 px-3 py-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium">{value}</span>
    </div>
  );
}

function normalizeEventRate(value: unknown) {
  if (!Array.isArray(value)) {
    if (value && typeof value === "object") {
      const point = value as Record<string, unknown>;
      return [{
        label: "Current",
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
