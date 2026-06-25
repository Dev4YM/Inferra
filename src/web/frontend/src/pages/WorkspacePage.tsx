import { BrainCircuit, RefreshCcw, Waypoints } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { toast } from "sonner";

import type {
  AiGeneration,
  AiGenerationsResponse,
  EventRow,
  InvestigationResponse,
  ScannerStatusResponse,
  WorkspaceAppLogsResponse,
  WorkspaceAppResources,
  WorkspaceAppResourcesResponse,
  WorkspaceMapResponse,
  WorkspaceRuntimeApp,
  WorkspaceSetupAction,
} from "@/api";
import { errorMessage, postJson } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { TraceSummaryInline } from "@/components/inferra/trace-summary";
import { Td, Th, Table, TableWrap } from "@/components/ui/table";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { InvestigationView } from "@/components/investigation/investigation-view";
import { Input } from "@/components/ui/input";
import type { Mode } from "@/lib/experience";
import { isAdvancedMode } from "@/lib/experience";
import { formatDisplayValue, formatSeverityLabel } from "@/lib/format";
import { buildTracePath, hasValidTraceId, shortTraceId } from "@/lib/observability";
import { useQueryClient } from "@tanstack/react-query";
import { useApiMutation, useApiQuery } from "@/lib/query";

type WorkspaceAppDetailResponse = { app: WorkspaceRuntimeApp };

