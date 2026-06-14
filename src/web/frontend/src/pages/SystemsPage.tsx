import { Box, Container, Database, HardDrive, RefreshCcw, Server, Waypoints } from "lucide-react";
import { useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";

import type {
  AnomalyStatus,
  AiGeneration,
  AiGenerationsResponse,
  CollectorRow,
  InvestigationResponse,
  OverviewResponse,
  ServiceDetailResponse,
  ServiceRow,
  TopologyEdge,
  WorkspaceMapResponse,
} from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Td, Th, Table, TableWrap } from "@/components/ui/table";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { InvestigationView } from "@/components/investigation/investigation-view";
import {
  ApplicationInventoryRow,
  AttentionStrip,
  ContainerInventoryRow,
  DataStoreInventoryRow,
  DeveloperServiceRegistry,
  InventorySection,
  InventorySummaryStrip,
  ObservedServiceRow,
  ServerInventoryCard,
} from "@/components/inferra/systems-runtime-inventory";
import type { Mode } from "@/lib/experience";
import { isAdvancedMode } from "@/lib/experience";
import { formatDisplayValue, formatRiskTone, formatRelativeDate, formatSeverityLabel, summarizeEvent } from "@/lib/format";
import { buildTracePath, hasValidTraceId, shortTraceId } from "@/lib/observability";
import { buildSystemsInventory, hasInventoryContent } from "@/lib/systems-inventory";
import { useInferraRuntime } from "@/lib/inferra-runtime";
import { useApiQuery } from "@/lib/query";
import { ServiceHealthBadge } from "@/components/inferra/health";
import { TraceSummaryInline } from "@/components/inferra/trace-summary";

