import { Copy, DatabaseZap, Play, RefreshCcw, Square } from "lucide-react";
import { useCallback } from "react";
import { toast } from "sonner";

import type { AiDoctorResponse, AiStatus, CollectorRow, EventRow, ScannerStatusResponse } from "@/api";
import { postJson } from "@/api";
import { InferraRuntimePanel } from "@/components/inferra/runtime-console";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/layout/page-header";
import { ErrorState, LoadingState } from "@/components/feedback/states";
import type { Mode } from "@/lib/experience";
import { formatDisplayValue } from "@/lib/format";
import { useInferraRuntime } from "@/lib/inferra-runtime";
import { useApiMutation, useApiQuery } from "@/lib/query";
import { TimelineView } from "@/components/inferra/timeline";

export function ControlPage({ mode }: { mode: Mode }) {
  const runtime = useInferraRuntime();
  const collectors = useApiQuery<{ collectors: CollectorRow[]; queue_depth: number }>("/api/collectors");
  const ai = useApiQuery<AiStatus>("/api/ai/status");
  const doctor = useApiQuery<AiDoctorResponse>("/api/ai/doctor");
  const scanner = useApiQuery<ScannerStatusResponse>("/api/scanner/status", { staleTime: 15_000 });
  const collectorLogs = useApiQuery<{ logs: EventRow[] }>("/api/logs?search=collector&limit=25", { staleTime: 20_000 });
  const action = useApiMutation(async (verb: "start" | "stop") => postJson(`/api/collectors/${verb}`, {}));
  const scannerRun = useApiMutation(async () => postJson("/api/scanner/run", {}));
  const aiMetric = ai.data?.enabled ? (ai.data.available ? "ready" : "degraded") : "disabled";
  const aiMetricNote = ai.data?.enabled ? (ai.data?.resolved_model ?? ai.data?.model ?? "No model") : "AI is disabled in config";
  const collectorRows = collectors.data?.collectors ?? [];
  const problemCollectors = collectorRows.filter((collector) => (collector.error_count ?? 0) > 0 || Boolean(collector.last_error));
  const activeProblemCollectors = problemCollectors.filter((collector) => collector.status === "error");
  const collectorLogRows = collectorLogs.data?.logs ?? [];

  const run = useCallback(
    async (verb: "start" | "stop") => {
      try {
        await action.run(verb);
        toast.success(`Collectors ${verb === "start" ? "started" : "stopped"}.`);
        void collectors.reload({ silent: true });
        void collectorLogs.reload({ silent: true });
      } catch (error) {
        toast.error(`Could not ${verb} collectors`, { description: error instanceof Error ? error.message : String(error) });
      }
    },
    [action, collectors],
  );

  const copyCollectorReport = useCallback(async () => {
    const report = buildCollectorReport(collectorRows, collectorLogRows, collectors.data?.queue_depth ?? 0);
    try {
      await navigator.clipboard.writeText(report);
      toast.success("Collector report copied");
    } catch (error) {
      toast.error("Could not copy collector report", { description: error instanceof Error ? error.message : String(error) });
    }
  }, [collectorRows, collectorLogRows, collectors.data?.queue_depth]);

  if ((collectors.isLoading && !collectors.data) || (ai.isLoading && !ai.data) || (doctor.isLoading && !doctor.data)) {
    return (
      <div className="space-y-6">
        <PageHeader title="Control" subtitle="Manage Inferra itself — collectors, AI provider, and storage policy." mode={mode} />
        <InferraRuntimePanel runtime={runtime} />
        <LoadingState title="Loading control plane" />
      </div>
    );
  }

  const collectorsUnavailable = collectors.errorMessage && !collectors.data;
  const aiUnavailable = ai.errorMessage && !ai.data;
  const doctorUnavailable = doctor.errorMessage && !doctor.data;

  if (collectorsUnavailable && aiUnavailable && doctorUnavailable) {
    return (
      <div className="space-y-6">
        <PageHeader title="Control" subtitle="Manage Inferra itself — collectors, AI provider, and storage policy." mode={mode} />
        <InferraRuntimePanel runtime={runtime} />
        <ErrorState
          description={collectors.errorMessage ?? ai.errorMessage ?? doctor.errorMessage ?? "Unknown error"}
          onRetry={() => {
            void collectors.reload();
            void ai.reload();
            void doctor.reload();
          }}
        />
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        title="Control"
        subtitle="Manage Inferra itself while staying read-only toward the systems it observes."
        mode={mode}
        actions={
          <Button variant="outline" size="sm" onClick={() => { void collectors.reload({ silent: true }); void ai.reload({ silent: true }); void doctor.reload({ silent: true }); }}>
            <RefreshCcw className={`size-4 ${collectors.isRefreshing || ai.isRefreshing || doctor.isRefreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      <InferraRuntimePanel runtime={runtime} />

      {collectorsUnavailable ? (
        <Alert variant="warning">
          <div className="min-w-0">
            <AlertTitle>Collectors API unavailable</AlertTitle>
            <AlertDescription>{collectors.errorMessage}</AlertDescription>
          </div>
        </Alert>
      ) : null}

      <div className="dashboard-grid">
        <Metric title="Collectors" value={String(collectors.data?.collectors.length ?? 0)} note={`queue depth ${collectors.data?.queue_depth ?? 0}`} />
        <Metric title="Collector errors" value={String(activeProblemCollectors.reduce((sum, row) => sum + (row.error_count ?? 0), 0))} note={`${problemCollectors.length} collectors have error history`} />
        <Metric title="AI provider" value={aiMetric} note={aiMetricNote} />
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(360px,1fr)]">
        <Card>
          <CardHeader>
            <CardTitle>Collectors</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="flex flex-wrap gap-3">
              <Button onClick={() => void run("start")} disabled={action.isPending}>
                <Play className="size-4" />
                Start
              </Button>
              <Button variant="outline" onClick={() => void run("stop")} disabled={action.isPending}>
                <Square className="size-4" />
                Stop
              </Button>
              <Button variant="outline" onClick={() => void copyCollectorReport()} disabled={!collectorRows.length}>
                <Copy className="size-4" />
                Copy collector report
              </Button>
            </div>
            {problemCollectors.length ? (
              <Alert variant={activeProblemCollectors.length ? "warning" : "default"}>
                <div className="min-w-0">
                  <AlertTitle>
                    {activeProblemCollectors.length ? "Collector errors are active" : "Collector error history"}
                  </AlertTitle>
                  <AlertDescription>
                    {activeProblemCollectors.length
                      ? "System health is degraded while collectors remain in error state. Copy the report to share status, last errors, hints, and log extraction routes."
                      : "Collectors have recovered, so health should not stay degraded. The report still includes the last errors for support context."}
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}
            <div className="space-y-3">
              {collectorRows.map((collector) => (
                <div key={collector.collector_id} className="rounded-md border border-border bg-panel-inset p-4">
                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <p className="font-medium">{collector.collector_id}</p>
                      <p className="text-sm text-muted-foreground">{formatDisplayValue(collector.source_type ?? "unknown source")}</p>
                    </div>
                    <Badge variant={collectorBadgeVariant(collector)}>
                      {formatDisplayValue(collector.status ?? (collector.is_running ? "running" : "idle"))}
                    </Badge>
                  </div>
                  <div className="mt-3 grid gap-2 text-sm text-muted-foreground md:grid-cols-2">
                    <p>Emitted: {collector.events_emitted ?? 0}</p>
                    <p>Dropped: {collector.dropped_events ?? 0}</p>
                    <p>Errors: {collector.error_count ?? 0}</p>
                    <p>Lag: {collector.lag_seconds ?? 0}s</p>
                    <p>Last event: {collector.last_event_at ?? "never"}</p>
                    <p>EPS: {collector.events_per_second?.toFixed?.(2) ?? collector.events_per_second ?? 0}</p>
                    <p>Last error: {collector.last_error_at ?? "none"}</p>
                    <p>Log route: {collector.log_query ?? `/api/logs?search=${collector.collector_id}&limit=100`}</p>
                  </div>
                  {collector.last_error ? (
                    <Alert className="mt-3" variant="warning">
                      <div className="min-w-0">
                        <AlertTitle>Last collector error</AlertTitle>
                        <AlertDescription>{collector.last_error}</AlertDescription>
                      </div>
                    </Alert>
                  ) : null}
                  {collector.error_hint ? (
                    <Alert className="mt-3" variant="default">
                      <div className="min-w-0">
                        <AlertTitle>Likely fix</AlertTitle>
                        <AlertDescription>{collector.error_hint}</AlertDescription>
                      </div>
                    </Alert>
                  ) : null}
                </div>
              ))}
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Scanner service</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <Button
              variant="outline"
              onClick={async () => {
                try {
                  await scannerRun.run(undefined);
                  toast.success("Scanner refreshed");
                  void scanner.reload({ silent: true });
                } catch (error) {
                  toast.error("Scanner refresh failed", { description: error instanceof Error ? error.message : String(error) });
                }
              }}
              disabled={scannerRun.isPending}
            >
              <DatabaseZap className="size-4" />
              Refresh workspace scan
            </Button>
            <div className="space-y-3">
              {Object.entries(scanner.data?.scanner ?? {}).map(([key, row]) => (
                <div key={key} className="rounded-md border border-border bg-panel-inset p-4">
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <p className="font-medium">{formatDisplayValue(row.data_type)}</p>
                      <p className="text-sm text-muted-foreground">{formatDisplayValue(row.mode)}</p>
                    </div>
                    {row.interval_seconds ? <Badge variant="outline">{row.interval_seconds}s</Badge> : null}
                  </div>
                  {row.last_scanned_at ? (
                    <p className="mt-2 text-sm text-muted-foreground">
                      Last scan {row.last_scanned_at}; next in {row.next_scan_in_seconds ?? 0}s.
                    </p>
                  ) : null}
                  {row.min_interval_seconds && row.max_interval_seconds ? (
                    <p className="mt-1 text-xs text-muted-foreground">
                      Customizable via config between {row.min_interval_seconds}s and {row.max_interval_seconds}s.
                    </p>
                  ) : null}
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.1fr)_minmax(360px,0.9fr)]">
        <Card>
          <CardHeader className="flex flex-row items-center justify-between gap-3">
            <CardTitle>Collector logs</CardTitle>
            <Button variant="outline" size="sm" onClick={() => void copyCollectorReport()} disabled={!collectorRows.length}>
              <Copy className="size-4" />
              Copy report
            </Button>
          </CardHeader>
          <CardContent className="space-y-3">
            {collectorLogRows.length ? (
              <TimelineView events={collectorLogRows} limit={10} compact />
            ) : (
              <p className="text-sm text-muted-foreground">
                No normalized collector log events were found. Runtime collector errors are shown on the collector rows above and included in the copied report.
              </p>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>AI provider</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <StatusRow label="Enabled" value={String(ai.data?.enabled ?? false)} />
            <StatusRow label="Available" value={String(ai.data?.available ?? false)} />
            <StatusRow label="Model" value={ai.data?.resolved_model ?? ai.data?.model ?? "unknown"} />
            <StatusRow label="Base URL" value={ai.data?.base_url ?? "unknown"} />
            <StatusRow label="Remote allowed" value={String(doctor.data?.allow_remote ?? ai.data?.allow_remote ?? false)} />
            {ai.data?.reason ? (
              <Alert variant="warning">
                <div className="min-w-0">
                  <AlertTitle>Provider note</AlertTitle>
                  <AlertDescription>{ai.data.reason}</AlertDescription>
                </div>
              </Alert>
            ) : null}
            {doctor.data?.warnings?.length ? (
              <Alert variant="warning">
                <div className="min-w-0">
                  <AlertTitle>Doctor warnings</AlertTitle>
                  <AlertDescription>
                    {doctor.data.warnings.map((warning, index) => (
                      <span key={index} className="block">
                        • {warning}
                      </span>
                    ))}
                  </AlertDescription>
                </div>
              </Alert>
            ) : null}
            {doctor.data?.guidance?.length ? (
              <div className="rounded-md border border-border bg-panel-inset p-3 text-sm text-muted-foreground">
                {doctor.data.guidance.map((item, index) => (
                  <p key={index}>• {item}</p>
                ))}
              </div>
            ) : null}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function buildCollectorReport(collectors: CollectorRow[], logs: EventRow[], queueDepth: number) {
  const lines = [
    "Inferra collector diagnostic report",
    `Generated: ${new Date().toISOString()}`,
    `Queue depth: ${queueDepth}`,
    "",
    "Collectors:",
  ];
  for (const collector of collectors) {
    lines.push(`- ${collector.collector_id}`);
    lines.push(`  status: ${collector.status ?? "unknown"}`);
    lines.push(`  running: ${String(collector.is_running ?? false)}`);
    lines.push(`  source: ${collector.source_type ?? "unknown"}`);
    lines.push(`  emitted: ${collector.events_emitted ?? 0}`);
    lines.push(`  dropped: ${collector.dropped_events ?? 0}`);
    lines.push(`  errors: ${collector.error_count ?? 0}`);
    lines.push(`  last_event_at: ${collector.last_event_at ?? "never"}`);
    lines.push(`  last_error_at: ${collector.last_error_at ?? "none"}`);
    lines.push(`  log_query: ${collector.log_query ?? `/api/logs?search=${collector.collector_id}&limit=100`}`);
    if (collector.last_error) lines.push(`  last_error: ${collector.last_error}`);
    if (collector.error_hint) lines.push(`  likely_fix: ${collector.error_hint}`);
  }
  lines.push("");
  lines.push("Recent normalized collector logs:");
  if (logs.length) {
    for (const log of logs.slice(0, 25)) {
      lines.push(`- ${log.timestamp ?? ""} ${log.severity ?? ""} ${log.service_id ?? ""}: ${log.message ?? ""}`);
    }
  } else {
    lines.push("- none returned by /api/logs?search=collector&limit=25");
  }
  return lines.join("\n");
}

function collectorBadgeVariant(collector: CollectorRow) {
  if (collector.status === "error") return "warning" as const;
  if (collector.status === "unavailable") return "secondary" as const;
  return collector.is_running ? "success" as const : "secondary" as const;
}

function Metric({ title, value, note }: { title: string; value: string; note: string }) {
  return (
    <Card className="border-border bg-panel-inset">
      <CardContent className="p-5">
        <p className="text-xs font-semibold uppercase tracking-[0.2em] text-muted-foreground">{title}</p>
        <p className="mt-2 text-3xl font-semibold">{value}</p>
        <p className="mt-1 text-sm text-muted-foreground">{note}</p>
      </CardContent>
    </Card>
  );
}

function StatusRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-4 rounded-xl border border-border bg-panel-inset px-3 py-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium text-right">{value}</span>
    </div>
  );
}
