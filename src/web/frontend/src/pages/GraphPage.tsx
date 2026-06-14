import { useState } from "react";
import { RefreshCcw } from "lucide-react";

import type { IncidentRow, ServiceRow, TopologyEdge, WorkspaceMapResponse } from "@/api";
import { Button } from "@/components/ui/button";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { CorrelationGraph, type GraphSelection } from "@/components/inferra/correlation-graph";
import { GraphNodeDetail } from "@/components/inferra/graph-node-detail";
import type { Mode } from "@/lib/experience";
import { formatModeLabel } from "@/lib/format";
import { useApiQuery } from "@/lib/query";
import { cn } from "@/lib/utils";

export function GraphPage({ mode }: { mode: Mode }) {
  const [selection, setSelection] = useState<GraphSelection>(null);
  const services = useApiQuery<{ services: ServiceRow[] }>("/api/services", { staleTime: 15_000 });
  const incidents = useApiQuery<{ incidents: IncidentRow[] }>("/api/incidents", { staleTime: 15_000 });
  const topology = useApiQuery<{ edges: TopologyEdge[] }>("/api/topology", { staleTime: 30_000 });
  const workspace = useApiQuery<WorkspaceMapResponse>("/api/workspace/map", { staleTime: 60_000 });
  const loading =
    (services.isLoading && !services.data) ||
    (incidents.isLoading && !incidents.data) ||
    (topology.isLoading && !topology.data);
  const error = services.errorMessage ?? incidents.errorMessage ?? topology.errorMessage;
  const serviceRows = services.data?.services ?? [];
  const incidentRows = incidents.data?.incidents ?? [];
  const edges = topology.data?.edges ?? [];
  const runtimeApps = workspace.data?.runtime_apps ?? [];
  const serviceMappings = workspace.data?.service_mappings ?? [];
  const unmappedServices = workspace.data?.unmapped_services ?? [];
  const refreshing = services.isRefreshing || incidents.isRefreshing || topology.isRefreshing || workspace.isRefreshing;

  const reloadAll = () => {
    void services.reload({ silent: true });
    void incidents.reload({ silent: true });
    void topology.reload({ silent: true });
    void workspace.reload({ silent: true });
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 items-center justify-between gap-4 border-b border-border bg-card/80 px-4 py-3 backdrop-blur-sm md:px-5">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="text-sm font-semibold tracking-tight">Correlation graph</h1>
            <span className="rounded-sm border border-border bg-panel-inset px-1.5 py-0.5 font-data text-[10px] text-muted-foreground">
              {formatModeLabel(mode)}
            </span>
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Services, workspace apps, incidents, and live topology pipes.
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={reloadAll}>
          <RefreshCcw className={cn("size-4", refreshing && "animate-spin")} />
          Refresh
        </Button>
      </header>

      {loading ? (
        <div className="flex flex-1 items-center justify-center p-6">
          <LoadingState title="Loading graph" description="Resolving services, incidents, workspace apps, and dependency edges." />
        </div>
      ) : null}

      {error && !loading ? (
        <div className="flex flex-1 items-center justify-center p-6">
          <ErrorState description={error} onRetry={reloadAll} />
        </div>
      ) : null}

      {!loading && !error ? (
        serviceRows.length || incidentRows.length || runtimeApps.length ? (
          <div className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[minmax(0,1fr)_minmax(300px,380px)]">
            <div className="flex min-h-0 min-w-0 flex-col overflow-hidden border-b border-border lg:border-b-0 lg:border-r">
              <CorrelationGraph
                className="min-h-0 flex-1"
                services={serviceRows}
                incidents={incidentRows}
                edges={edges}
                runtimeApps={runtimeApps}
                serviceMappings={serviceMappings}
                unmappedServices={unmappedServices}
                selection={selection}
                onSelect={setSelection}
              />
            </div>
            {selection ? (
              <GraphNodeDetail
                selection={selection}
                services={serviceRows}
                incidents={incidentRows}
                runtimeApps={runtimeApps}
                onClose={() => setSelection(null)}
                className="min-h-0"
              />
            ) : (
              <aside className="hidden min-h-0 flex-col justify-center border-border bg-panel-inset/40 p-6 lg:flex">
                <p className="label-caps text-[10px] text-muted-foreground">Node details</p>
                <h2 className="mt-2 text-sm font-semibold">Select a node on the graph</h2>
                <p className="mt-2 text-sm leading-6 text-muted-foreground">
                  Click a service, incident, or workspace app to inspect live metrics, traces, and correlations. The panel scrolls independently.
                </p>
              </aside>
            )}
          </div>
        ) : (
          <div className="flex flex-1 items-center justify-center p-6">
            <EmptyState title="No graph data yet" description="Start collectors or ingest events so Inferra can infer services and relationships." />
          </div>
        )
      ) : null}
    </div>
  );
}