export function SystemsPage({ mode }: { mode: Mode }) {
  const runtime = useInferraRuntime();
  const services = useApiQuery<{ services: ServiceRow[] }>("/api/services");
  const overview = useApiQuery<OverviewResponse>("/api/overview", { staleTime: 15_000 });
  const workspace = useApiQuery<WorkspaceMapResponse>("/api/workspace/map", { staleTime: 60_000 });
  const collectors = useApiQuery<{ collectors: CollectorRow[] }>("/api/collectors", { staleTime: 15_000 });

  const loading = (services.isLoading && !services.data) || (overview.isLoading && !overview.data);
  const error = services.errorMessage ?? overview.errorMessage;

  const collectorStats = useMemo(() => {
    const rows = collectors.data?.collectors ?? [];
    const configured = rows.filter((row) => row.status !== "disabled").length;
    const running = rows.filter((row) => row.is_running).length;
    return { running, configured };
  }, [collectors.data]);

  const inventory = useMemo(
    () =>
      buildSystemsInventory(
        services.data?.services ?? [],
        overview.data,
        workspace.data,
        runtime.health,
        collectorStats,
      ),
    [services.data, overview.data, workspace.data, runtime.health, collectorStats],
  );

  const refreshAll = () => {
    void services.reload({ silent: true });
    void overview.reload({ silent: true });
    void workspace.reload({ silent: true });
    void collectors.reload({ silent: true });
  };

  const refreshing = services.isRefreshing || overview.isRefreshing || workspace.isRefreshing || collectors.isRefreshing;
  const rows = services.data?.services ?? [];

  if (loading) {
    return (
      <div className="space-y-6">
        <PageHeader title="Systems" subtitle="Servers, applications, data stores, and platform runtimes." mode={mode} />
        <LoadingState title="Loading runtime inventory" />
      </div>
    );
  }

  if (error && !services.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Systems" subtitle="Servers, applications, data stores, and platform runtimes." mode={mode} />
        <ErrorState description={error} onRetry={() => void services.reload()} />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Systems"
        subtitle="Type-first runtime inventory — server, apps, data, and platform signals."
        mode={mode}
        actions={
          <Button variant="outline" size="sm" onClick={refreshAll}>
            <RefreshCcw className={`size-4 ${refreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      <InventorySummaryStrip inventory={inventory} />
      <AttentionStrip items={inventory.attention} />

      {!hasInventoryContent(inventory) ? (
        <EmptyState
          title="No runtime inventory yet"
          description="Start collectors, enable workspace scanning, or ingest app events. Inferra groups what it can observe by server, application, and data store."
          action={
            <Button asChild>
              <Link to="/control">Open Control</Link>
            </Button>
          }
        />
      ) : (
        <div className="space-y-4">
          <InventorySection
            title="Server"
            description="Host pressure, top processes, and machine-level evidence."
            icon={HardDrive}
            count={1}
            emptyLabel="No host identity available from the runtime snapshot."
          >
            <ServerInventoryCard inventory={inventory} />
          </InventorySection>

          <InventorySection
            title="Applications"
            description="Live Python, Rust, Node, and other workspace runtimes with CPU, mapping, and log errors."
            icon={Box}
            count={inventory.applications.length}
            emptyLabel="No mapped workspace apps detected. Run a workspace scan or start your dev processes."
          >
            <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
              {inventory.applications.map((entry) => (
                <ApplicationInventoryRow key={`${entry.app.name}-${entry.app.pid ?? "na"}`} entry={entry} />
              ))}
            </div>
          </InventorySection>

          <InventorySection
            title="Data stores"
            description="Postgres, MSSQL, Supabase, and other databases — from log signals or detected processes."
            icon={Database}
            count={inventory.dataStores.length}
            emptyLabel="No database processes or log-attributed data stores observed."
          >
            {inventory.dataStores.map((store) => (
              <DataStoreInventoryRow key={store.id} store={store} />
            ))}
          </InventorySection>

          <InventorySection
            title="Containers"
            description="Docker workloads observed on this host."
            icon={Container}
            count={inventory.containers.length}
            emptyLabel="No containers reported. Enable the Docker collector if this host runs containers."
          >
            {inventory.containers.map((container) => (
              <ContainerInventoryRow key={container.name} container={container} />
            ))}
          </InventorySection>

          {inventory.platformServices.length || inventory.otherServices.length ? (
            <InventorySection
              title="Platform & other observed services"
              description="Windows services, ingress, and log-attributed services not tied to a workspace app."
              icon={Server}
              count={inventory.platformServices.length + inventory.otherServices.length}
              emptyLabel="No additional services."
            >
              {[...inventory.platformServices, ...inventory.otherServices].map((service) => (
                <ObservedServiceRow key={service.service_id} service={service} />
              ))}
            </InventorySection>
          ) : null}
        </div>
      )}

      {isAdvancedMode(mode) ? (
        <div className="space-y-4">
          <DeveloperServiceRegistry services={rows} />
          {rows.length ? (
            <Card>
              <CardHeader>
                <CardTitle>Service registry table</CardTitle>
                <CardDescription>Developer view of internal service_id projections.</CardDescription>
              </CardHeader>
              <CardContent>
                <TableWrap>
                  <Table>
                    <thead>
                      <tr>
                        <Th>Service</Th>
                        <Th>Status</Th>
                        <Th>Events</Th>
                        <Th>Errors</Th>
                        <Th>Error rate</Th>
                        <Th>Latest trace</Th>
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
                          <Td>
                            <TraceSummaryInline
                              summary={service.latest_trace_summary}
                              context={{ from: "service", serviceId: service.service_id }}
                              emptyLabel="—"
                            />
                          </Td>
                          <Td className="text-muted-foreground">{formatRelativeDate(service.last_event_at)}</Td>
                        </tr>
                      ))}
                    </tbody>
                  </Table>
                </TableWrap>
              </CardContent>
            </Card>
          ) : null}
        </div>
      ) : null}
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
  const traceLinkedEventCount = detail.data.events.filter((event) => hasValidTraceId(event.trace_id)).length;

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
                <div className="rounded-md border border-border bg-panel-inset p-4">
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
                    <div key={`${edge.source}-${edge.target}-${index}`} className="rounded-md border border-border bg-panel-inset p-4">
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
                  <div key={`${mapping.service_id}-${mapping.project_path}`} className="rounded-md border border-border bg-panel-inset p-4">
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
                  <div key={incident.incident_id} className="rounded-md border border-border bg-panel-inset p-4">
                    <div className="flex items-center justify-between gap-3">
                      <Link className="font-medium" to={`/incidents/${incident.incident_id}`}>
                        {incident.incident_id}
                      </Link>
                      <Badge variant={formatRiskTone(String(incident.severity))}>Sev {formatSeverityLabel(incident.severity)}</Badge>
                    </div>
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
                <p className="text-sm text-muted-foreground">No active incidents.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Recent events</CardTitle>
              <CardDescription>
                {traceLinkedEventCount
                  ? `${traceLinkedEventCount} recent events carry a trace id and can jump directly into the correlated timeline.`
                  : "Recent service events and their normalized summaries."}
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {detail.data.events.length ? (
                detail.data.events.slice(0, isAdvancedMode(mode) ? 24 : 10).map((event) => (
                  <div key={event.event_id} className="rounded-md border border-border bg-panel-inset p-4">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge variant={formatRiskTone(event.severity ? String(event.severity) : serviceStatus)}>
                          {event.severity == null ? "Event" : formatSeverityLabel(event.severity)}
                        </Badge>
                        {event.source_ref?.source_type ? <Badge variant="outline">{formatDisplayValue(event.source_ref.source_type)}</Badge> : null}
                        {hasValidTraceId(event.trace_id) ? (
                          <Badge variant="info" className="font-mono">
                            {shortTraceId(event.trace_id)}
                          </Badge>
                        ) : null}
                      </div>
                      <span className="text-xs text-muted-foreground">{formatRelativeDate(event.timestamp)}</span>
                    </div>
                    <p className="mt-2 text-sm">{summarizeEvent(event)}</p>
                    {hasValidTraceId(event.trace_id) ? (
                      <div className="mt-3">
                        <Button variant="outline" size="sm" asChild>
                          <Link to={buildTracePath(event.trace_id ?? "", { from: "service", serviceId })}>
                            <Waypoints className="size-4" />
                            Open trace
                          </Link>
                        </Button>
                      </div>
                    ) : null}
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
