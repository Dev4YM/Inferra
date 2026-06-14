import { Activity, ArrowLeft, Copy, Filter, RefreshCcw, Search, Waypoints } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link, useParams, useSearchParams } from "react-router-dom";
import { toast } from "sonner";

import { apiTraceTimelinePath, type TraceTimelineResponse } from "@/api";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { RuntimeStatusCard } from "@/components/inferra/health";
import { TimelineView } from "@/components/inferra/timeline";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import type { Mode } from "@/lib/experience";
import { formatDisplayValue, formatRelativeDate, formatSeverity } from "@/lib/format";
import { buildEvidencePath, eventMatchesSearch, normalizeTraceId, shortTraceId } from "@/lib/observability";
import { useApiQuery } from "@/lib/query";

export function TracePage({ mode }: { mode: Mode }) {
  const { traceId } = useParams();
  const [searchParams] = useSearchParams();
  const searchKey = searchParams.toString();
  const normalizedTraceId = normalizeTraceId(traceId);
  const [windowDraft, setWindowDraft] = useState<TraceWindow>(() => initialTraceWindow(searchParams));
  const [requestedWindow, setRequestedWindow] = useState<TraceWindow>(() => initialTraceWindow(searchParams));
  const [serviceFilter, setServiceFilter] = useState("");
  const [environmentFilter, setEnvironmentFilter] = useState("");
  const [sourceFilter, setSourceFilter] = useState("");
  const [severityFilter, setSeverityFilter] = useState("");
  const [searchText, setSearchText] = useState("");
  const queryPath = normalizedTraceId
    ? apiTraceTimelinePath(normalizedTraceId, {
        limit: parseLimit(requestedWindow.limit, 500),
        start: normalizeDateTimeQuery(requestedWindow.start),
        end: normalizeDateTimeQuery(requestedWindow.end),
      })
    : null;
  const trace = useApiQuery<TraceTimelineResponse>(queryPath, {
    deps: [normalizedTraceId, requestedWindow.limit, requestedWindow.start, requestedWindow.end],
    staleTime: 5_000,
  });

  useEffect(() => {
    const next = initialTraceWindow(searchParams);
    setWindowDraft(next);
    setRequestedWindow(next);
    setServiceFilter("");
    setEnvironmentFilter("");
    setSourceFilter("");
    setSeverityFilter("");
    setSearchText("");
  }, [normalizedTraceId, searchKey]);

  const stats = useMemo(() => {
    const items = trace.data?.items ?? [];
    const services = summarizeValues(items.map((item) => item.service_id));
    const environments = summarizeValues(items.map((item) => item.deployment_environment));
    const sources = summarizeValues(items.map((item) => item.source_ref?.source_type));
    const spanIds = new Set(items.map((item) => item.span_id).filter(Boolean));
    const errorCount = items.filter((item) => severityRank(item.severity) >= 3).length;
    return {
      services,
      environments,
      sources,
      spanCount: spanIds.size,
      errorCount,
      firstSeen: items[0]?.timestamp ?? null,
      lastSeen: items[items.length - 1]?.timestamp ?? null,
    };
  }, [trace.data]);

  const visibleItems = useMemo(() => {
    const items = trace.data?.items ?? [];
    return items.filter((item) => {
      if (serviceFilter && item.service_id !== serviceFilter) return false;
      if (environmentFilter && (item.deployment_environment ?? "") !== environmentFilter) return false;
      if (sourceFilter && (item.source_ref?.source_type ?? "") !== sourceFilter) return false;
      if (severityFilter && severityRank(item.severity) < Number(severityFilter)) return false;
      return eventMatchesSearch(item, searchText);
    });
  }, [environmentFilter, searchText, serviceFilter, severityFilter, sourceFilter, trace.data]);

  const linkback = resolveTraceLinkback(searchParams, normalizedTraceId);

  if (!normalizedTraceId) {
    return <EmptyState title="Missing trace id" description="Open this page with a valid 32-hex W3C trace id." />;
  }

  if (trace.isLoading && !trace.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Trace timeline" subtitle="Loading the chronological event stream for this trace." mode={mode} />
        <LoadingState title="Loading trace timeline" />
      </div>
    );
  }

  if (trace.errorMessage && !trace.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Trace timeline" subtitle="Inferra could not load this trace." mode={mode} />
        <ErrorState description={trace.errorMessage} onRetry={() => void trace.reload()} />
      </div>
    );
  }

  if (!trace.data) {
    return <EmptyState title="No trace data" description="Inferra did not return a trace response for this id." />;
  }

  const traceData = trace.data;

  return (
    <div className="space-y-6">
      <PageHeader
        eyebrow="Trace"
        title={shortTraceId(traceData.trace_id)}
        subtitle="Chronological event timeline for one correlated W3C trace id."
        mode={mode}
        actions={
          <>
            {linkback ? (
              <Button variant="outline" size="sm" asChild>
                <Link to={linkback.to}>
                  <ArrowLeft className="size-4" />
                  {linkback.label}
                </Link>
              </Button>
            ) : null}
            <Button variant="outline" size="sm" asChild>
              <Link to={buildEvidencePath({ trace_id: normalizedTraceId })}>
                <Search className="size-4" />
                Open in evidence
              </Link>
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => void copyTraceId(traceData.trace_id)}
            >
              <Copy className="size-4" />
              Copy id
            </Button>
            <Button variant="outline" size="sm" onClick={() => void trace.reload({ silent: true })}>
              <RefreshCcw className={`size-4 ${trace.isRefreshing ? "animate-spin" : ""}`} />
              Refresh
            </Button>
          </>
        }
      />

      <div className="dashboard-grid">
        <RuntimeStatusCard
          icon={Activity}
          label="Loaded rows"
          value={String(traceData.count)}
          tone={traceData.count ? "info" : "secondary"}
          detail={`Oldest-first rows in the current ${requestedWindow.limit || "500"} item window.`}
        />
        <RuntimeStatusCard
          icon={Waypoints}
          label="Matching rows"
          value={String(visibleItems.length)}
          tone={visibleItems.length === traceData.count ? "success" : visibleItems.length ? "warning" : "secondary"}
          detail={visibleItems.length === traceData.count ? "No local filters applied." : "Rows still visible after local filters."}
        />
        <RuntimeStatusCard
          icon={Waypoints}
          label="Services"
          value={String(stats.services.length)}
          tone={stats.services.length > 1 ? "warning" : "success"}
          detail={stats.services.slice(0, 3).map((item) => item.label).join(", ") || "No service ids present"}
        />
        <RuntimeStatusCard
          icon={Activity}
          label="Span ids"
          value={String(stats.spanCount)}
          tone={stats.spanCount ? "info" : "secondary"}
          detail={`${stats.errorCount} error/critical rows · retention ${traceData.retention_hours}h`}
        />
      </div>

      <div className="content-grid">
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Filter className="size-4" />
                Trace window
              </CardTitle>
              <CardDescription>
                Limit controls the server fetch size. Start and end narrow the stored window before local filters apply.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="grid gap-3 md:grid-cols-3">
                <Input
                  aria-label="Trace timeline limit"
                  type="number"
                  min={1}
                  max={2000}
                  value={windowDraft.limit}
                  onChange={(event) => setWindowDraft((current) => ({ ...current, limit: event.target.value }))}
                />
                <Input
                  aria-label="Trace timeline start"
                  type="datetime-local"
                  value={windowDraft.start}
                  onChange={(event) => setWindowDraft((current) => ({ ...current, start: event.target.value }))}
                />
                <Input
                  aria-label="Trace timeline end"
                  type="datetime-local"
                  value={windowDraft.end}
                  onChange={(event) => setWindowDraft((current) => ({ ...current, end: event.target.value }))}
                />
              </div>
              <div className="flex flex-wrap gap-2">
                <Button onClick={() => setRequestedWindow(windowDraft)}>Apply window</Button>
                <Button
                  variant="outline"
                  onClick={() => {
                    const next = emptyTraceWindow();
                    setWindowDraft(next);
                    setRequestedWindow(next);
                  }}
                >
                  Reset window
                </Button>
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Search className="size-4" />
                Local filters
              </CardTitle>
              <CardDescription>
                Refine the already loaded timeline by service, environment, source type, severity, or free-text search.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-5">
                <Input
                  aria-label="Search loaded trace rows"
                  placeholder="search loaded rows"
                  value={searchText}
                  onChange={(event) => setSearchText(event.target.value)}
                />
                <select
                  aria-label="Filter trace rows by service"
                  className="h-11 rounded-xl border border-input bg-secondary/50 px-3 text-sm"
                  value={serviceFilter}
                  onChange={(event) => setServiceFilter(event.target.value)}
                >
                  <option value="">all services</option>
                  {stats.services.map((service) => (
                    <option key={service.label} value={service.label}>
                      {service.label} ({service.count})
                    </option>
                  ))}
                </select>
                <select
                  aria-label="Filter trace rows by environment"
                  className="h-11 rounded-xl border border-input bg-secondary/50 px-3 text-sm"
                  value={environmentFilter}
                  onChange={(event) => setEnvironmentFilter(event.target.value)}
                >
                  <option value="">all environments</option>
                  {stats.environments.map((environment) => (
                    <option key={environment.label} value={environment.label}>
                      {environment.label} ({environment.count})
                    </option>
                  ))}
                </select>
                <select
                  aria-label="Filter trace rows by source"
                  className="h-11 rounded-xl border border-input bg-secondary/50 px-3 text-sm"
                  value={sourceFilter}
                  onChange={(event) => setSourceFilter(event.target.value)}
                >
                  <option value="">all sources</option>
                  {stats.sources.map((source) => (
                    <option key={source.label} value={source.label}>
                      {source.label} ({source.count})
                    </option>
                  ))}
                </select>
                <select
                  aria-label="Filter trace rows by minimum severity"
                  className="h-11 rounded-xl border border-input bg-secondary/50 px-3 text-sm"
                  value={severityFilter}
                  onChange={(event) => setSeverityFilter(event.target.value)}
                >
                  <option value="">all severities</option>
                  <option value="1">info+</option>
                  <option value="2">warn+</option>
                  <option value="3">error+</option>
                  <option value="4">critical</option>
                </select>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  variant="outline"
                  onClick={() => {
                    setServiceFilter("");
                    setEnvironmentFilter("");
                    setSourceFilter("");
                    setSeverityFilter("");
                    setSearchText("");
                  }}
                >
                  Clear local filters
                </Button>
                {visibleItems.length !== traceData.items.length ? (
                  <Badge variant="warning">
                    Showing {visibleItems.length} of {traceData.items.length} loaded rows
                  </Badge>
                ) : (
                  <Badge variant="success">Showing all loaded rows</Badge>
                )}
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Timeline</CardTitle>
              <CardDescription>
                Ordered by timestamp and event id so duplicated timestamps remain stable across refreshes.
              </CardDescription>
            </CardHeader>
            <CardContent>
              {visibleItems.length ? (
                <TimelineView events={visibleItems} limit={visibleItems.length} />
              ) : (
                <EmptyState
                  title="No rows match the current filters"
                  description="The trace exists, but the current local filters removed every loaded row."
                  action={
                    <Button
                      variant="outline"
                      onClick={() => {
                        setServiceFilter("");
                        setEnvironmentFilter("");
                        setSourceFilter("");
                        setSeverityFilter("");
                        setSearchText("");
                      }}
                    >
                      Clear local filters
                    </Button>
                  }
                />
              )}
            </CardContent>
          </Card>
        </div>

        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Trace identity</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3 text-sm">
              <div className="rounded-md border border-border bg-panel-inset p-4">
                <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Trace id</p>
                <code className="mt-2 block overflow-x-auto text-xs text-primary">{traceData.trace_id}</code>
                <div className="mt-3 flex flex-wrap gap-2">
                  <Button variant="outline" size="sm" onClick={() => void copyTraceId(traceData.trace_id)}>
                    <Copy className="size-4" />
                    Copy full id
                  </Button>
                  <Button variant="outline" size="sm" asChild>
                    <Link to={buildEvidencePath({ trace_id: traceData.trace_id })}>
                      <Search className="size-4" />
                      Trace logs in evidence
                    </Link>
                  </Button>
                </div>
              </div>
              <div className="rounded-md border border-border bg-panel-inset p-4">
                <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Observed window</p>
                <div className="mt-3 space-y-2">
                  <TraceStat label="First seen" value={stats.firstSeen ? formatRelativeDate(stats.firstSeen) : "Unknown"} />
                  <TraceStat label="Last seen" value={stats.lastSeen ? formatRelativeDate(stats.lastSeen) : "Unknown"} />
                  <TraceStat label="Retention" value={`${traceData.retention_hours}h`} />
                  <TraceStat label="Server limit" value={String(traceData.limit)} />
                </div>
              </div>
              <div className="rounded-md border border-border bg-panel-inset p-4">
                <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Observed services</p>
                <div className="mt-3 flex flex-wrap gap-2">
                  {stats.services.length ? (
                    stats.services.map((service) => (
                      <Button key={service.label} variant="outline" size="sm" asChild>
                        <Link to={`/systems/${encodeURIComponent(service.label)}`}>
                          {service.label}
                          <Badge variant="outline" className="ml-1 border-transparent bg-secondary/60">
                            {service.count}
                          </Badge>
                        </Link>
                      </Button>
                    ))
                  ) : (
                    <span className="text-muted-foreground">No services recorded.</span>
                  )}
                </div>
              </div>
              <div className="rounded-md border border-border bg-panel-inset p-4">
                <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Environments</p>
                <div className="mt-3 flex flex-wrap gap-2">
                  {stats.environments.length ? (
                    stats.environments.map((environment) => (
                      <Badge key={environment.label} variant="outline">
                        {formatDisplayValue(environment.label)} ({environment.count})
                      </Badge>
                    ))
                  ) : (
                    <span className="text-muted-foreground">No deployment environment labels recorded.</span>
                  )}
                </div>
              </div>
              <div className="rounded-md border border-border bg-panel-inset p-4">
                <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Source types</p>
                <div className="mt-3 flex flex-wrap gap-2">
                  {stats.sources.length ? (
                    stats.sources.map((source) => (
                      <Badge key={source.label} variant="outline">
                        {formatDisplayValue(source.label)} ({source.count})
                      </Badge>
                    ))
                  ) : (
                    <span className="text-muted-foreground">No source types recorded.</span>
                  )}
                </div>
              </div>
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}

