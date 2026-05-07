import { RefreshCcw } from "lucide-react";

import type { WorkspaceMapResponse } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Td, Th, Table, TableWrap } from "@/components/ui/table";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import type { Mode } from "@/lib/experience";
import { isAdvancedMode } from "@/lib/experience";
import { useApiQuery } from "@/lib/query";

export function WorkspacePage({ mode }: { mode: Mode }) {
  const workspace = useApiQuery<WorkspaceMapResponse>("/api/workspace/map");

  if (workspace.isLoading && !workspace.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Workspace" subtitle="Detected projects and service-to-project mappings." mode={mode} />
        <LoadingState title="Scanning workspace" />
      </div>
    );
  }

  if (workspace.errorMessage && !workspace.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Workspace" subtitle="Detected projects and service-to-project mappings." mode={mode} />
        <ErrorState description={workspace.errorMessage} onRetry={() => void workspace.reload()} />
      </div>
    );
  }

  if (!workspace.data) {
    return <EmptyState title="No workspace data" description="Inferra could not load local project metadata." />;
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Workspace"
        subtitle="Detected local projects and the service-to-project mapping graph."
        mode={mode}
        actions={
          <Button variant="outline" size="sm" onClick={() => void workspace.reload({ silent: true })}>
            <RefreshCcw className={`size-4 ${workspace.isRefreshing ? "animate-spin" : ""}`} />
            Re-scan
          </Button>
        }
      />

      <div className="dashboard-grid">
        <SummaryCard label="Projects" value={String(workspace.data.projects.length)} />
        <SummaryCard label="Mappings" value={String(workspace.data.service_mappings.length)} />
        <SummaryCard label="Unmapped services" value={String(workspace.data.unmapped_services.length)} />
      </div>

      {workspace.data.service_mappings.length ? (
        <TableWrap>
          <Table>
            <thead>
              <tr>
                <Th>Service</Th>
                <Th>Project path</Th>
                <Th>Confidence</Th>
                <Th>Source</Th>
                {isAdvancedMode(mode) ? <Th>Signals</Th> : null}
              </tr>
            </thead>
            <tbody>
              {workspace.data.service_mappings.map((mapping, index) => (
                <tr key={`${mapping.service_id}-${index}`} className="transition hover:bg-secondary/50">
                  <Td>{mapping.service_id}</Td>
                  <Td className="font-mono text-xs text-muted-foreground">{mapping.project_path}</Td>
                  <Td>{mapping.confidence.toFixed(2)}</Td>
                  <Td>{mapping.source}</Td>
                  {isAdvancedMode(mode) ? (
                    <Td>
                      <div className="flex flex-wrap gap-2">
                        {mapping.signals.map((signal) => (
                          <Badge key={`${signal.name}-${signal.detail}`} variant="outline">
                            {signal.name}
                          </Badge>
                        ))}
                      </div>
                    </Td>
                  ) : null}
                </tr>
              ))}
            </tbody>
          </Table>
        </TableWrap>
      ) : (
        <EmptyState
          title="No mappings inferred"
          description="Add explicit mappings under [[workspace.service_mappings]] in inferra.toml or let Inferra observe more runtime signals."
        />
      )}

      <Card>
        <CardHeader>
          <CardTitle>Detected projects</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {workspace.data.projects.map((project) => (
            <div key={project.path} className="rounded-2xl border border-border/60 bg-background/30 p-4">
              <p className="font-medium">{project.kind}</p>
              <p className="mt-2 break-all font-mono text-xs text-muted-foreground">{project.path}</p>
              <Badge className="mt-3 w-fit" variant="outline">
                {project.marker}
              </Badge>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}

function SummaryCard({ label, value }: { label: string; value: string }) {
  return (
    <Card className="border-border/70 bg-background/30">
      <CardContent className="p-5">
        <p className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">{label}</p>
        <p className="mt-2 text-3xl font-semibold">{value}</p>
      </CardContent>
    </Card>
  );
}

