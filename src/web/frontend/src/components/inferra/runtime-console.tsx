import { Activity, AlertTriangle, Copy, RefreshCcw, ServerCog } from "lucide-react";
import { Link } from "react-router-dom";
import { toast } from "sonner";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { ServiceHealthBadge } from "@/components/inferra/health";
import {
  type InferraRuntimeSnapshot,
  runtimeStateLabel,
  useInferraRuntime,
} from "@/lib/inferra-runtime";
import { formatDisplayValue } from "@/lib/format";
import { cn } from "@/lib/utils";

function runtimeTone(state: InferraRuntimeSnapshot["state"]) {
  if (state === "online") return "success" as const;
  if (state === "degraded" || state === "auth_required") return "warning" as const;
  if (state === "offline") return "destructive" as const;
  return "secondary" as const;
}

export function InferraRuntimeRail({ runtime }: { runtime: InferraRuntimeSnapshot }) {
  const tone = runtimeTone(runtime.state);
  const dotClass =
    tone === "success"
      ? "bg-success"
      : tone === "warning"
        ? "bg-warning"
        : tone === "destructive"
          ? "bg-critical"
          : "bg-muted-foreground animate-pulse";

  return (
    <div className="rounded-sm border border-border bg-panel-inset px-2.5 py-2">
      <div className="flex items-center gap-2">
        <span className={cn("size-2 rounded-full", dotClass)} aria-hidden />
        <div className="min-w-0 flex-1">
          <p className="font-data text-[11px] font-medium text-sidebar-foreground">{runtimeStateLabel(runtime.state)}</p>
          <p className="truncate text-[10px] text-sidebar-muted">
            {runtime.health?.runtime ?? runtime.ready?.runtime ?? "rust"} · same-origin proxy
          </p>
        </div>
        {runtime.isRefreshing ? <Activity className="size-3.5 shrink-0 animate-pulse text-sidebar-muted" /> : null}
      </div>
    </div>
  );
}

export function InferraRuntimeBanner({ runtime }: { runtime: InferraRuntimeSnapshot }) {
  if (runtime.state === "loading" || runtime.state === "online") return null;

  const variant = runtime.state === "offline" ? "destructive" : "warning";
  const title =
    runtime.state === "offline"
      ? "Inferra API is not responding"
      : runtime.state === "auth_required"
        ? "Inferra API requires authentication"
        : "Inferra runtime is degraded";

  const description =
    runtime.errorMessage ??
    runtime.health?.degraded_reasons?.join(" · ") ??
    "Storage or readiness checks failed. Open Control for runtime details.";

  return (
    <Alert variant={variant} className="mb-4">
      <AlertTriangle className="size-4" />
      <div className="min-w-0">
        <AlertTitle>{title}</AlertTitle>
        <AlertDescription>{description}</AlertDescription>
        {runtime.state === "offline" ? (
          <p className="mt-2 text-xs text-muted-foreground">
            Dev tip: Vite proxies to <code className="font-data">INFERRA_API_URL</code> (default from{" "}
            <code className="font-data">inferra.dev.toml</code>). A zombie listener on 7433 while the live API is on 7434
            causes partial timeouts.
          </p>
        ) : null}
        <div className="mt-3 flex flex-wrap gap-2">
          <Button asChild variant="outline" size="sm">
            <Link to="/control">Open Control</Link>
          </Button>
          {runtime.state === "auth_required" ? (
            <Button asChild variant="outline" size="sm">
              <Link to="/settings">Token settings</Link>
            </Button>
          ) : (
            <Button variant="outline" size="sm" onClick={() => void runtime.reload()}>
              <RefreshCcw className="size-4" />
              Retry health
            </Button>
          )}
        </div>
      </div>
    </Alert>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-start justify-between gap-4 rounded-md border border-border bg-panel-inset px-3 py-2 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className="max-w-[65%] text-right font-data text-xs leading-5 break-all">{value}</span>
    </div>
  );
}