type TraceWindow = {
  limit: string;
  start: string;
  end: string;
};

type TraceFacet = {
  label: string;
  count: number;
};

function emptyTraceWindow(): TraceWindow {
  return {
    limit: "500",
    start: "",
    end: "",
  };
}

function initialTraceWindow(searchParams: URLSearchParams): TraceWindow {
  const defaults = emptyTraceWindow();
  return {
    limit: searchParams.get("limit") ?? defaults.limit,
    start: toDateTimeLocal(searchParams.get("start")),
    end: toDateTimeLocal(searchParams.get("end")),
  };
}

function parseLimit(value: string, fallback: number): number {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.max(1, Math.min(2000, parsed));
}

function summarizeValues(values: Array<string | null | undefined>): TraceFacet[] {
  const counts = new Map<string, number>();
  for (const value of values) {
    const normalized = String(value ?? "").trim();
    if (!normalized) continue;
    counts.set(normalized, (counts.get(normalized) ?? 0) + 1);
  }
  return Array.from(counts.entries())
    .map(([label, count]) => ({ label, count }))
    .sort((left, right) => right.count - left.count || left.label.localeCompare(right.label));
}

function severityRank(value: string | number | null | undefined): number {
  switch (formatSeverity(value)) {
    case "critical":
      return 4;
    case "error":
      return 3;
    case "warn":
      return 2;
    case "info":
      return 1;
    default:
      return 0;
  }
}