function findWorkspaceAppInMap(
  map: WorkspaceMapResponse | null | undefined,
  appName: string,
): WorkspaceRuntimeApp | null {
  const needle = appName.trim().toLowerCase();
  if (!needle) return null;
  return (
    (map?.runtime_apps ?? []).find(
      (item) =>
        item.name.toLowerCase() === needle ||
        item.display_name?.trim().toLowerCase() === needle,
    ) ?? null
  );
}

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
        <Card className="border-border bg-panel-inset">
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

      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-center gap-3">
            <CardTitle>Workspace setup</CardTitle>
            <Badge variant={setupBadgeVariant(workspace.data.setup.status)}>
              {formatDisplayValue(workspace.data.setup.status)}
            </Badge>
            <Badge variant="outline">Score {workspace.data.setup.score}</Badge>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          <div>
            <p className="font-medium">{workspace.data.setup.headline}</p>
            <p className="mt-1 text-sm text-muted-foreground">{workspace.data.setup.summary}</p>
          </div>
          {workspace.data.setup.issues?.length ? (
            <div className="grid gap-3 md:grid-cols-2">
              {workspace.data.setup.issues.map((issue) => (
                <div key={issue.id} className="rounded-md border border-border bg-panel-inset p-4">
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant={setupBadgeVariant(issue.severity)}>{formatDisplayValue(issue.severity)}</Badge>
                    <p className="font-medium">{issue.title}</p>
                  </div>
                  <p className="mt-2 text-sm text-muted-foreground">{issue.detail}</p>
                </div>
              ))}
            </div>
          ) : null}
          {workspace.data.setup.recommended_roots?.length ? (
            <div className="space-y-3">
              <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">
                Recommended roots
              </p>
              <div className="grid gap-3 xl:grid-cols-2">
                {workspace.data.setup.recommended_roots.map((candidate) => (
                  <div key={`${candidate.path}-${candidate.config_key}`} className="rounded-md border border-border bg-panel-inset p-4">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge variant="outline">{candidate.config_key}</Badge>
                      <Badge variant="outline">{candidate.project_count} project(s)</Badge>
                      <Badge variant="outline">{formatDisplayValue(candidate.source)}</Badge>
                    </div>
                    <p className="mt-2 break-all font-mono text-xs">{candidate.path}</p>
                    <p className="mt-2 break-all font-mono text-xs text-muted-foreground">
                      {candidate.config_key} += "{candidate.config_value}"
                    </p>
                    {candidate.sample_projects?.length ? (
                      <div className="mt-3 flex flex-wrap gap-2">
                        {candidate.sample_projects.map((project) => (
                          <Badge key={project} variant="outline">
                            {project}
                          </Badge>
                        ))}
                      </div>
                    ) : null}
                  </div>
                ))}
              </div>
            </div>
          ) : null}
          {workspace.data.setup.actions?.length ? (
            <div className="space-y-3">
              <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Next actions</p>
              {workspace.data.setup.actions.map((action) => (
                <SetupActionCard key={action.id} action={action} />
              ))}
            </div>
          ) : null}
        </CardContent>
      </Card>

      {!workspace.data.projects.length ? (
        <Card className="border-dashed">
          <CardHeader>
            <CardTitle>No workspace projects detected</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm text-muted-foreground">
            <p>Inferra maps services to projects so incidents and graphs have an owner. Try these steps:</p>
            <ol className="list-decimal space-y-2 pl-5">
              <li>Point <code className="font-data">workspace.roots</code> or <code className="font-data">workspace.home_roots</code> at your code folders in inferra.toml.</li>
              <li>Run <strong>Force scan</strong> above or start your dev apps so runtime detection can see them.</li>
              <li>Add manual mappings from Systems or map noisy unmapped services first.</li>
              <li>Confirm collectors are running from Control so service ids appear in evidence.</li>
            </ol>
            <Button asChild variant="outline" size="sm">
              <Link to="/control">Open Control</Link>
            </Button>
          </CardContent>
        </Card>
      ) : null}

      {workspace.data.support_layers?.length ? (
        <Card>
          <CardHeader>
            <CardTitle>Supported detection layers</CardTitle>
          </CardHeader>
          <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            {workspace.data.support_layers.map((layer) => (
              <div key={layer.layer} className="rounded-md border border-border bg-panel-inset p-4">
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
                <Th>State</Th>
                <Th>URL</Th>
                <Th>Host CPU / Memory</Th>
                <Th>Project</Th>
                <Th>Confidence</Th>
                <Th>Latest trace</Th>
                <Th>Details</Th>
                {isAdvancedMode(mode) ? <Th>Signals</Th> : null}
              </tr>
            </thead>
            <tbody>
              {runtimeApps.map((app, index) => (
                <tr key={`${app.name}-${app.pid ?? index}`} className="transition hover:bg-secondary/50">
                  <Td>
                    <div className="min-w-0">
                      <p className="font-medium">{app.display_name ?? app.name}</p>
                      {app.display_name && app.display_name !== app.name ? <p className="text-xs text-muted-foreground">{app.name}</p> : null}
                      {app.framework ? <p className="text-xs text-muted-foreground">{formatDisplayValue(app.framework)}</p> : null}
                    </div>
                  </Td>
                  <Td>{formatDisplayValue(app.language ?? app.runtime)}</Td>
                  <Td>{formatDisplayValue(app.manager ?? app.source)}</Td>
                  <Td>{app.app_state?.health ? formatDisplayValue(app.app_state.health) : formatDisplayValue(app.status ?? "Observed")}</Td>
                  <Td>
                    {app.app_url ? (
                      <a className="font-mono text-xs" href={app.app_url} target="_blank" rel="noreferrer">
                        {app.app_url}
                      </a>
                    ) : (
                      "-"
                    )}
                  </Td>
                  <Td className="text-xs text-muted-foreground">
                    {app.resources?.cpu_percent != null || app.resources?.memory_mb != null
                      ? `${formatHostCpuPercent(app.resources)} / ${app.resources?.memory_mb ?? "-"} MB`
                      : "-"}
                  </Td>
                  <Td className="font-mono text-xs text-muted-foreground">{app.project_path ?? app.cwd ?? "-"}</Td>
                  <Td>{app.confidence.toFixed(2)}</Td>
                  <Td>
                    <TraceSummaryInline
                      summary={app.latest_trace_summary}
                      context={{ from: "workspace", appName: app.name }}
                      emptyLabel="—"
                      openLabel="View"
                    />
                  </Td>
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
            <div key={project.path} className="rounded-md border border-border bg-panel-inset p-4">
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
  const appName = params.get("name")?.trim() ?? "";
  const queryClient = useQueryClient();
  const appPath = appName ? `/api/workspace/apps/${encodeURIComponent(appName)}` : null;
  const cachedApp = useMemo(
    () =>
      findWorkspaceAppInMap(
        queryClient.getQueryData<WorkspaceMapResponse>(["api", "/api/workspace/map"]),
        appName,
      ),
    [appName, queryClient],
  );
  const appQuery = useApiQuery<WorkspaceAppDetailResponse>(appPath, {
    deps: [appName],
    staleTime: 30_000,
    initialData: cachedApp ? { app: cachedApp } : undefined,
  });
  const app = appQuery.data?.app ?? cachedApp;
  const appLogsPath = app ? `/api/workspace/apps/${encodeURIComponent(app.name)}/logs?limit=80` : null;
  const appLogs = useApiQuery<WorkspaceAppLogsResponse>(appLogsPath, { deps: [app?.name], staleTime: 5_000 });

  if (!app && appQuery.isLoading) {
    return (
      <div className="space-y-6">
        <PageHeader title="Workspace app" subtitle="Loading app details, logs, and monitor state." mode={mode} />
        <LoadingState title="Loading workspace app" description="Reading cached app metadata…" />
      </div>
    );
  }

  if (!app && appQuery.errorMessage) {
    return (
      <div className="space-y-6">
        <PageHeader title="Workspace app" subtitle="Loading app details, logs, and monitor state." mode={mode} />
        <ErrorState description={appQuery.errorMessage} onRetry={() => void appQuery.reload()} />
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
        logs={appLogs.data?.events ?? []}
        rawLogs={appLogs.data?.raw_logs ?? []}
        logsSampledAt={appLogs.data?.sampled_at ?? null}
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
    <Card className="border-border bg-panel-inset">
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
  rawLogs,
  logsSampledAt,
  logsLoading,
  logsError,
  onRefreshLogs,
  mode,
}: {
  app: WorkspaceRuntimeApp;
  logs: EventRow[];
  rawLogs: WorkspaceAppLogsResponse["raw_logs"];
  logsSampledAt: string | null;
  logsLoading: boolean;
  logsError: string | null;
  onRefreshLogs: () => void;
  mode: Mode;
}) {
  const [monitorSeconds, setMonitorSeconds] = useState(20);
  const [aiResult, setAiResult] = useState<InvestigationResponse | null>(null);
  const aiScope = `workspace_app:${app.name}`;
  const savedGenerations = useApiQuery<AiGenerationsResponse>(
    `/api/ai/generations?scope=${encodeURIComponent(aiScope)}&limit=6`,
    { deps: [aiScope], staleTime: 5_000 },
  );
  const liveResources = useApiQuery<WorkspaceAppResourcesResponse>(
    `/api/workspace/apps/${encodeURIComponent(app.name)}/resources${app.pid ? `?pid=${app.pid}` : ""}`,
    {
      deps: [app.name, app.pid],
      refetchInterval: 5_000,
      refetchIntervalInBackground: false,
      staleTime: 2_000,
    },
  );
  const resources = liveResources.data?.resources ?? app.resources ?? null;
  const aiMonitor = useApiMutation(
    async (payload: { question: string; scope: string; mode: Mode; monitor_seconds: number }) =>
      postJson<InvestigationResponse>("/api/ai/ask", payload),
  );
  const runAiMonitor = useCallback(async () => {
    try {
      const next = await aiMonitor.run({
        question:
          "Monitor this workspace app using recent stored logs and live runtime signals. Summarize health, anomalies, likely causes, missing evidence, and safe read-only next checks.",
        scope: aiScope,
        mode,
        monitor_seconds: monitorSeconds,
      });
      setAiResult(next);
      void savedGenerations.reload({ silent: true });
      if (!next.used_ai) {
        toast.message("Deterministic fallback used.", { description: next.fallback_reason || "AI was unavailable." });
      } else {
        toast.success("AI monitor completed");
      }
    } catch (error) {
      toast.error("AI monitor failed", { description: errorMessage(error) });
    }
  }, [aiMonitor, aiScope, mode, monitorSeconds, savedGenerations]);

  useEffect(() => {
    setAiResult(null);
  }, [aiScope]);

  useEffect(() => {
    if (aiResult) return;
    const saved = savedGenerations.data?.generations?.[0];
    if (saved?.response) {
      setAiResult(hydrateWorkspaceSavedGeneration(saved));
    }
  }, [aiResult, savedGenerations.data]);

  const detailRows = [
    ["Name", app.name],
    ["Display name", app.display_name ?? app.name],
    ["Language", formatDisplayValue(app.language ?? app.runtime)],
    ["Process kind", app.process_kind ? formatDisplayValue(app.process_kind) : "-"],
    ["Framework", app.framework ? formatDisplayValue(app.framework) : "-"],
    ["Manager", formatDisplayValue(app.manager ?? app.source)],
    ["PID", app.pid ? String(app.pid) : "-"],
    ["State", app.app_state?.health ? formatDisplayValue(app.app_state.health) : formatDisplayValue(app.status ?? "Observed")],
    ["App URL", app.app_url ?? "-"],
    ["Heartbeat", app.health_endpoint?.url ?? "-"],
    ["Project", app.project_path ?? "-"],
    ["CWD", app.cwd ?? "-"],
    ["Script", app.script ?? "-"],
    ["Executable", app.app_location?.executable ?? "-"],
    ["Command", app.command ?? "-"],
  ];

  return (
    <div className="space-y-4">
      <div className="grid gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
      <Card>
        <CardHeader>
          <CardTitle>{app.display_name ?? app.name}</CardTitle>
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
          {app.context_capabilities?.length ? (
            <div className="space-y-2">
              <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">AI context coverage</p>
              <div className="flex flex-wrap gap-2">
                {app.context_capabilities.map((capability) => (
                  <Badge key={capability.key} variant={capability.supported ? "success" : "outline"}>
                    {formatDisplayValue(capability.key)}
                  </Badge>
                ))}
              </div>
            </div>
          ) : null}
          <div className="space-y-2">
            <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Latest trace</p>
            <TraceSummaryInline
              summary={app.latest_trace_summary}
              context={{ from: "workspace", appName: app.name }}
              emptyLabel="No correlated trace found yet"
              openLabel="Open trace"
              showMessage
            />
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="flex-row items-center justify-between gap-3">
          <div>
            <CardTitle>Extracted logs</CardTitle>
            {logsSampledAt ? <p className="mt-1 text-xs text-muted-foreground">Sampled {logsSampledAt}</p> : null}
          </div>
          <Button variant="outline" size="sm" onClick={onRefreshLogs}>
            <RefreshCcw className={`size-4 ${logsLoading ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        </CardHeader>
        <CardContent>
          {logsError ? <ErrorState description={logsError} onRetry={onRefreshLogs} /> : null}
          {!logsError && logsLoading ? <LoadingState title="Loading app logs" /> : null}
          {!logsError && !logsLoading && rawLogs.length ? (
            <div className="space-y-2">
              {rawLogs.slice(0, 16).map((entry, index) => (
                <div key={`${entry.source?.path ?? "raw"}-${entry.line_number_from_tail ?? index}`} className="rounded-lg border border-border bg-panel-inset p-3">
                  <div className="mb-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <Badge variant="outline">{formatDisplayValue(entry.source?.label ?? "File")}</Badge>
                    {entry.source?.path || entry.source?.command || entry.source?.stream ? (
                      <span className="break-all font-mono">{entry.source.path ?? entry.source.command ?? entry.source.stream}</span>
                    ) : null}
                  </div>
                  <p className="whitespace-pre-wrap break-words font-mono text-xs leading-relaxed">{entry.line || "-"}</p>
                </div>
              ))}
            </div>
          ) : null}
          {!logsError && !logsLoading && logs.length ? (
            <div className="mt-4 space-y-3">
              <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Stored normalized events</p>
              {logs.slice(0, 8).map((event, index) => (
                <div key={event.event_id ?? index} className="rounded-lg border border-border bg-panel-inset p-3">
                  <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                    <span>{event.timestamp ?? "-"}</span>
                    <Badge variant="outline">Severity {event.severity == null ? "-" : formatSeverityLabel(event.severity)}</Badge>
                    {event.source_ref?.source_type ? <Badge variant="outline">{formatDisplayValue(event.source_ref.source_type)}</Badge> : null}
                    {hasValidTraceId(event.trace_id) ? (
                      <Badge variant="info" className="font-mono">
                        {shortTraceId(event.trace_id)}
                      </Badge>
                    ) : null}
                  </div>
                  <p className="mt-2 break-words text-sm">{event.message ?? event.summary ?? "-"}</p>
                  {hasValidTraceId(event.trace_id) ? (
                    <div className="mt-3">
                      <Button variant="outline" size="sm" asChild>
                        <Link to={buildTracePath(event.trace_id ?? "", { from: "workspace", appName: app.name })}>
                          <Waypoints className="size-4" />
                          Open trace
                        </Link>
                      </Button>
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
          {!logsError && !logsLoading && !rawLogs.length && !logs.length ? (
            <EmptyState
              title="No logs extracted for this app"
              description="Register file log paths under .inferra/app.toml or start collectors so Inferra can attach app-specific evidence."
            />
          ) : null}
        </CardContent>
      </Card>
      </div>

      <div className="grid gap-4 xl:grid-cols-3">
      <Card>
        <CardHeader>
          <div className="flex flex-wrap items-center gap-3">
            <CardTitle>Setup readiness</CardTitle>
            <Badge variant={setupBadgeVariant(app.setup?.status ?? "partial")}>
              {formatDisplayValue(app.setup?.status ?? "partial")}
            </Badge>
            {app.setup ? <Badge variant="outline">Score {app.setup.score}</Badge> : null}
          </div>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
            <p className="text-sm text-muted-foreground">{app.setup?.summary ?? "Setup guidance is unavailable for this app."}</p>
            {app.setup?.issues?.length ? (
              <div className="space-y-3">
                {app.setup.issues.map((issue) => (
                  <div key={issue.id} className="rounded-md border border-border bg-panel-inset p-3">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge variant={setupBadgeVariant(issue.severity)}>{formatDisplayValue(issue.severity)}</Badge>
                      <p className="font-medium">{issue.title}</p>
                    </div>
                    <p className="mt-2 text-xs text-muted-foreground">{issue.detail}</p>
                  </div>
                ))}
              </div>
            ) : null}
            {app.setup?.actions?.length ? (
              <div className="space-y-3">
                {app.setup.actions.map((action) => (
                  <SetupActionCard key={action.id} action={action} />
                ))}
              </div>
            ) : null}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>App state</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
            <InfoLine label="Health" value={app.app_state?.health ? formatDisplayValue(app.app_state.health) : "-"} />
            <InfoLine label="Status" value={app.app_state?.status ? formatDisplayValue(app.app_state.status) : app.status ?? "-"} />
            <InfoLine label="Observed by" value={formatDisplayValue(app.app_state?.observed_by ?? app.manager ?? app.source)} />
            <InfoLine label="Restarts" value={app.app_state?.restarts != null ? String(app.app_state.restarts) : "-"} />
            {app.app_state?.reason ? <p className="text-xs text-muted-foreground">{app.app_state.reason}</p> : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
          <CardTitle>Resources</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
            <div className="mb-2 flex items-center gap-2">
              <Badge variant={liveResources.data?.live ? "success" : "outline"}>
                {liveResources.data?.live ? "Live" : "Snapshot"}
              </Badge>
              {liveResources.data?.sampled_at ? <span className="text-xs text-muted-foreground">{liveResources.data.sampled_at}</span> : null}
            </div>
            <InfoLine label="Host CPU share" value={formatHostCpuPercent(resources)} />
            <InfoLine label="Raw process CPU" value={formatRawCpuPercent(resources)} />
            <InfoLine label="CPU scope" value={resources?.cpu_percent_scope ? formatDisplayValue(resources.cpu_percent_scope) : "-"} />
            <InfoLine label="Memory" value={resources?.memory_mb != null ? `${resources.memory_mb} MB` : "-"} />
            <InfoLine label="Virtual memory" value={resources?.virtual_memory_mb != null ? `${resources.virtual_memory_mb} MB` : "-"} />
            <InfoLine label="Uptime" value={resources?.uptime_seconds != null ? `${resources.uptime_seconds}s` : "-"} />
            <InfoLine label="Process status" value={resources?.process_status ? formatDisplayValue(resources.process_status) : "-"} />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Endpoints</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {app.endpoints?.length ? (
              app.endpoints.map((endpoint) => (
                <div key={`${endpoint.url}-${endpoint.source}`} className="rounded-xl border border-border bg-panel-inset p-3">
                  <a className="break-all font-mono text-xs" href={endpoint.url} target="_blank" rel="noreferrer">
                    {endpoint.url}
                  </a>
                  <div className="mt-2 flex flex-wrap gap-2">
                    <Badge variant="outline">{formatDisplayValue(endpoint.source)}</Badge>
                    <Badge variant="outline">{Math.round(endpoint.confidence * 100)}%</Badge>
                  </div>
                </div>
              ))
            ) : (
              <p className="text-sm text-muted-foreground">No app URL or listening port has been inferred yet.</p>
            )}
          </CardContent>
        </Card>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>App directory structure</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-2">
          {app.app_structure?.length ? (
            app.app_structure.map((item) => (
              <Badge key={`${item.path}-${item.role}`} variant={item.role === "inferra_config" ? "success" : "outline"}>
                {item.path} - {formatDisplayValue(item.role)}
              </Badge>
            ))
          ) : (
            <p className="text-sm text-muted-foreground">No project structure was captured for this app.</p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Log sources</CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {app.log_sources?.length ? (
            app.log_sources.map((source, index) => (
              <div key={`${source.kind}-${source.path ?? source.command ?? index}`} className="rounded-xl border border-border bg-panel-inset p-3">
                <div className="mb-2 flex flex-wrap items-center gap-2">
                  <Badge variant="outline">{formatDisplayValue(source.kind)}</Badge>
                  <Badge variant={source.readable === false ? "warning" : "info"}>{formatDisplayValue(source.source)}</Badge>
                </div>
                <p className="font-medium">{source.label}</p>
                <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{source.path ?? source.command ?? source.stream ?? "-"}</p>
                <p className="mt-2 text-xs text-muted-foreground">
                  {source.exists == null ? "Availability depends on the manager/runtime stream." : source.exists ? "Found locally." : "Inferred but not found on disk yet."}
                </p>
              </div>
            ))
          ) : (
            <EmptyState title="No log source metadata" description="Inferra has runtime hints, but no file, stream, or manager log source has been resolved yet." />
          )}
        </CardContent>
      </Card>

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
            {savedGenerations.data?.count ? (
              <Badge variant="success">{savedGenerations.data.count} saved</Badge>
            ) : null}
          </div>
          {aiMonitor.errorMessage ? <ErrorState description={aiMonitor.errorMessage} onRetry={() => void runAiMonitor()} /> : null}
          {savedGenerations.data?.generations?.length ? (
            <div className="space-y-2">
              {savedGenerations.data.generations.slice(0, 3).map((generation) => (
                <button
                  key={generation.generation_id}
                  type="button"
                  className="flex w-full items-center justify-between gap-3 rounded-xl border border-border bg-panel-inset p-3 text-left text-sm transition hover:bg-secondary/40"
                  onClick={() => setAiResult(hydrateWorkspaceSavedGeneration(generation))}
                >
                  <span className="min-w-0">
                    <span className="block truncate font-medium">{generation.question || generation.focus}</span>
                    <span className="block truncate text-xs text-muted-foreground">{generation.scope_key}</span>
                  </span>
                  <Badge variant={generation.used_ai ? "success" : "outline"}>{generation.created_at}</Badge>
                </button>
              ))}
            </div>
          ) : null}
        </CardContent>
      </Card>
      {aiResult ? <InvestigationView result={aiResult} showRaw={isAdvancedMode(mode)} onRefresh={() => void runAiMonitor()} /> : null}
    </div>
  );
}

function hydrateWorkspaceSavedGeneration(generation: AiGeneration): InvestigationResponse {
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

function formatHostCpuPercent(resources: WorkspaceAppResources | null | undefined): string {
  if (resources?.cpu_percent == null) return "-";
  return `${formatPercentValue(resources.cpu_percent)}%`;
}

function formatRawCpuPercent(resources: WorkspaceAppResources | null | undefined): string {
  if (resources?.cpu_raw_percent == null) return "-";
  const cpuCount = resources.cpu_logical_processors ? ` across ${resources.cpu_logical_processors} logical CPU(s)` : "";
  return `${formatPercentValue(resources.cpu_raw_percent)}% single-core equivalent${cpuCount}`;
}

function formatPercentValue(value: number): string {
  return Number.isInteger(value) ? String(value) : value.toFixed(2).replace(/\.?0+$/, "");
}

function SetupActionCard({ action }: { action: WorkspaceSetupAction }) {
  return (
    <div className="rounded-md border border-border bg-panel-inset p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <p className="font-medium">{action.title}</p>
          <p className="mt-1 text-sm text-muted-foreground">{action.detail}</p>
        </div>
        {action.href ? (
          <Button variant="outline" size="sm" asChild>
            <Link to={action.href}>Open</Link>
          </Button>
        ) : null}
      </div>
      {action.command ? <p className="mt-3 break-all font-mono text-xs text-muted-foreground">{action.command}</p> : null}
      {action.config?.length ? (
        <div className="mt-3 space-y-2">
          {action.config.map((item, index) => (
            <div key={`${action.id}-${item.key}-${index}`} className="rounded-md border border-border/60 bg-background p-3">
              <p className="font-mono text-xs">
                {item.key}: {stringifyConfigValue(item.value)}
              </p>
              <p className="mt-1 text-xs text-muted-foreground">{item.reason}</p>
            </div>
          ))}
        </div>
      ) : null}
      {action.manifest_path ? <p className="mt-3 break-all font-mono text-xs text-muted-foreground">{action.manifest_path}</p> : null}
      {action.manifest_snippet ? (
        <pre className="mt-3 overflow-x-auto rounded-md border border-border/60 bg-background p-3 text-xs leading-6">
          <code>{action.manifest_snippet}</code>
        </pre>
      ) : null}
    </div>
  );
}

function setupBadgeVariant(status: string) {
  switch (status.trim().toLowerCase()) {
    case "ready":
      return "success";
    case "warning":
    case "partial":
      return "warning";
    case "critical":
    case "blocked":
      return "destructive";
    default:
      return "outline";
  }
}

function stringifyConfigValue(value: unknown): string {
  return JSON.stringify(value);
}

function InfoLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3 border-b border-border/50 pb-2 last:border-b-0">
      <span className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">{label}</span>
      <span className="break-all text-right font-mono text-xs">{value}</span>
    </div>
  );
}
