import { AlertTriangle, Filter, RadioTower, RefreshCcw, Search, Waypoints } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";

import type { AnomalyStatus, EventDetailResponse, EventRow, LogsV2Response } from "@/api";
import { apiLogsV2Path } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { JsonInspector } from "@/components/ui/json-inspector";
import { Input } from "@/components/ui/input";
import { Td, Th, Table, TableWrap } from "@/components/ui/table";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import type { Mode } from "@/lib/experience";
import { formatDisplayValue, formatRiskTone, formatRelativeDate, formatSeverity, formatSeverityLabel, summarizeEvent } from "@/lib/format";
import { buildTracePath, hasValidTraceId, normalizeTraceId, shortTraceId } from "@/lib/observability";
import { useApiQuery } from "@/lib/query";
import { SeverityDistribution } from "@/components/inferra/charts";
import { RuntimeStatusCard } from "@/components/inferra/health";

export function EvidencePage({ mode }: { mode: Mode }) {
  const [searchParams] = useSearchParams();
  const [filters, setFilters] = useState<EvidenceFilters>(() => initialFiltersFromSearch(searchParams));
  const [applied, setApplied] = useState<EvidenceFilters>(() => initialFiltersFromSearch(searchParams));
  const [selectedEventId, setSelectedEventId] = useState<string | null>(null);
  const [selectedEventPreview, setSelectedEventPreview] = useState<EventRow | null>(null);

  const path = useMemo(() => {
    return apiLogsV2Path({
      service: applied.service.trim() || undefined,
      severity: applied.severity || undefined,
      source_type: applied.sourceType.trim() || undefined,
      q: applied.query.trim() || undefined,
      trace_id: normalizedTraceIdOrEmpty(applied.traceId),
      limit: parseLimit(applied.limit),
      start: normalizeDateTimeQuery(applied.start),
      end: normalizeDateTimeQuery(applied.end),
      cursor_timestamp: applied.cursorTimestamp || undefined,
      cursor_event_id: applied.cursorEventId || undefined,
    });
  }, [applied]);

  const logs = useApiQuery<LogsV2Response>(path, { deps: [path] });
  const selectedEvent = useApiQuery<EventDetailResponse>(selectedEventId ? `/api/events/${encodeURIComponent(selectedEventId)}` : null, { deps: [selectedEventId] });
  const rows = logs.data?.items ?? [];
  const selectedDetails = selectedEvent.data?.event ?? selectedEventPreview;
  const anomalyService = applied.service.trim() || selectedDetails?.service_id || "";
  const anomaly = useApiQuery<AnomalyStatus>(
    anomalyService ? `/api/anomaly/${encodeURIComponent(anomalyService)}/status` : null,
    { deps: [anomalyService] },
  );
  const severityCounts = rows.reduce<Record<string, number>>((acc, event) => {
    const key = formatSeverity(event.severity);
    acc[key] = (acc[key] ?? 0) + 1;
    return acc;
  }, {});
  const servicesSeen = new Set(rows.map((event) => event.service_id).filter(Boolean)).size;
  const traceLinked = rows.filter((event) => hasValidTraceId(event.trace_id)).length;
  const selectedTraceId = hasValidTraceId(selectedDetails?.trace_id) ? normalizeTraceId(selectedDetails?.trace_id) : "";

  useEffect(() => {
    if (!rows.length) {
      setSelectedEventId(null);
      setSelectedEventPreview(null);
      return;
    }

    const nextSelected = rows[0];
    setSelectedEventPreview(nextSelected);
    setSelectedEventId(nextSelected.event_id ?? null);
  }, [path, rows]);

  const applyFilters = () => {
    const next = {
      ...filters,
      traceId: normalizedTraceIdOrEmpty(filters.traceId) ?? "",
      cursorTimestamp: "",
      cursorEventId: "",
    };
    setFilters(next);
    setApplied(next);
    setSelectedEventId(null);
    setSelectedEventPreview(null);
  };

  const clearFilters = () => {
    const next = emptyFilters();
    setFilters(next);
    setApplied(next);
    setSelectedEventId(null);
    setSelectedEventPreview(null);
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title="Evidence"
        subtitle="Trace-aware normalized log explorer with server-side filters, keyset paging, and direct jumps into trace timelines."
        mode={mode}
        actions={
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" onClick={clearFilters}>
              Clear filters
            </Button>
            <Button variant="outline" size="sm" onClick={() => void logs.reload({ silent: true })}>
              <RefreshCcw className={`size-4 ${logs.isRefreshing ? "animate-spin" : ""}`} />
              Refresh
            </Button>
          </div>
        }
      />

      <div className="dashboard-grid">
        <RuntimeStatusCard
          icon={RadioTower}
          label="Loaded rows"
          value={String(rows.length)}
          tone="info"
          detail={`Retention ${logs.data?.retention_hours ?? "?"}h · page size ${logs.data?.limit ?? parseLimit(applied.limit)}`}
        />
        <RuntimeStatusCard
          icon={AlertTriangle}
          label="Errors"
          value={String((severityCounts.error ?? 0) + (severityCounts.critical ?? 0))}
          tone={(severityCounts.error ?? 0) + (severityCounts.critical ?? 0) ? "warning" : "success"}
          detail="Rows at error severity or above in the current page."
        />
        <RuntimeStatusCard icon={Search} label="Services seen" value={String(servicesSeen)} tone="info" detail="Unique services represented by the current filters." />
        <RuntimeStatusCard
          icon={Waypoints}
          label="Trace-linked"
          value={String(traceLinked)}
          tone={traceLinked ? "success" : "secondary"}
          detail={selectedTraceId ? `Selected trace ${shortTraceId(selectedTraceId)}` : "Rows carrying a W3C trace id."}
        />
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Filter className="size-4" />
            Filter logs
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={logs.data?.log_fts_enabled ? "success" : "outline"}>
              {logs.data?.log_fts_enabled ? "FTS enabled (`q=`)" : "LIKE fallback search"}
            </Badge>
            <Badge variant="outline">{applied.cursorTimestamp ? "Older page" : "Newest page"}</Badge>
            {applied.traceId ? <Badge variant="info">Trace {shortTraceId(applied.traceId)}</Badge> : null}
            {logs.data?.next_cursor ? <Badge variant="outline">More rows available</Badge> : null}
          </div>

          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
            <Input
              aria-label="Filter logs by service id"
              placeholder="service id"
              value={filters.service}
              onChange={(event) => setFilters((current) => ({ ...current, service: event.target.value }))}
            />
            <Input
              aria-label="Filter logs by source type"
              placeholder="source type"
              value={filters.sourceType}
              onChange={(event) => setFilters((current) => ({ ...current, sourceType: event.target.value }))}
            />
            <Input
              aria-label="Filter logs by trace id"
              placeholder="trace id or traceparent"
              value={filters.traceId}
              onChange={(event) => setFilters((current) => ({ ...current, traceId: event.target.value }))}
            />
            <select
              aria-label="Filter logs by severity"
              className="h-11 rounded-xl border border-input bg-secondary/50 px-3 text-sm"
              value={filters.severity}
              onChange={(event) => setFilters((current) => ({ ...current, severity: event.target.value }))}
            >
              <option value="">any severity</option>
              <option value="1">info+</option>
              <option value="2">warn+</option>
              <option value="3">error+</option>
              <option value="4">critical</option>
            </select>
            <Input
              aria-label="Filter logs by text"
              placeholder="message / text search"
              value={filters.query}
              onChange={(event) => setFilters((current) => ({ ...current, query: event.target.value }))}
            />
            <Input
              aria-label="Filter logs start time"
              type="datetime-local"
              value={filters.start}
              onChange={(event) => setFilters((current) => ({ ...current, start: event.target.value }))}
            />
            <Input
              aria-label="Filter logs end time"
              type="datetime-local"
              value={filters.end}
              onChange={(event) => setFilters((current) => ({ ...current, end: event.target.value }))}
            />
            <Input
              aria-label="Filter logs page size"
              type="number"
              min={1}
              max={2000}
              value={filters.limit}
              onChange={(event) => setFilters((current) => ({ ...current, limit: event.target.value }))}
            />
          </div>

          <div className="flex flex-wrap gap-2">
            <Button onClick={applyFilters}>Apply filters</Button>
            {logs.data?.next_cursor ? (
              <Button
                variant="outline"
                onClick={() =>
                  setApplied((current) => ({
                    ...current,
                    cursorTimestamp: logs.data?.next_cursor?.cursor_timestamp ?? "",
                    cursorEventId: logs.data?.next_cursor?.cursor_event_id ?? "",
                  }))
                }
              >
                Load older page
              </Button>
            ) : null}
            {applied.cursorTimestamp ? (
              <Button
                variant="outline"
                onClick={() =>
                  setApplied((current) => ({
                    ...current,
                    cursorTimestamp: "",
                    cursorEventId: "",
                  }))
                }
              >
                Back to newest
              </Button>
            ) : null}
          </div>

          {filters.traceId && !hasValidTraceId(filters.traceId) ? (
            <p className="text-xs text-muted-foreground">
              Trace input accepts either a 32-hex W3C trace id or a full `traceparent` header.
            </p>
          ) : null}
        </CardContent>
      </Card>

      {logs.isLoading && !logs.data ? <LoadingState title="Loading evidence" /> : null}
      {logs.errorMessage && !logs.data ? <ErrorState description={logs.errorMessage} onRetry={() => void logs.reload()} /> : null}

      {logs.data ? (
        rows.length ? (
          <div className="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(320px,0.9fr)]">
            <TableWrap>
              <Table className="align-top">
                <thead>
                  <tr>
                    <Th>Timestamp</Th>
                    <Th>Severity</Th>
                    <Th>Service</Th>
                    <Th>Source</Th>
                    <Th>Trace</Th>
                    <Th>Message</Th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((event, index) => (
                    <tr
                      key={`${event.event_id ?? "event"}-${index}`}
                      className="cursor-pointer align-top transition hover:bg-secondary/50"
                      onClick={() => {
                        setSelectedEventPreview(event);
                        setSelectedEventId(event.event_id ?? null);
                      }}
                    >
                      <Td className="text-muted-foreground">{formatRelativeDate(event.timestamp)}</Td>
                      <Td>
                        <Badge variant={formatRiskTone(formatSeverity(event.severity))}>{formatSeverityLabel(event.severity)}</Badge>
                      </Td>
                      <Td>{event.service_id ?? "unknown"}</Td>
                      <Td className="text-muted-foreground">{formatDisplayValue(event.source_ref?.source_type ?? "unknown")}</Td>
                      <Td>
                        {hasValidTraceId(event.trace_id) ? (
                          <div onClick={(traceEvent) => traceEvent.stopPropagation()}>
                            <Button variant="ghost" size="sm" asChild>
                              <Link to={buildTracePath(event.trace_id ?? "", { from: "evidence" })}>
                                <Waypoints className="size-4" />
                                {shortTraceId(event.trace_id)}
                              </Link>
                            </Button>
                          </div>
                        ) : (
                          <span className="text-muted-foreground">-</span>
                        )}
                      </Td>
                      <Td className="max-w-[520px]">{summarizeEvent(event)}</Td>
                    </tr>
                  ))}
                </tbody>
              </Table>
            </TableWrap>

            <div className="space-y-4">
              <Card>
                <CardHeader>
                  <CardTitle>Current severity mix</CardTitle>
                </CardHeader>
                <CardContent>
                  <SeverityDistribution counts={severityCounts} />
                </CardContent>
              </Card>

              <Card>
                <CardHeader>
                  <CardTitle>Selected event</CardTitle>
                </CardHeader>
                <CardContent className="space-y-3 text-sm">
                  {selectedDetails ? (
                    <>
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge variant={formatRiskTone(formatSeverity(selectedDetails.severity))}>
                          {formatSeverityLabel(selectedDetails.severity)}
                        </Badge>
                        <span className="text-muted-foreground">{formatRelativeDate(selectedDetails.timestamp)}</span>
                        {selectedDetails.service_id ? <Badge variant="outline">{selectedDetails.service_id}</Badge> : null}
                        {selectedTraceId ? <Badge variant="info">{shortTraceId(selectedTraceId)}</Badge> : null}
                      </div>
                      <p className="font-medium">{summarizeEvent(selectedDetails)}</p>
                      <div className="flex flex-wrap gap-2">
                        {selectedTraceId ? (
                          <Button variant="outline" size="sm" asChild>
                            <Link to={buildTracePath(selectedTraceId, { from: "evidence" })}>
                              <Waypoints className="size-4" />
                              Open trace
                            </Link>
                          </Button>
                        ) : null}
                        {selectedDetails.service_id ? (
                          <Button variant="outline" size="sm" asChild>
                            <Link to={`/systems/${encodeURIComponent(selectedDetails.service_id)}`}>Open service</Link>
                          </Button>
                        ) : null}
                      </div>
                      {selectedDetails.tags?.length ? (
                        <div className="flex flex-wrap gap-2">
                          {selectedDetails.tags.map((tag) => (
                            <Badge key={tag} variant="outline">
                              {tag}
                            </Badge>
                          ))}
                        </div>
                      ) : null}
                      <JsonInspector data={selectedDetails} title="Event payload" />
                    </>
                  ) : (
                    <p className="text-muted-foreground">Select an event row to inspect the full payload.</p>
                  )}
                </CardContent>
              </Card>

              <Card>
                <CardHeader>
                  <CardTitle>Service anomaly status</CardTitle>
                </CardHeader>
                <CardContent className="space-y-3 text-sm">
                  {anomaly.data ? (
                    <>
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge variant={formatRiskTone(anomaly.data.status)}>{formatDisplayValue(anomaly.data.status)}</Badge>
                        <span className="text-muted-foreground">{anomaly.data.service_id}</span>
                      </div>
                      <p className="text-muted-foreground">
                        {anomaly.data.error_count} errors across {anomaly.data.event_count} events in the last {anomaly.data.window_hours}h.
                      </p>
                    </>
                  ) : anomaly.errorMessage ? (
                    <p className="text-destructive">{anomaly.errorMessage}</p>
                  ) : (
                    <p className="text-muted-foreground">Filter to a service or select an event to inspect anomaly status.</p>
                  )}
                </CardContent>
              </Card>
            </div>
          </div>
        ) : (
          <EmptyState
            title="No matching events"
            description="Try widening the filters, clearing the trace id, or returning to the newest page."
          />
        )
      ) : null}
    </div>
  );
}

