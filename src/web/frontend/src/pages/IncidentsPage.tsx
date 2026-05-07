import { RefreshCcw } from "lucide-react";
import { Link, useParams } from "react-router-dom";

import type { IncidentDetailResponse, IncidentRow, InvestigationResponse } from "@/api";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Td, Th, Table, TableWrap } from "@/components/ui/table";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { InvestigationView } from "@/components/investigation/investigation-view";
import type { Mode } from "@/lib/experience";
import { isAdvancedMode } from "@/lib/experience";
import { formatRiskTone, formatSeverity, formatRelativeDate, summarizeEvent } from "@/lib/format";
import { useApiQuery } from "@/lib/query";

export function IncidentsPage({ mode }: { mode: Mode }) {
  const incidents = useApiQuery<{ incidents: IncidentRow[] }>("/api/incidents");

  if (incidents.isLoading && !incidents.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Incidents" subtitle="Active investigations and their latest evidence." mode={mode} />
        <LoadingState title="Loading incidents" />
      </div>
    );
  }

  if (incidents.errorMessage && !incidents.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Incidents" subtitle="Active investigations and their latest evidence." mode={mode} />
        <ErrorState description={incidents.errorMessage} onRetry={() => void incidents.reload()} />
      </div>
    );
  }

  const rows = incidents.data?.incidents ?? [];

  return (
    <div className="space-y-6">
      <PageHeader
        title="Incidents"
        subtitle="Active failures, their severity, and the latest evidence-backed routes."
        mode={mode}
        actions={
          <Button variant="outline" size="sm" onClick={() => void incidents.reload({ silent: true })}>
            <RefreshCcw className={`size-4 ${incidents.isRefreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      {rows.length === 0 ? (
        <EmptyState title="No active incidents" description="The current runtime looks stable. Seed demo data or wait for collectors to observe problems." />
      ) : (
        <TableWrap>
          <Table>
            <thead>
              <tr>
                <Th>ID</Th>
                <Th>State</Th>
                <Th>Severity</Th>
                <Th>Primary service</Th>
                <Th>Events</Th>
                <Th>Updated</Th>
              </tr>
            </thead>
            <tbody>
              {rows.map((incident) => (
                <tr key={incident.incident_id} className="transition hover:bg-secondary/50">
                  <Td>
                    <Link className="font-medium" to={`/incidents/${incident.incident_id}`}>
                      {incident.incident_id}
                    </Link>
                  </Td>
                  <Td>{incident.state}</Td>
                  <Td>
                    <Badge variant={formatRiskTone(formatSeverity(incident.severity))}>{formatSeverity(incident.severity)}</Badge>
                  </Td>
                  <Td>{incident.primary_service || "—"}</Td>
                  <Td>{incident.event_count ?? 0}</Td>
                  <Td className="text-muted-foreground">{formatRelativeDate(incident.updated_at)}</Td>
                </tr>
              ))}
            </tbody>
          </Table>
        </TableWrap>
      )}
    </div>
  );
}

export function IncidentDetailPage({ mode }: { mode: Mode }) {
  const { incidentId } = useParams();
  const detail = useApiQuery<IncidentDetailResponse>(incidentId ? `/api/incidents/${incidentId}` : null, { deps: [incidentId] });
  const investigation = useApiQuery<InvestigationResponse>(
    incidentId ? `/api/investigate/incident/${incidentId}?mode=${mode}` : null,
    { deps: [incidentId, mode] },
  );

  if (!incidentId) return <EmptyState title="Missing incident id" description="Select an incident from the list first." />;
  if (detail.isLoading && !detail.data) return <LoadingState title="Loading incident" />;
  if (detail.errorMessage && !detail.data) return <ErrorState description={detail.errorMessage} onRetry={() => void detail.reload()} />;
  if (!detail.data) return <EmptyState title="No incident data" description="Inferra could not load the incident details." />;

  const incident = detail.data.incident;
  const investigationMissing = investigation.error?.status === 404;

  return (
    <div className="space-y-6">
      <PageHeader
        title={`Incident ${incident.incident_id}`}
        subtitle={`${incident.primary_service || "unknown"} · severity ${incident.severity} · ${incident.state}`}
        mode={mode}
        actions={
          <Button variant="outline" size="sm" onClick={() => { void detail.reload({ silent: true }); void investigation.reload({ silent: true }); }}>
            <RefreshCcw className={`size-4 ${detail.isRefreshing || investigation.isRefreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      <div className="content-grid">
        <div className="space-y-4">
          {investigation.data ? (
            <InvestigationView result={investigation.data} showRaw={isAdvancedMode(mode)} onRefresh={() => void investigation.reload({ silent: true })} />
          ) : investigation.errorMessage ? (
            investigationMissing ? (
              <EmptyState
                title="Investigation not available"
                description="This incident no longer has enough current evidence to generate an investigation bundle."
                action={<Button onClick={() => void investigation.reload()}>Retry investigation</Button>}
              />
            ) : (
              <ErrorState description={`Investigation unavailable: ${investigation.errorMessage}`} onRetry={() => void investigation.reload()} />
            )
          ) : (
            <LoadingState title="Running investigation" description="Inferra is collecting evidence and asking the AI investigator for a structured summary." />
          )}
        </div>

        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Correlation clusters</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {detail.data.clusters.length ? (
                detail.data.clusters.map((cluster, index) => {
                  const data = asRecord(cluster);
                  const sourceTypes = arrayOfStrings(data?.source_types);
                  const affectedServices = arrayOfStrings(data?.affected_services);
                  const messages = arrayOfStrings(data?.top_messages);
                  return (
                    <div key={String(data?.cluster_id ?? index)} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                      <div className="flex flex-wrap items-center gap-2">
                        <p className="font-medium">{String(data?.cluster_id ?? `cluster-${index + 1}`)}</p>
                        {data?.source_type ? <Badge variant="outline">{String(data.source_type)}</Badge> : null}
                        {typeof data?.event_count === "number" ? <Badge variant="outline">{data.event_count} events</Badge> : null}
                      </div>
                      {affectedServices.length ? (
                        <p className="mt-2 text-sm text-muted-foreground">Services: {affectedServices.join(", ")}</p>
                      ) : null}
                      {sourceTypes.length ? (
                        <p className="mt-1 text-sm text-muted-foreground">Sources: {sourceTypes.join(", ")}</p>
                      ) : null}
                      {messages.length ? (
                        <div className="mt-3 space-y-1 text-sm text-muted-foreground">
                          {messages.slice(0, 3).map((message, messageIndex) => (
                            <p key={messageIndex}>• {message}</p>
                          ))}
                        </div>
                      ) : null}
                    </div>
                  );
                })
              ) : (
                <p className="text-sm text-muted-foreground">No cluster payloads recorded for this incident yet.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Persisted explanation</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3 text-sm">
              {detail.data.explanation ? (
                <>
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant="outline">{detail.data.explanation.quality}</Badge>
                    <Badge variant="outline">{detail.data.explanation.model_used}</Badge>
                    <span className="text-muted-foreground">{formatRelativeDate(detail.data.explanation.created_at)}</span>
                  </div>
                  <p className="font-medium">{detail.data.explanation.summary}</p>
                  {detail.data.explanation.evidence_text ? (
                    <pre className="overflow-auto rounded-xl border border-border/70 bg-background/70 p-3 text-xs text-primary">
                      <code>{detail.data.explanation.evidence_text}</code>
                    </pre>
                  ) : null}
                  {detail.data.explanation.timeline_text ? (
                    <pre className="overflow-auto rounded-xl border border-border/70 bg-background/70 p-3 text-xs text-primary">
                      <code>{detail.data.explanation.timeline_text}</code>
                    </pre>
                  ) : null}
                </>
              ) : (
                <p className="text-muted-foreground">No persisted explanation is stored for this incident yet.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Hypotheses</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {detail.data.hypotheses.length ? (
                detail.data.hypotheses.map((hypothesis) => (
                  <div key={hypothesis.hypothesis_id} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                    <div className="flex flex-wrap items-center gap-2">
                      <p className="font-medium">{hypothesis.cause_type}</p>
                      {hypothesis.confidence_label ? <Badge variant="outline">{hypothesis.confidence_label}</Badge> : null}
                    </div>
                    {hypothesis.description ? <p className="mt-2 text-sm text-muted-foreground">{hypothesis.description}</p> : null}
                  </div>
                ))
              ) : (
                <p className="text-sm text-muted-foreground">No hypotheses recorded yet.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Correlated events</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {detail.data.events.length ? (
                detail.data.events.slice(0, 12).map((event) => (
                  <div key={event.event_id} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <Badge variant={formatRiskTone(formatSeverity(event.severity))}>{formatSeverity(event.severity)}</Badge>
                      <span className="text-xs text-muted-foreground">{formatRelativeDate(event.timestamp)}</span>
                    </div>
                    <p className="mt-2 text-sm">{summarizeEvent(event)}</p>
                  </div>
                ))
              ) : (
                <p className="text-sm text-muted-foreground">No events attached to this incident.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Incident audit</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3 text-sm">
              {detail.data.latest_trace ? (
                <div className="rounded-2xl border border-border/60 bg-background/30 p-4">
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant="outline">{detail.data.latest_trace.trace_kind}</Badge>
                    <span className="text-muted-foreground">{formatRelativeDate(detail.data.latest_trace.created_at)}</span>
                  </div>
                  <p className="mt-2 text-muted-foreground">
                    Allowed fields: {detail.data.latest_trace.allowed_fields.join(", ") || "none"}
                  </p>
                </div>
              ) : null}
              {detail.data.state_log?.length ? (
                detail.data.state_log.slice(0, 5).map((entry, index) => (
                  <div key={`${entry.changed_at ?? "audit"}-${index}`} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge variant="outline">{entry.old_state ?? "?"}</Badge>
                      <span className="text-muted-foreground">to</span>
                      <Badge variant="outline">{entry.new_state ?? "?"}</Badge>
                    </div>
                    {entry.reason ? <p className="mt-2 text-muted-foreground">{entry.reason}</p> : null}
                    <p className="mt-1 text-xs text-muted-foreground">{formatRelativeDate(entry.changed_at)}</p>
                  </div>
                ))
              ) : (
                <p className="text-muted-foreground">No state transitions recorded yet.</p>
              )}
            </CardContent>
          </Card>
        </div>
      </div>
    </div>
  );
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function arrayOfStrings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

