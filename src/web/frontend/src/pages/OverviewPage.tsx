import { Activity, AlertTriangle, Bot, FolderGit2, RefreshCcw, ServerCog, Sparkles } from "lucide-react";

import type { Mode } from "@/lib/experience";
import type { OverviewResponse } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState, MetricGridSkeleton } from "@/components/feedback/states";
import { formatRiskTone, formatSeverity, formatRelativeDate } from "@/lib/format";
import { useApiQuery } from "@/lib/query";
import { Link } from "react-router-dom";

export function OverviewPage({ mode }: { mode: Mode }) {
  const overview = useApiQuery<OverviewResponse>("/api/overview");

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

      <Card className="overflow-hidden">
        <CardContent className="grid gap-6 p-6 lg:grid-cols-[1.5fr_1fr]">
          <div className="space-y-4">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={formatRiskTone(quick.risk_level)}>risk {quick.risk_level}</Badge>
              <Badge variant={aiState.variant}>
                <Bot className="size-3.5" />
                {aiState.label}
              </Badge>
              <Badge variant="outline">{experience.ai_role}</Badge>
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
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-1">
            <QuickStat icon={AlertTriangle} label="Active incidents" value={String(incidents.length)} note="Open or investigating now" />
            <QuickStat icon={ServerCog} label="Risky services" value={String(riskyServices.length)} note="Critical, degraded, or elevated" />
            <QuickStat icon={FolderGit2} label="Workspace projects" value={String(projects.length)} note="Mapped to local runtime context" />
            <QuickStat icon={Sparkles} label="Safe actions" value={experience.suggest_safe_actions ? "suggest" : "disabled"} note="Never executed automatically" />
          </div>
        </CardContent>
      </Card>

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
            <HealthRow label="Status" value={health.status ?? "unknown"} />
            <HealthRow label="Queue depth" value={String(health.queue_depth ?? 0)} />
            <HealthRow label="Storage writes" value={String(health.storage_writes_ok ?? false)} />
            <HealthRow
              label="AI status"
              value={
                health.ai_enabled
                  ? (health.ai_available ? "available" : "degraded")
                  : "disabled"
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
                        • {reason}
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
                      <Badge variant={formatRiskTone(formatSeverity(incident.severity))}>sev {formatSeverity(incident.severity)}</Badge>
                      <Link className="text-sm font-medium" to={`/incidents/${incident.incident_id}`}>
                        Open incident
                      </Link>
                    </div>
                  </div>
                  <p className="mt-3 text-sm text-muted-foreground">
                    Updated {formatRelativeDate(incident.updated_at)} with {incident.event_count ?? 0} correlated events.
                  </p>
                </div>
              ))
            ) : (
              <EmptyState
                title="No active incidents"
                description="Inferra is quiet right now. Seed demo events or wait for collectors to observe failures."
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
                    <Badge variant={formatRiskTone(service.status)}>{service.status}</Badge>
                  </div>
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

