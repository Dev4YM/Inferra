import { BrainCircuit, RefreshCcw } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { toast } from "sonner";

import type { EventRow, InvestigationResponse, ScannerStatusResponse, WorkspaceMapResponse, WorkspaceRuntimeApp } from "@/api";
import { errorMessage, postJson } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Td, Th, Table, TableWrap } from "@/components/ui/table";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { InvestigationView } from "@/components/investigation/investigation-view";
import { Input } from "@/components/ui/input";
import type { Mode } from "@/lib/experience";
import { isAdvancedMode } from "@/lib/experience";
import { formatDisplayValue, formatSeverityLabel } from "@/lib/format";
import { useApiMutation, useApiQuery } from "@/lib/query";

export function WorkspacePage({ mode }: { mode: Mode }) {
  const workspace = useApiQuery<WorkspaceMapResponse>("/api/workspace/map", { staleTime: 60_000 });
  const scanner = useApiQuery<ScannerStatusResponse>("/api/scanner/status", { staleTime: 15_000 });
  const scannerRun = useApiMutation(async () => postJson("/api/scanner/run", {}));
  const runtimeApps = workspace.data?.runtime_apps ?? [];
  const workspaceScanner = scanner.data?.scanner.workspace;

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
          <div className="flex flex-wrap gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={async () => {
                try {
                  await scannerRun.run(undefined);
                  toast.success("Workspace scan refreshed");
                  void scanner.reload({ silent: true });
                  void workspace.reload({ silent: true });
                } catch (error) {
                  toast.error("Workspace scan failed", { description: errorMessage(error) });
                }
              }}
              disabled={scannerRun.isPending}
            >
              <RefreshCcw className={`size-4 ${scannerRun.isPending ? "animate-spin" : ""}`} />
              Force scan
            </Button>
            <Button variant="outline" size="sm" onClick={() => void workspace.reload({ silent: true })}>
              <RefreshCcw className={`size-4 ${workspace.isRefreshing ? "animate-spin" : ""}`} />
              Refresh cache
            </Button>
          </div>
        }
      />

      {workspaceScanner ? (
        <Card className="border-border/70 bg-background/35">
          <CardContent className="flex flex-wrap items-center justify-between gap-3 p-4 text-sm">
            <div>
              <p className="font-medium">Workspace scanner cache</p>
              <p className="text-muted-foreground">
                Interval {workspaceScanner.interval_seconds ?? 120}s, age {workspaceScanner.age_seconds ?? 0}s, next scan in{" "}
                {workspaceScanner.next_scan_in_seconds ?? 0}s.
              </p>
            </div>
            <Badge variant={workspaceScanner.cached ? "success" : "warning"}>
              {workspaceScanner.cached ? "Cached" : "Cold"}
            </Badge>
          </CardContent>
        </Card>
      ) : null}

      <div className="dashboard-grid">
        <SummaryCard label="Projects" value={String(workspace.data.projects.length)} />
        <SummaryCard label="Running apps" value={String(workspace.data.runtime_apps?.length ?? 0)} />
        <SummaryCard label="Mappings" value={String(workspace.data.service_mappings.length)} />
        <SummaryCard label="Unmapped services" value={String(workspace.data.unmapped_services.length)} />
      </div>

      {workspace.data.support_layers?.length ? (
        <Card>
          <CardHeader>
            <CardTitle>Supported detection layers</CardTitle>
          </CardHeader>
          <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            {workspace.data.support_layers.map((layer) => (
              <div key={layer.layer} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                <p className="font-medium">{layer.title}</p>
                <p className="mt-1 text-xs text-muted-foreground">{layer.items.length} supported</p>
                <div className="mt-3 flex flex-wrap gap-2">
                  {layer.items.slice(0, 8).map((item) => (
                    <Badge key={item.id} variant="outline">
                      {item.label}
                      {item.children?.length ? ` +${item.children.length}` : ""}
                    </Badge>
                  ))}
                  {layer.items.length > 8 ? <Badge variant="outline">+{layer.items.length - 8}</Badge> : null}
                </div>
              </div>
            ))}
          </CardContent>
        </Card>
      ) : null}

      {runtimeApps.length ? (
        <TableWrap>
          <Table>
            <thead>
              <tr>
                <Th>App</Th>
                <Th>Runtime</Th>
                <Th>Manager</Th>
                <Th>Project</Th>
                <Th>Confidence</Th>
                <Th>Details</Th>
                {isAdvancedMode(mode) ? <Th>Signals</Th> : null}
              </tr>
            </thead>
            <tbody>
              {runtimeApps.map((app, index) => (
                <tr key={`${app.name}-${app.pid ?? index}`} className="transition hover:bg-secondary/50">
                  <Td>
                    <div className="min-w-0">
                      <p className="font-medium">{app.name}</p>
                      {app.framework ? <p className="text-xs text-muted-foreground">{formatDisplayValue(app.framework)}</p> : null}
                    </div>
                  </Td>
                  <Td>{formatDisplayValue(app.language ?? app.runtime)}</Td>
                  <Td>{formatDisplayValue(app.manager ?? app.source)}</Td>
                  <Td className="font-mono text-xs text-muted-foreground">{app.project_path ?? app.cwd ?? "-"}</Td>
                  <Td>{app.confidence.toFixed(2)}</Td>
                  <Td>
                    <Button variant="outline" size="sm" asChild>
                      <Link to={`/workspace/apps?name=${encodeURIComponent(app.name)}`}>View</Link>
                    </Button>
                  </Td>
                  {isAdvancedMode(mode) ? (
                    <Td>
                      <div className="flex flex-wrap gap-2">
                        {app.signals.map((signal) => (
                          <Badge key={`${app.name}-${signal.name}-${signal.detail}`} variant="outline">
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
      ) : null}

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
                  <Td>{formatDisplayValue(mapping.source)}</Td>
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

export function WorkspaceAppPage({ mode }: { mode: Mode }) {
  const [params] = useSearchParams();
  const appName = params.get("name") ?? "";
  const workspace = useApiQuery<WorkspaceMapResponse>("/api/workspace/map", { deps: [appName] });
  const app = useMemo(
    () => (workspace.data?.runtime_apps ?? []).find((item) => item.name === appName) ?? null,
    [appName, workspace.data],
  );
  const appLogsPath = app ? `/api/logs?service=${encodeURIComponent(app.name)}&limit=50` : null;
  const appLogs = useApiQuery<{ logs: EventRow[] }>(appLogsPath, { deps: [app?.name] });

  if (workspace.isLoading && !workspace.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Workspace app" subtitle="Loading app details, logs, and monitor state." mode={mode} />
        <LoadingState title="Loading workspace app" />
      </div>
    );
  }

  if (workspace.errorMessage && !workspace.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Workspace app" subtitle="Loading app details, logs, and monitor state." mode={mode} />
        <ErrorState description={workspace.errorMessage} onRetry={() => void workspace.reload()} />
      </div>
    );
  }

  if (!app) {
    return (
      <div className="space-y-6">
        <PageHeader title="Workspace app" subtitle="App details, logs, and AI monitoring." mode={mode} />
        <EmptyState
          title="App not found"
          description="Re-scan the workspace or open an app from the Workspace running apps table."
        />
        <Button variant="outline" asChild>
          <Link to="/workspace">Back to workspace</Link>
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title={app.name}
        subtitle="Workspace app details, recent logs, and read-only AI monitoring."
        mode={mode}
        actions={
          <Button variant="outline" size="sm" asChild>
            <Link to="/workspace">Back</Link>
          </Button>
        }
      />
      <WorkspaceAppDetails
        app={app}
        logs={appLogs.data?.logs ?? []}
        logsLoading={appLogs.isLoading}
        logsError={appLogs.errorMessage}
        onRefreshLogs={() => void appLogs.reload({ silent: true })}
        mode={mode}
      />
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

function WorkspaceAppDetails({
  app,
  logs,
  logsLoading,
  logsError,
  onRefreshLogs,
  mode,
}: {
  app: WorkspaceRuntimeApp;
  logs: EventRow[];
  logsLoading: boolean;
  logsError: string | null;
  onRefreshLogs: () => void;
  mode: Mode;
}) {
  const [monitorSeconds, setMonitorSeconds] = useState(20);
  const [aiResult, setAiResult] = useState<InvestigationResponse | null>(null);
  const aiMonitor = useApiMutation(
    async (payload: { question: string; scope: string; mode: Mode; monitor_seconds: number }) =>
      postJson<InvestigationResponse>("/api/ai/ask", payload),
  );
  const runAiMonitor = useCallback(async () => {
    try {
      const next = await aiMonitor.run({
        question:
          "Monitor this workspace app using recent stored logs and live runtime signals. Summarize health, anomalies, likely causes, missing evidence, and safe read-only next checks.",
        scope: `workspace_app:${app.name}`,
        mode,
        monitor_seconds: monitorSeconds,
      });
      setAiResult(next);
      if (!next.used_ai) {
        toast.message("Deterministic fallback used.", { description: next.fallback_reason || "AI was unavailable." });
      } else {
        toast.success("AI monitor completed");
      }
    } catch (error) {
      toast.error("AI monitor failed", { description: errorMessage(error) });
    }
  }, [aiMonitor, app.name, mode, monitorSeconds]);

  const detailRows = [
    ["Name", app.name],
    ["Language", formatDisplayValue(app.language ?? app.runtime)],
    ["Process kind", app.process_kind ? formatDisplayValue(app.process_kind) : "-"],
    ["Framework", app.framework ? formatDisplayValue(app.framework) : "-"],
    ["Manager", formatDisplayValue(app.manager ?? app.source)],
    ["PID", app.pid ? String(app.pid) : "-"],
    ["Project", app.project_path ?? "-"],
    ["CWD", app.cwd ?? "-"],
    ["Script", app.script ?? "-"],
    ["Command", app.command ?? "-"],
  ];

  return (
    <div className="space-y-4">
      <div className="grid gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
      <Card>
        <CardHeader>
          <CardTitle>{app.name}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-2">
            {detailRows.map(([label, value]) => (
              <div key={label} className="grid gap-1 border-b border-border/50 pb-2 last:border-b-0">
                <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">{label}</p>
                <p className="break-all font-mono text-xs">{value}</p>
              </div>
            ))}
          </div>
          {app.libraries?.length ? (
            <div className="flex flex-wrap gap-2">
              {app.libraries.map((library) => (
                <Badge key={library} variant="outline">
                  {library}
                </Badge>
              ))}
            </div>
          ) : null}
          {app.log_hints?.length ? (
            <div className="space-y-2">
              <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Log hints</p>
              <div className="flex flex-wrap gap-2">
                {app.log_hints.map((hint) => (
                  <Badge key={hint} variant="outline">
                    {hint}
                  </Badge>
                ))}
              </div>
            </div>
          ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row items-center justify-between gap-3">
          <CardTitle>Recent logs</CardTitle>
          <Button variant="outline" size="sm" onClick={onRefreshLogs}>
            <RefreshCcw className={`size-4 ${logsLoading ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        </CardHeader>
        <CardContent>
          {logsError ? <ErrorState description={logsError} onRetry={onRefreshLogs} /> : null}
          {!logsError && logsLoading ? <LoadingState title="Loading app logs" /> : null}
          {!logsError && !logsLoading && logs.length ? (
            <div className="space-y-3">
              {logs.slice(0, 8).map((event, index) => (
                <div key={event.event_id ?? index} className="rounded-lg border border-border/60 bg-background/30 p-3">
                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <span>{event.timestamp ?? "-"}</span>
                    <Badge variant="outline">Severity {event.severity == null ? "-" : formatSeverityLabel(event.severity)}</Badge>
                    {event.source_ref?.source_type ? <Badge variant="outline">{formatDisplayValue(event.source_ref.source_type)}</Badge> : null}
                  </div>
                  <p className="mt-2 break-words text-sm">{event.message ?? event.summary ?? "-"}</p>
                </div>
              ))}
            </div>
          ) : null}
          {!logsError && !logsLoading && !logs.length ? (
            <EmptyState
              title="No stored logs for this app"
              description="Start collectors or register an app-specific log source to populate this panel."
            />
          ) : null}
        </CardContent>
      </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>AI log monitor</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap items-center gap-3">
            <label className="flex items-center gap-2 text-sm text-muted-foreground">
              <span className="whitespace-nowrap">Monitor seconds</span>
              <Input
                aria-label="Workspace app monitor seconds"
                className="h-10 w-24"
                type="number"
                min={0}
                max={180}
                value={monitorSeconds}
                onChange={(event) => setMonitorSeconds(Math.min(180, Math.max(0, Number(event.target.value) || 0)))}
              />
            </label>
            <Button onClick={() => void runAiMonitor()} disabled={aiMonitor.isPending}>
              <BrainCircuit className="size-4" />
              {aiMonitor.isPending ? "Monitoring..." : "Run AI monitor"}
            </Button>
          </div>
          {aiMonitor.errorMessage ? <ErrorState description={aiMonitor.errorMessage} onRetry={() => void runAiMonitor()} /> : null}
        </CardContent>
      </Card>
      {aiResult ? <InvestigationView result={aiResult} showRaw={isAdvancedMode(mode)} onRefresh={() => void runAiMonitor()} /> : null}
    </div>
  );
}
