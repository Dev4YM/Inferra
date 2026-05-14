import { Activity, AlertTriangle, Gauge, RefreshCcw, ServerCog } from "lucide-react";
import { useState } from "react";
import { Link, useParams } from "react-router-dom";

import type {
  AnomalyStatus,
  AiGeneration,
  AiGenerationsResponse,
  InvestigationResponse,
  ServiceDetailResponse,
  ServiceRow,
  TopologyEdge,
  WorkspaceMapResponse,
} from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Td, Th, Table, TableWrap } from "@/components/ui/table";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { InvestigationView } from "@/components/investigation/investigation-view";
import type { Mode } from "@/lib/experience";
import { isAdvancedMode } from "@/lib/experience";
import { formatDisplayValue, formatRiskTone, formatRelativeDate, formatSeverityLabel, summarizeEvent } from "@/lib/format";
import { useApiQuery } from "@/lib/query";
import { RuntimeIdentity, RuntimeStatusCard, ServiceHealthBadge, riskTone } from "@/components/inferra/health";
import { Sparkline } from "@/components/inferra/charts";

export function SystemsPage({ mode }: { mode: Mode }) {
  const services = useApiQuery<{ services: ServiceRow[] }>("/api/services");

  if (services.isLoading && !services.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Systems" subtitle="Services, processes, and dependency health." mode={mode} />
        <LoadingState title="Loading systems" />
      </div>
    );
  }

  if (services.errorMessage && !services.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Systems" subtitle="Services, processes, and dependency health." mode={mode} />
        <ErrorState description={services.errorMessage} onRetry={() => void services.reload()} />
      </div>
    );
  }

  const rows = services.data?.services ?? [];
  const degraded = rows.filter((service) => ["critical", "degraded", "elevated"].includes(service.status));
  const totalEvents = rows.reduce((sum, service) => sum + (service.event_count ?? 0), 0);
  const totalErrors = rows.reduce((sum, service) => sum + (service.error_count ?? 0), 0);

  return (
    <div className="space-y-6">
      <PageHeader
        title="Systems"
        subtitle="Observed services, their status, and recent event volume."
        mode={mode}
        actions={
          <Button variant="outline" size="sm" onClick={() => void services.reload({ silent: true })}>
            <RefreshCcw className={`size-4 ${services.isRefreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      <div className="dashboard-grid">
        <RuntimeStatusCard icon={ServerCog} label="Services" value={String(rows.length)} tone="info" detail="Observed services with normalized runtime events." />
        <RuntimeStatusCard icon={AlertTriangle} label="Needs attention" value={String(degraded.length)} tone={degraded.length ? "warning" : "success"} detail="Critical, degraded, or elevated services." />
        <RuntimeStatusCard icon={Activity} label="Events" value={String(totalEvents)} tone="info" detail="Total events attached to service rows." />
        <RuntimeStatusCard icon={Gauge} label="Errors" value={String(totalErrors)} tone={totalErrors ? "warning" : "success"} detail="Current known error count across services." />
      </div>

      {rows.length === 0 ? (
        <EmptyState title="No services observed yet" description="Start collectors or ingest runtime events to populate the systems view." />
      ) : (
        <div className="space-y-4">
          <div className="grid gap-4 xl:grid-cols-3">
            {rows.slice(0, 6).map((service) => (
              <Link
                key={service.service_id}
                to={`/systems/${service.service_id}`}
                className="rounded-2xl border border-border/70 bg-card/75 p-4 text-foreground shadow-sm transition hover:-translate-y-0.5 hover:border-primary/30 hover:opacity-100"
              >
                <div className="flex items-start justify-between gap-3">
                  <RuntimeIdentity service={service.service_id} runtime="service" latency={service.last_event_at ? `last ${formatRelativeDate(service.last_event_at)}` : null} />
                  <ServiceHealthBadge status={service.status} />
                </div>
                <div className="mt-4 grid grid-cols-2 gap-3 text-sm">
                  <div className="rounded-xl border border-border/60 bg-background/35 p-3">
                    <p className="text-xs text-muted-foreground">Events</p>
                    <p className="mt-1 text-lg font-semibold">{service.event_count ?? 0}</p>
                  </div>
                  <div className="rounded-xl border border-border/60 bg-background/35 p-3">
                    <p className="text-xs text-muted-foreground">Errors</p>
                    <p className="mt-1 text-lg font-semibold">{service.error_count ?? 0}</p>
                  </div>
                </div>
                <div className="mt-4">
                  <Sparkline
                    values={[1, 2, service.event_count ?? 0, service.error_count ?? 0, Math.max(1, service.event_count ?? 0)]}
                    tone={riskTone(service.status) === "destructive" ? "critical" : riskTone(service.status) === "warning" ? "warning" : "success"}
                  />
                </div>
              </Link>
            ))}
          </div>

          <TableWrap>
            <Table>
              <thead>
                <tr>
                  <Th>Service</Th>
                  <Th>Status</Th>
                  <Th>Events</Th>
                  <Th>Errors</Th>
                  <Th>Error rate</Th>
                  <Th>Last event</Th>
                </tr>
              </thead>
              <tbody>
                {rows.map((service) => (
                  <tr key={service.service_id} className="transition hover:bg-secondary/50">
                    <Td>
                      <Link className="font-medium" to={`/systems/${service.service_id}`}>
                        {service.service_id}
                      </Link>
                    </Td>
                    <Td>
                      <ServiceHealthBadge status={service.status} />
                    </Td>
                    <Td>{service.event_count ?? 0}</Td>
                    <Td>{service.error_count ?? 0}</Td>
                    <Td>{typeof service.error_ratio === "number" ? `${Math.round(service.error_ratio * 100)}%` : "-"}</Td>
                    <Td className="text-muted-foreground">{formatRelativeDate(service.last_event_at)}</Td>
                  </tr>
                ))}
              </tbody>
            </Table>
          </TableWrap>
        </div>
      )}
    </div>
  );
}

export function ServiceDetailPage({ mode }: { mode: Mode }) {
  const { serviceId } = useParams();
  const [forceInvestigationRun, setForceInvestigationRun] = useState(0);
  const generationScope = serviceId ? `service:${serviceId}` : null;
  const savedGenerations = useApiQuery<AiGenerationsResponse>(
    generationScope ? `/api/ai/generations?scope=${encodeURIComponent(generationScope)}&limit=1` : null,
    { deps: [generationScope], staleTime: 5_000 },
  );
  const savedInvestigation = savedGenerations.data?.generations?.[0]
    ? hydrateServiceSavedGeneration(savedGenerations.data.generations[0])
    : null;
  const savedLookupDone = Boolean(savedGenerations.data || savedGenerations.error);
  const shouldRunInvestigation = Boolean(
    serviceId && (forceInvestigationRun > 0 || (savedLookupDone && !savedInvestigation)),
  );
  const detail = useApiQuery<ServiceDetailResponse>(serviceId ? `/api/services/${encodeURIComponent(serviceId)}` : null, { deps: [serviceId] });
  const investigation = useApiQuery<InvestigationResponse>(
    shouldRunInvestigation && serviceId
      ? `/api/investigate/service/${encodeURIComponent(serviceId)}?mode=${mode}${
          forceInvestigationRun ? `&force=true&run=${forceInvestigationRun}` : ""
        }`
      : null,
    { deps: [serviceId, mode, forceInvestigationRun] },
  );
  const anomaly = useApiQuery<AnomalyStatus>(serviceId ? `/api/anomaly/${encodeURIComponent(serviceId)}/status` : null, { deps: [serviceId] });
  const topology = useApiQuery<{ edges: TopologyEdge[] }>("/api/topology");
  const workspace = useApiQuery<WorkspaceMapResponse>("/api/workspace/map");

  if (!serviceId) return <EmptyState title="Missing service id" description="Select a service from the systems table." />;
  if (detail.isLoading && !detail.data) return <LoadingState title="Loading service detail" />;
  if (detail.errorMessage && !detail.data) return <ErrorState description={detail.errorMessage} onRetry={() => void detail.reload()} />;
  if (!detail.data) return <EmptyState title="No service data" description="Inferra could not load the service detail." />;
  const serviceStatus = detail.data.service.status;
  const investigationMissing = investigation.error?.status === 404;
  const displayedInvestigation = investigation.data ?? savedInvestigation;
  const topologyEdges = (topology.data?.edges ?? []).filter((edge) => edge.source === serviceId || edge.target === serviceId);
  const workspaceMappings = (workspace.data?.service_mappings ?? []).filter((mapping) => mapping.service_id === serviceId);

  return (
    <div className="space-y-6">
      <PageHeader
        title={`Service ${serviceId}`}
        subtitle={`Status ${formatDisplayValue(detail.data.service.status || "unknown")}`}
        mode={mode}
        actions={
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void detail.reload({ silent: true });
              setForceInvestigationRun((value) => value + 1);
              void anomaly.reload({ silent: true });
              void topology.reload({ silent: true });
              void workspace.reload({ silent: true });
            }}
          >
            <RefreshCcw
              className={`size-4 ${
                detail.isRefreshing || investigation.isRefreshing || anomaly.isRefreshing || topology.isRefreshing || workspace.isRefreshing
                  ? "animate-spin"
                  : ""
              }`}
            />
            Refresh
          </Button>
        }
      />

      <div className="content-grid">
        <div className="space-y-4">
          {displayedInvestigation ? (
            <InvestigationView result={displayedInvestigation} showRaw={isAdvancedMode(mode)} onRefresh={() => setForceInvestigationRun((value) => value + 1)} />
          ) : investigation.errorMessage ? (
            investigationMissing ? (
              <EmptyState
                title="Investigation not available"
                description="Inferra could not build a current investigation bundle for this service. Try refreshing after more events arrive."
                action={<Button onClick={() => void investigation.reload()}>Retry investigation</Button>}
              />
            ) : (
              <ErrorState description={`Investigation unavailable: ${investigation.errorMessage}`} onRetry={() => void investigation.reload()} />
            )
          ) : savedGenerations.errorMessage ? (
            <ErrorState description={`Saved investigation unavailable: ${savedGenerations.errorMessage}`} onRetry={() => void savedGenerations.reload()} />
          ) : savedGenerations.isLoading ? (
            <LoadingState title="Loading saved investigation" />
          ) : (
            <LoadingState title="Running investigation" />
          )}
        </div>

        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Anomaly & topology</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4 text-sm">
              {anomaly.data ? (
                <div className="rounded-2xl border border-border/60 bg-background/30 p-4">
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant={formatRiskTone(anomaly.data.status)}>{formatDisplayValue(anomaly.data.status)}</Badge>
                    <span className="text-muted-foreground">{anomaly.data.event_count} events in {anomaly.data.window_hours}h</span>
                  </div>
                  <p className="mt-2 text-muted-foreground">
                    Errors: {anomaly.data.error_count} · last event {formatRelativeDate(anomaly.data.last_event_at)}
                  </p>
                </div>
              ) : anomaly.errorMessage ? (
                <p className="text-sm text-destructive">{anomaly.errorMessage}</p>
              ) : null}

              {topologyEdges.length ? (
                <div className="space-y-2">
                  {topologyEdges.map((edge, index) => (
                    <div key={`${edge.source}-${edge.target}-${index}`} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                      <p className="font-medium">
                        {edge.source} {formatDisplayValue(edge.relation_type ?? edge.type ?? "relates_to")} {edge.target}
                      </p>
                      {edge.notes ? <p className="mt-1 text-muted-foreground">{edge.notes}</p> : null}
                    </div>
                  ))}
                </div>
              ) : (
                <p className="text-sm text-muted-foreground">No topology edges currently reference this service.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Workspace mapping</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3 text-sm">
              {workspaceMappings.length ? (
                workspaceMappings.map((mapping) => (
                  <div key={`${mapping.service_id}-${mapping.project_path}`} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                    <p className="font-medium">{mapping.project_path}</p>
                    <p className="mt-1 text-muted-foreground">
                      Confidence {(mapping.confidence * 100).toFixed(0)}% via {formatDisplayValue(mapping.source)}
                    </p>
                    {mapping.notes ? <p className="mt-2 text-muted-foreground">{mapping.notes}</p> : null}
                  </div>
                ))
              ) : (
                <p className="text-sm text-muted-foreground">No workspace mapping is attached to this service yet.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Active incidents</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {detail.data.incidents.length ? (
                detail.data.incidents.map((incident) => (
                  <div key={incident.incident_id} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                    <div className="flex items-center justify-between gap-3">
                      <Link className="font-medium" to={`/incidents/${incident.incident_id}`}>
                        {incident.incident_id}
                      </Link>
                      <Badge variant={formatRiskTone(String(incident.severity))}>Sev {formatSeverityLabel(incident.severity)}</Badge>
                    </div>
                  </div>
                ))
              ) : (
                <p className="text-sm text-muted-foreground">No active incidents.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Recent events</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {detail.data.events.length ? (
                detail.data.events.slice(0, isAdvancedMode(mode) ? 24 : 10).map((event) => (
                  <div key={event.event_id} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <Badge variant={formatRiskTone(event.severity ? String(event.severity) : serviceStatus)}>
                        {event.severity == null ? "Event" : formatSeverityLabel(event.severity)}
                      </Badge>
                      <span className="text-xs text-muted-foreground">{formatRelativeDate(event.timestamp)}</span>
                    </div>
                    <p className="mt-2 text-sm">{summarizeEvent(event)}</p>
                  </div>
                ))
              ) : (
                <p className="text-sm text-muted-foreground">No recent events for this service.</p>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}

function hydrateServiceSavedGeneration(generation: AiGeneration): InvestigationResponse {
  return {
    ...generation.response,
    cached: true,
    ai_generation: {
      generation_id: generation.generation_id,
      scope_key: generation.scope_key,
      focus: generation.focus,
      mode: generation.mode,
      question: generation.question,
      bundle_hash: generation.bundle_hash,
      used_ai: generation.used_ai,
      created_at: generation.created_at,
    },
  };
}
