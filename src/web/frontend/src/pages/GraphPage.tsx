import { RefreshCcw } from "lucide-react";

import type { IncidentRow, ServiceRow, TopologyEdge } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { CorrelationGraph } from "@/components/inferra/correlation-graph";
import type { Mode } from "@/lib/experience";
import { useApiQuery } from "@/lib/query";

export function GraphPage({ mode }: { mode: Mode }) {
  const services = useApiQuery<{ services: ServiceRow[] }>("/api/services");
  const incidents = useApiQuery<{ incidents: IncidentRow[] }>("/api/incidents");
  const topology = useApiQuery<{ edges: TopologyEdge[] }>("/api/topology");
  const loading = (services.isLoading && !services.data) || (incidents.isLoading && !incidents.data) || (topology.isLoading && !topology.data);
  const error = services.errorMessage ?? incidents.errorMessage ?? topology.errorMessage;
  const serviceRows = services.data?.services ?? [];
  const incidentRows = incidents.data?.incidents ?? [];
  const edges = topology.data?.edges ?? [];

  return (
    <div className="space-y-6">
      <PageHeader
        title="Correlation graph"
        subtitle="A controlled relationship map for services, incidents, and inferred topology."
        mode={mode}
        actions={
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              void services.reload({ silent: true });
              void incidents.reload({ silent: true });
              void topology.reload({ silent: true });
            }}
          >
            <RefreshCcw className={`size-4 ${services.isRefreshing || incidents.isRefreshing || topology.isRefreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      {loading ? <LoadingState title="Loading graph" description="Resolving services, incidents, and dependency edges." /> : null}
      {error && !loading ? <ErrorState description={error} onRetry={() => void Promise.all([services.reload(), incidents.reload(), topology.reload()])} /> : null}

      {!loading && !error ? (
        serviceRows.length || incidentRows.length ? (
          <Card>
            <CardHeader>
              <CardTitle>Runtime correlation map</CardTitle>
              <CardDescription>
                Active paths are emphasized when incidents share affected services. Use this for orientation, then drill into Systems or Incidents.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <CorrelationGraph services={serviceRows} incidents={incidentRows} edges={edges} />
            </CardContent>
          </Card>
        ) : (
          <EmptyState title="No graph data yet" description="Start collectors or ingest events so Inferra can infer services and relationships." />
        )
      ) : null}
    </div>
  );
}
