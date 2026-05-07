import { Play, RefreshCcw, Square } from "lucide-react";
import { useCallback } from "react";
import { toast } from "sonner";

import type { AiDoctorResponse, AiStatus, CollectorRow } from "@/api";
import { postJson } from "@/api";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { PageHeader } from "@/components/layout/page-header";
import { ErrorState, LoadingState } from "@/components/feedback/states";
import type { Mode } from "@/lib/experience";
import { useApiMutation, useApiQuery } from "@/lib/query";

export function ControlPage({ mode }: { mode: Mode }) {
  const collectors = useApiQuery<{ collectors: CollectorRow[]; queue_depth: number }>("/api/collectors");
  const ai = useApiQuery<AiStatus>("/api/ai/status");
  const doctor = useApiQuery<AiDoctorResponse>("/api/ai/doctor");
  const action = useApiMutation(async (verb: "start" | "stop") => postJson(`/api/collectors/${verb}`, {}));
  const aiMetric = ai.data?.enabled ? (ai.data.available ? "ready" : "degraded") : "disabled";
  const aiMetricNote = ai.data?.enabled ? (ai.data?.resolved_model ?? ai.data?.model ?? "No model") : "AI is disabled in config";

  const run = useCallback(
    async (verb: "start" | "stop") => {
      try {
        await action.run(verb);
        toast.success(`Collectors ${verb === "start" ? "started" : "stopped"}.`);
        void collectors.reload({ silent: true });
      } catch (error) {
        toast.error(`Could not ${verb} collectors`, { description: error instanceof Error ? error.message : String(error) });
      }
    },
    [action, collectors],
  );

  if ((collectors.isLoading && !collectors.data) || (ai.isLoading && !ai.data) || (doctor.isLoading && !doctor.data)) {
    return (
      <div className="space-y-6">
        <PageHeader title="Control" subtitle="Manage Inferra itself — collectors, AI provider, and storage policy." mode={mode} />
        <LoadingState title="Loading control plane" />
      </div>
    );
  }

  if ((collectors.errorMessage && !collectors.data) || (ai.errorMessage && !ai.data) || (doctor.errorMessage && !doctor.data)) {
    return (
      <div className="space-y-6">
        <PageHeader title="Control" subtitle="Manage Inferra itself — collectors, AI provider, and storage policy." mode={mode} />
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

      <div className="dashboard-grid">
        <Metric title="Collectors" value={String(collectors.data?.collectors.length ?? 0)} note={`queue depth ${collectors.data?.queue_depth ?? 0}`} />
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
            </div>
            <div className="space-y-3">
              {(collectors.data?.collectors ?? []).map((collector) => (
                <div key={collector.collector_id} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <p className="font-medium">{collector.collector_id}</p>
                      <p className="text-sm text-muted-foreground">{collector.source_type ?? "unknown source"}</p>
                    </div>
                    <Badge variant={collector.is_running ? "success" : "secondary"}>
                      {collector.is_running ? "running" : collector.status ?? "idle"}
                    </Badge>
                  </div>
                  <div className="mt-3 grid gap-2 text-sm text-muted-foreground md:grid-cols-2">
                    <p>Emitted: {collector.events_emitted ?? 0}</p>
                    <p>Dropped: {collector.dropped_events ?? 0}</p>
                    <p>Errors: {collector.error_count ?? 0}</p>
                    <p>Lag: {collector.lag_seconds ?? 0}s</p>
                    <p>Last event: {collector.last_event_at ?? "never"}</p>
                    <p>EPS: {collector.events_per_second?.toFixed?.(2) ?? collector.events_per_second ?? 0}</p>
                  </div>
                  {collector.last_error ? (
                    <Alert className="mt-3" variant="warning">
                      <div className="min-w-0">
                        <AlertTitle>Last collector error</AlertTitle>
                        <AlertDescription>{collector.last_error}</AlertDescription>
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
              <div className="rounded-2xl border border-border/60 bg-background/30 p-3 text-sm text-muted-foreground">
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

function Metric({ title, value, note }: { title: string; value: string; note: string }) {
  return (
    <Card className="border-border/70 bg-background/30">
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
    <div className="flex items-center justify-between gap-4 rounded-xl border border-border/60 bg-background/30 px-3 py-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium text-right">{value}</span>
    </div>
  );
}