export function InferraRuntimePanel({ runtime }: { runtime?: InferraRuntimeSnapshot }) {
  const resolved = runtime ?? useInferraRuntime();
  const health = resolved.health;
  const status = health?.status ?? resolved.ready?.status ?? resolved.state;
  const copyPaths = async () => {
    const lines = [
      "Inferra runtime snapshot",
      `state: ${resolved.state}`,
      `status: ${status}`,
      `runtime: ${health?.runtime ?? resolved.ready?.runtime ?? "unknown"}`,
      `storage_writes_ok: ${String(health?.storage_writes_ok ?? resolved.ready?.storage_writes_ok ?? false)}`,
      health?.config_path ? `config_path: ${health.config_path}` : null,
      health?.data_dir ? `data_dir: ${health.data_dir}` : null,
      health?.events_db ? `events_db: ${health.events_db}` : null,
      health?.incidents_db ? `incidents_db: ${health.incidents_db}` : null,
      health?.degraded_reasons?.length ? `degraded_reasons:\n${health.degraded_reasons.map((r) => `  - ${r}`).join("\n")}` : null,
      resolved.errorMessage ? `error: ${resolved.errorMessage}` : null,
    ].filter(Boolean);
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      toast.success("Runtime snapshot copied");
    } catch (error) {
      toast.error("Could not copy snapshot", { description: error instanceof Error ? error.message : String(error) });
    }
  };

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-3">
        <div>
          <CardTitle className="flex items-center gap-2">
            <ServerCog className="size-4 text-accent" />
            Inferra runtime
          </CardTitle>
          <CardDescription>
            This Rust API process serves the console. It is separate from the systems Inferra observes.
          </CardDescription>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" onClick={() => void resolved.reload()} disabled={resolved.isRefreshing}>
            <RefreshCcw className={cn("size-4", resolved.isRefreshing && "animate-spin")} />
            Refresh
          </Button>
          <Button variant="outline" size="sm" onClick={() => void copyPaths()}>
            <Copy className="size-4" />
            Copy snapshot
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        <div className="flex flex-wrap items-center gap-2">
          <ServiceHealthBadge status={status} />
          <Badge variant={runtimeTone(resolved.state)}>{runtimeStateLabel(resolved.state)}</Badge>
          {health?.runtime ? <Badge variant="outline">{formatDisplayValue(health.runtime)}</Badge> : null}
          {health?.ai_enabled != null ? (
            <Badge variant="outline">AI {health.ai_enabled ? "enabled" : "disabled"}</Badge>
          ) : null}
        </div>

        {resolved.errorMessage && resolved.state !== "online" ? (
          <Alert variant={resolved.state === "offline" ? "destructive" : "warning"}>
            <div className="min-w-0">
              <AlertTitle>Health probe</AlertTitle>
              <AlertDescription>{resolved.errorMessage}</AlertDescription>
            </div>
          </Alert>
        ) : null}

        {health?.degraded_reasons?.length ? (
          <Alert variant="warning">
            <div className="min-w-0">
              <AlertTitle>Degraded reasons</AlertTitle>
              <AlertDescription>
                {health.degraded_reasons.map((reason) => (
                  <span key={reason} className="block">
                    • {reason}
                  </span>
                ))}
              </AlertDescription>
            </div>
          </Alert>
        ) : null}

        <div className="grid gap-2 md:grid-cols-2">
          <DetailRow label="API health" value="/api/health" />
          <DetailRow label="Liveness probe" value="/healthz" />
          <DetailRow label="Readiness probe" value="/readyz" />
          <DetailRow
            label="Storage writes"
            value={String(health?.storage_writes_ok ?? resolved.ready?.storage_writes_ok ?? "unknown")}
          />
          {health?.config_path ? <DetailRow label="Config path" value={health.config_path} /> : null}
          {health?.data_dir ? <DetailRow label="Data directory" value={health.data_dir} /> : null}
          {health?.events_db ? <DetailRow label="Events DB" value={health.events_db} /> : null}
          {health?.incidents_db ? <DetailRow label="Incidents DB" value={health.incidents_db} /> : null}
        </div>

        {resolved.state === "offline" ? (
          <div className="rounded-md border border-dashed border-border px-3 py-3 text-sm text-muted-foreground">
            <p className="font-medium text-foreground">Multiple instances?</p>
            <p className="mt-1 leading-6">
              On Windows, check <code className="font-data">inferra service status</code> for the installed service and
              avoid a second <code className="font-data">inferra serve</code> on the same port. Stale listeners that accept
              TCP but never respond usually need a process kill or reboot.
            </p>
          </div>
        ) : null}
      </CardContent>
    </Card>
  );
}