type EvidenceFilters = {
  service: string;
  severity: string;
  sourceType: string;
  query: string;
  traceId: string;
  limit: string;
  start: string;
  end: string;
  cursorTimestamp: string;
  cursorEventId: string;
};

function emptyFilters(): EvidenceFilters {
  return {
    service: "",
    severity: "",
    sourceType: "",
    query: "",
    traceId: "",
    limit: "100",
    start: "",
    end: "",
    cursorTimestamp: "",
    cursorEventId: "",
  };
}

function initialFiltersFromSearch(searchParams: URLSearchParams): EvidenceFilters {
  const defaults = emptyFilters();
  return {
    service: searchParams.get("service") ?? defaults.service,
    severity: searchParams.get("severity") ?? defaults.severity,
    sourceType: searchParams.get("source_type") ?? defaults.sourceType,
    query: searchParams.get("q") ?? searchParams.get("search") ?? defaults.query,
    traceId: normalizeTraceId(searchParams.get("trace_id") ?? defaults.traceId),
    limit: searchParams.get("limit") ?? defaults.limit,
    start: toDateTimeLocal(searchParams.get("start")),
    end: toDateTimeLocal(searchParams.get("end")),
    cursorTimestamp: searchParams.get("cursor_timestamp") ?? defaults.cursorTimestamp,
    cursorEventId: searchParams.get("cursor_event_id") ?? defaults.cursorEventId,
  };
}

function parseLimit(value: string): number {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed)) return 100;
  return Math.max(1, Math.min(2000, parsed));
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

function normalizedTraceIdOrEmpty(value: string): string | undefined {
  const normalized = normalizeTraceId(value);
  return normalized.length === 32 ? normalized : undefined;
}
