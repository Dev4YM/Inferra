import { AlertTriangle, Filter, RadioTower, RefreshCcw, Search } from "lucide-react";
import { useMemo, useState } from "react";

import type { AnomalyStatus, EventDetailResponse, EventRow } from "@/api";
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
import { useApiQuery } from "@/lib/query";
import { SeverityDistribution } from "@/components/inferra/charts";
import { RuntimeStatusCard } from "@/components/inferra/health";

export function EvidencePage({ mode }: { mode: Mode }) {
  const [service, setService] = useState("");
  const [severity, setSeverity] = useState("");
  const [search, setSearch] = useState("");
  const [selectedEventId, setSelectedEventId] = useState<string | null>(null);

  const path = useMemo(() => {
    const params = new URLSearchParams();
    if (service.trim()) params.set("service", service.trim());
    if (severity) params.set("severity", severity);
    if (search.trim()) params.set("search", search.trim());
    params.set("limit", "100");
    return `/api/logs?${params.toString()}`;
  }, [search, service, severity]);

  const logs = useApiQuery<{ logs: EventRow[] }>(path, { deps: [path] });
  const selectedEvent = useApiQuery<EventDetailResponse>(selectedEventId ? `/api/events/${encodeURIComponent(selectedEventId)}` : null, { deps: [selectedEventId] });
  const anomalyService = service.trim() || selectedEvent.data?.event.service_id || "";
  const anomaly = useApiQuery<AnomalyStatus>(
    anomalyService ? `/api/anomaly/${encodeURIComponent(anomalyService)}/status` : null,
    { deps: [anomalyService] },
  );
  const rows = logs.data?.logs ?? [];
  const severityCounts = rows.reduce<Record<string, number>>((acc, event) => {
    const key = formatSeverity(event.severity);
    acc[key] = (acc[key] ?? 0) + 1;
    return acc;
  }, {});
  const servicesSeen = new Set(rows.map((event) => event.service_id).filter(Boolean)).size;

  return (
    <div className="space-y-6">
      <PageHeader
        title="Evidence"
        subtitle="Filter normalized events from the last 24 hours and inspect what actually happened."
        mode={mode}
        actions={
          <Button variant="outline" size="sm" onClick={() => void logs.reload({ silent: true })}>
            <RefreshCcw className={`size-4 ${logs.isRefreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      <div className="dashboard-grid">
        <RuntimeStatusCard icon={RadioTower} label="Stream window" value={String(rows.length)} tone="info" detail="Normalized events currently loaded." />
        <RuntimeStatusCard icon={AlertTriangle} label="Errors" value={String((severityCounts.error ?? 0) + (severityCounts.critical ?? 0))} tone={(severityCounts.error ?? 0) + (severityCounts.critical ?? 0) ? "warning" : "success"} detail="Events at error severity or above." />
        <RuntimeStatusCard icon={Search} label="Services seen" value={String(servicesSeen)} tone="info" detail="Unique services represented by the current filters." />
        <RuntimeStatusCard icon={Filter} label="Filter mode" value={service || severity || search ? "Scoped" : "All"} tone={service || severity || search ? "warning" : "secondary"} detail="Filters apply server-side where supported." />
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Filter className="size-4" />
            Filter evidence
          </CardTitle>
        </CardHeader>
        <CardContent className="grid gap-3 md:grid-cols-[1fr_220px_1fr_auto]">
          <Input
            aria-label="Filter evidence by service id"
            placeholder="service id"
            value={service}
            onChange={(event) => setService(event.target.value)}
          />
          <select
            aria-label="Filter evidence by severity"
            className="h-11 rounded-xl border border-input bg-secondary/50 px-3 text-sm"
            value={severity}
            onChange={(event) => setSeverity(event.target.value)}
          >
            <option value="">any severity</option>
            <option value="1">info+</option>
            <option value="2">warn+</option>
            <option value="3">error+</option>
            <option value="4">critical</option>
          </select>
          <Input
            aria-label="Filter evidence by message text"
            placeholder="message contains…"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
          />
          <Button onClick={() => void logs.reload()}>Apply</Button>
        </CardContent>
      </Card>

      {logs.isLoading && !logs.data ? <LoadingState title="Loading evidence" /> : null}
      {logs.errorMessage && !logs.data ? <ErrorState description={logs.errorMessage} onRetry={() => void logs.reload()} /> : null}

      {logs.data ? (
        logs.data.logs.length ? (
          <div className="grid gap-4 xl:grid-cols-[minmax(0,1.35fr)_minmax(320px,0.9fr)]">
            <TableWrap>
              <Table>
                <thead>
                  <tr>
                    <Th>Timestamp</Th>
                    <Th>Severity</Th>
                    <Th>Service</Th>
                    <Th>Source</Th>
                    <Th>Message</Th>
                  </tr>
                </thead>
                <tbody>
                  {logs.data.logs.map((event, index) => (
                    <tr
                      key={`${event.event_id ?? "event"}-${index}`}
                      className="cursor-pointer transition hover:bg-secondary/50"
                      onClick={() => setSelectedEventId(event.event_id ?? null)}
                    >
                      <Td className="text-muted-foreground">{formatRelativeDate(event.timestamp)}</Td>
                      <Td>
                        <Badge variant={formatRiskTone(formatSeverity(event.severity))}>{formatSeverityLabel(event.severity)}</Badge>
                      </Td>
                      <Td>{event.service_id ?? "unknown"}</Td>
                      <Td className="text-muted-foreground">{formatDisplayValue(event.source_ref?.source_type ?? "unknown")}</Td>
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
                  {selectedEvent.data?.event ? (
                    <>
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge variant={formatRiskTone(formatSeverity(selectedEvent.data.event.severity))}>
                          {formatSeverityLabel(selectedEvent.data.event.severity)}
                        </Badge>
                        <span className="text-muted-foreground">{formatRelativeDate(selectedEvent.data.event.timestamp)}</span>
                      </div>
                      <p className="font-medium">{summarizeEvent(selectedEvent.data.event)}</p>
                      {selectedEvent.data.event.tags?.length ? (
                        <div className="flex flex-wrap gap-2">
                          {selectedEvent.data.event.tags.map((tag) => (
                            <Badge key={tag} variant="outline">
                              {tag}
                            </Badge>
                          ))}
                        </div>
                      ) : null}
                      <JsonInspector data={selectedEvent.data.event} title="Event payload" />
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
          <EmptyState title="No matching events" description="Try widening the filters or waiting for collectors to emit more evidence." />
        )
      ) : null}
    </div>
  );
}