function normalizeDateTimeQuery(value: string): string | undefined {
  if (!value.trim()) return undefined;
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return undefined;
  return parsed.toISOString();
}

function toDateTimeLocal(value: string | null): string {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return "";
  const year = parsed.getFullYear();
  const month = String(parsed.getMonth() + 1).padStart(2, "0");
  const day = String(parsed.getDate()).padStart(2, "0");
  const hours = String(parsed.getHours()).padStart(2, "0");
  const minutes = String(parsed.getMinutes()).padStart(2, "0");
  return `${year}-${month}-${day}T${hours}:${minutes}`;
}

function resolveTraceLinkback(searchParams: URLSearchParams, traceId: string) {
  const from = searchParams.get("from");
  if (from === "incident") {
    const incidentId = searchParams.get("incidentId");
    if (incidentId) return { to: `/incidents/${encodeURIComponent(incidentId)}`, label: "Incident" };
  }
  if (from === "service") {
    const serviceId = searchParams.get("serviceId");
    if (serviceId) return { to: `/systems/${encodeURIComponent(serviceId)}`, label: "Service" };
  }
  if (from === "workspace") {
    const appName = searchParams.get("appName");
    if (appName) return { to: `/workspace/apps?name=${encodeURIComponent(appName)}`, label: "Workspace app" };
  }
  if (from === "evidence") {
    return { to: buildEvidencePath({ trace_id: traceId }), label: "Evidence" };
  }
  return null;
}

async function copyTraceId(traceId: string) {
  try {
    await navigator.clipboard.writeText(traceId);
    toast.success("Trace id copied to clipboard.");
  } catch (error) {
    toast.error("Could not copy trace id", {
      description: error instanceof Error ? error.message : String(error),
    });
  }
}

function TraceStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3 text-sm">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium">{value}</span>
    </div>
  );
}
