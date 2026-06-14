import { Activity, AlertTriangle, CheckCircle2, CircleOff, Eye, RefreshCcw, Sparkles, ThumbsDown, Undo2, Waypoints } from "lucide-react";
import { useMemo, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { toast } from "sonner";

import type {
  AdaptiveArtifactKind,
  AdaptiveInfluenceArtifact,
  AdaptiveLearningSummaryResponse,
  AdaptiveReviewDecision,
  AdaptiveRuntimeAction,
  AiGeneration,
  AiGenerationsResponse,
  IncidentDetailResponse,
  IncidentRow,
  EventRow,
  InvestigationResponse,
} from "@/api";
import { reviewAdaptiveArtifact, setAdaptiveArtifactState } from "@/api";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Td, Th, Table, TableWrap } from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { PageHeader } from "@/components/layout/page-header";
import { FilterBar, FilterChip } from "@/components/layout/console-patterns";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { InvestigationView } from "@/components/investigation/investigation-view";
import { AlternativeHypotheses, HypothesisPanel, SuggestedChecks } from "@/components/inferra/incident";
import { IncidentStateTimeline } from "@/components/inferra/timeline";
import { RuntimeStatusCard } from "@/components/inferra/health";
import { TraceSummaryInline } from "@/components/inferra/trace-summary";
import type { Mode } from "@/lib/experience";
import { isAdvancedMode } from "@/lib/experience";
import { formatDisplayValue, formatRiskTone, formatSeverity, formatSeverityLabel, formatRelativeDate, summarizeEvent } from "@/lib/format";
import { buildTracePath } from "@/lib/observability";
import { useApiMutation, useApiQuery } from "@/lib/query";

export function IncidentsPage({ mode }: { mode: Mode }) {
  const incidents = useApiQuery<{ incidents: IncidentRow[] }>("/api/incidents");
  const [stateFilter, setStateFilter] = useState<"all" | "open" | "resolved">("open");

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
  const openRows = rows.filter((incident) => incident.state !== "resolved");
  const resolvedRows = rows.filter((incident) => incident.state === "resolved");
  const visibleRows =
    stateFilter === "open" ? openRows : stateFilter === "resolved" ? resolvedRows : rows;

  return (
    <div className="space-y-5">
      <PageHeader
        title="Incidents"
        subtitle="Open failures, severity, and evidence routes."
        mode={mode}
        actions={
          <Button variant="outline" size="sm" onClick={() => void incidents.reload({ silent: true })}>
            <RefreshCcw className={`size-4 ${incidents.isRefreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      <div className="grid gap-3 sm:grid-cols-3">
        <div className="rounded-md border border-border bg-card px-3 py-2">
          <p className="label-caps">Total</p>
          <p className="font-data text-xl font-semibold">{rows.length}</p>
        </div>
        <div className="rounded-md border border-border bg-card px-3 py-2">
          <p className="label-caps">Open</p>
          <p className="font-data text-xl font-semibold">{openRows.length}</p>
        </div>
        <div className="rounded-md border border-border bg-card px-3 py-2">
          <p className="label-caps">Resolved</p>
          <p className="font-data text-xl font-semibold">{resolvedRows.length}</p>
        </div>
      </div>

      <FilterBar>
        <FilterChip active={stateFilter === "open"} onClick={() => setStateFilter("open")}>
          Open ({openRows.length})
        </FilterChip>
        <FilterChip active={stateFilter === "all"} onClick={() => setStateFilter("all")}>
          All ({rows.length})
        </FilterChip>
        <FilterChip active={stateFilter === "resolved"} onClick={() => setStateFilter("resolved")}>
          Resolved ({resolvedRows.length})
        </FilterChip>
      </FilterBar>

      {visibleRows.length === 0 ? (
        <EmptyState title="No incidents in this view" description="The runtime looks stable for this filter. Keep collectors running or ingest application events." />
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
                <Th>Latest trace</Th>
                <Th>Updated</Th>
              </tr>
            </thead>
            <tbody>
              {visibleRows.map((incident) => (
                <tr
                  key={incident.incident_id}
                  className="transition hover:bg-panel-inset"
                  style={{ borderLeftWidth: 3, borderLeftColor: severityRailColor(incident.severity) }}
                >
                  <Td>
                    <Link className="font-medium" to={`/incidents/${incident.incident_id}`}>
                      {incident.incident_id}
                    </Link>
                  </Td>
                  <Td>{formatDisplayValue(incident.state)}</Td>
                  <Td>
                    <Badge variant={formatRiskTone(formatSeverity(incident.severity))}>{formatSeverityLabel(incident.severity)}</Badge>
                  </Td>
                  <Td>{incident.primary_service || "—"}</Td>
                  <Td>{incident.event_count ?? 0}</Td>
                  <Td>
                    <TraceSummaryInline
                      summary={incident.latest_trace_summary}
                      context={{ from: "incident", incidentId: incident.incident_id }}
                      emptyLabel="—"
                    />
                  </Td>
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
  const [forceInvestigationRun, setForceInvestigationRun] = useState(0);
  const generationScope = incidentId ? `incident:${incidentId}` : null;
  const savedGenerations = useApiQuery<AiGenerationsResponse>(
    generationScope ? `/api/ai/generations?scope=${encodeURIComponent(generationScope)}&limit=1` : null,
    { deps: [generationScope], staleTime: 5_000 },
  );
  const savedInvestigation = savedGenerations.data?.generations?.[0]
    ? hydrateIncidentSavedGeneration(savedGenerations.data.generations[0])
    : null;
  const savedLookupDone = Boolean(savedGenerations.data || savedGenerations.error);
  const shouldRunInvestigation = Boolean(
    incidentId && (forceInvestigationRun > 0 || (savedLookupDone && !savedInvestigation)),
  );
  const detail = useApiQuery<IncidentDetailResponse>(incidentId ? `/api/incidents/${encodeURIComponent(incidentId)}` : null, { deps: [incidentId] });
  const investigation = useApiQuery<InvestigationResponse>(
    shouldRunInvestigation && incidentId
      ? `/api/investigate/incident/${encodeURIComponent(incidentId)}?mode=${mode}${
          forceInvestigationRun ? `&force=true&run=${forceInvestigationRun}` : ""
        }`
      : null,
    { deps: [incidentId, mode, forceInvestigationRun] },
  );
  const adaptive = useApiQuery<AdaptiveLearningSummaryResponse>(
    detail.data?.learning_provenance?.artifacts?.length ? "/api/learning/adaptive" : null,
    { deps: [incidentId, detail.data?.learning_provenance?.artifacts?.length ?? 0] },
  );
  const reviewMutation = useApiMutation(
    async (args: { kind: AdaptiveArtifactKind; artifactId: string; decision: AdaptiveReviewDecision; reason?: string }) =>
      reviewAdaptiveArtifact(args.kind, args.artifactId, { decision: args.decision, reason: args.reason }),
  );
  const stateMutation = useApiMutation(
    async (args: { kind: AdaptiveArtifactKind; artifactId: string; action: AdaptiveRuntimeAction; reason?: string }) =>
      setAdaptiveArtifactState(args.kind, args.artifactId, { action: args.action, reason: args.reason }),
  );
  const [artifactReasons, setArtifactReasons] = useState<Record<string, string>>({});
  const adaptiveArtifacts = useMemo(
    () => resolveIncidentAdaptiveArtifacts(detail.data?.learning_provenance?.artifacts ?? [], adaptive.data),
    [detail.data?.learning_provenance?.artifacts, adaptive.data],
  );
  const busy = reviewMutation.isPending || stateMutation.isPending;
  const incidentEvents = useMemo(
    () => dedupeIncidentEvents(detail.data?.events ?? []),
    [detail.data?.events],
  );
  const displayedInvestigation = investigation.data ?? savedInvestigation;
  const investigationRunning =
    shouldRunInvestigation && !displayedInvestigation && (investigation.isLoading || !savedLookupDone);
  const suggestedChecks = useMemo(() => {
    const fromInvestigation = displayedInvestigation?.output.next_steps ?? [];
    if (fromInvestigation.length) return fromInvestigation;
    const fromHypotheses = detail.data?.hypotheses.flatMap((hypothesis) => hypothesis.suggested_checks ?? []) ?? [];
    return [...new Set(fromHypotheses)].map((check) => ({ title: check, reason: check }));
  }, [detail.data?.hypotheses, displayedInvestigation]);

  if (!incidentId) return <EmptyState title="Missing incident id" description="Select an incident from the list first." />;
  if (detail.isLoading && !detail.data) return <LoadingState title="Loading incident" />;
  if (detail.errorMessage && !detail.data) return <ErrorState description={detail.errorMessage} onRetry={() => void detail.reload()} />;
  if (!detail.data) return <EmptyState title="No incident data" description="Inferra could not load the incident details." />;

  const incident = detail.data.incident;
  const investigationMissing = investigation.error?.status === 404;

  const runArtifactReview = async (artifact: IncidentAdaptiveArtifact, decision: AdaptiveReviewDecision) => {
    if (!artifact.actionKind) return;
    const reason = artifactReasons[artifact.key]?.trim();
    try {
      await reviewMutation.run({
        kind: artifact.actionKind,
        artifactId: artifact.artifactId,
        decision,
        reason: reason || undefined,
      });
      toast.success("Adaptive review updated", {
        description: `${artifact.label} is now ${decision}.`,
      });
      await Promise.all([
        adaptive.reload({ silent: true }),
        detail.reload({ silent: true }),
      ]);
    } catch (error) {
      toast.error("Could not update adaptive review", {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const runArtifactState = async (artifact: IncidentAdaptiveArtifact, action: AdaptiveRuntimeAction) => {
    if (!artifact.actionKind) return;
    const reason = artifactReasons[artifact.key]?.trim();
    try {
      await stateMutation.run({
        kind: artifact.actionKind,
        artifactId: artifact.artifactId,
        action,
        reason: reason || undefined,
      });
      toast.success("Adaptive runtime state updated", {
        description: `${artifact.label} was ${action}d.`,
      });
      await Promise.all([
        adaptive.reload({ silent: true }),
        detail.reload({ silent: true }),
        investigation.reload({ silent: true }),
      ]);
    } catch (error) {
      toast.error(`Could not ${action} artifact`, {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  return (
    <div className="space-y-6">
      <PageHeader
        title={`Incident ${incident.incident_id}`}
        subtitle={`${incident.primary_service || "Unknown"} · Severity ${incident.severity} · ${formatDisplayValue(incident.state)}`}
        mode={mode}
        actions={
          <div className="flex flex-wrap gap-2">
            <Button variant="outline" size="sm" asChild>
              <Link to={`/ai?incident=${encodeURIComponent(incident.incident_id)}`}>
                <Sparkles className="size-4" />
                AI scope
              </Link>
            </Button>
            <Button variant="outline" size="sm" onClick={() => { void detail.reload({ silent: true }); setForceInvestigationRun((value) => value + 1); }}>
              <RefreshCcw className={`size-4 ${detail.isRefreshing || investigation.isRefreshing ? "animate-spin" : ""}`} />
              Refresh
            </Button>
          </div>
        }
      />

      <div className="dashboard-grid">
        <RuntimeStatusCard
          icon={AlertTriangle}
          label="Severity"
          value={`sev ${incident.severity}`}
          tone={incident.severity >= 4 ? "destructive" : incident.severity >= 3 ? "warning" : "info"}
          detail={incident.primary_service || "No primary service recorded."}
        />
        <RuntimeStatusCard
          icon={Activity}
          label="Correlated events"
          value={String(incidentEvents.length)}
          tone={incidentEvents.length ? "info" : "secondary"}
          detail="Evidence currently attached to this incident."
        />
        <RuntimeStatusCard
          icon={Sparkles}
          label="Hypotheses"
          value={String(detail.data.hypotheses.length)}
          tone={detail.data.hypotheses.length ? "success" : "secondary"}
          detail="Ranked explanations generated from observed evidence."
        />
        <RuntimeStatusCard
          icon={Eye}
          label="Affected services"
          value={String(incident.affected_services?.length ?? (incident.primary_service ? 1 : 0))}
          tone="info"
          detail={(incident.affected_services ?? [incident.primary_service]).filter(Boolean).slice(0, 3).join(", ") || "Unknown scope"}
        />
      </div>

      <div className="content-grid">
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Ranked hypotheses</CardTitle>
              <CardDescription>
                Compare likely causes by confidence, supporting evidence, contradictions, and suggested checks.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {detail.data.hypotheses.length ? (
                detail.data.hypotheses.map((hypothesis, index) => (
                  <HypothesisPanel
                    key={hypothesis.hypothesis_id}
                    hypothesis={hypothesis}
                    rank={index + 1}
                    events={incidentEvents}
                    defaultOpen={index === 0}
                    advanced={isAdvancedMode(mode)}
                  />
                ))
              ) : (
                <p className="text-sm text-muted-foreground">No hypotheses recorded yet.</p>
              )}
            </CardContent>
          </Card>

          {displayedInvestigation ? (
            <InvestigationView result={displayedInvestigation} showRaw={isAdvancedMode(mode)} onRefresh={() => setForceInvestigationRun((value) => value + 1)} />
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
          ) : savedGenerations.errorMessage ? (
            <ErrorState description={`Saved investigation unavailable: ${savedGenerations.errorMessage}`} onRetry={() => void savedGenerations.reload()} />
          ) : savedGenerations.isLoading ? (
            <LoadingState title="Loading saved investigation" />
          ) : investigationRunning ? (
            <LoadingState
              title="Running investigation"
              description="Inferra is correlating evidence and asking the AI investigator for a structured summary. Saved runs load automatically when available."
            />
          ) : (
            <EmptyState
              title="Investigation not started"
              description="Refresh to run an investigation for this incident, or open AI Investigator with this scope prefilled."
              action={
                <Button asChild>
                  <Link to={`/ai?incident=${encodeURIComponent(incident.incident_id)}`}>Open AI Investigator</Link>
                </Button>
              }
            />
          )}
        </div>

        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Suggested checks</CardTitle>
            </CardHeader>
            <CardContent>
              <SuggestedChecks checks={suggestedChecks} />
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Alternative hypotheses</CardTitle>
            </CardHeader>
            <CardContent>
              <AlternativeHypotheses items={detail.data.explanation?.alternatives ?? investigation.data?.output.uncertainty ?? []} />
            </CardContent>
          </Card>

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
                    <div key={String(data?.cluster_id ?? index)} className="rounded-md border border-border bg-panel-inset p-4">
                      <div className="flex flex-wrap items-center gap-2">
                        <p className="font-medium">{String(data?.cluster_id ?? `cluster-${index + 1}`)}</p>
                        {data?.source_type ? <Badge variant="outline">{formatDisplayValue(String(data.source_type))}</Badge> : null}
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
                    <Badge variant="outline">{formatDisplayValue(detail.data.explanation.quality)}</Badge>
                    <Badge variant="outline">{detail.data.explanation.model_used}</Badge>
                    <span className="text-muted-foreground">{formatRelativeDate(detail.data.explanation.created_at)}</span>
                  </div>
                  <p className="font-medium">{detail.data.explanation.summary}</p>
                  {detail.data.explanation.evidence_text ? (
                    <pre className="overflow-auto rounded-xl border border-border bg-background/70 p-3 text-xs text-primary">
                      <code>{detail.data.explanation.evidence_text}</code>
                    </pre>
                  ) : null}
                  {detail.data.explanation.timeline_text ? (
                    <pre className="overflow-auto rounded-xl border border-border bg-background/70 p-3 text-xs text-primary">
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
              <CardTitle>Adaptive influence</CardTitle>
              <CardDescription>
                Review the learned artifacts that are currently shaping this incident without leaving the incident workflow.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              {detail.data.learning_provenance ? (
                <>
                  <div className="grid gap-3 md:grid-cols-3">
                    <StatusStat
                      label="Influenced hypotheses"
                      value={String(detail.data.learning_provenance.influenced_hypotheses)}
                    />
                    <StatusStat
                      label="Estimated learned impact"
                      value={detail.data.learning_provenance.estimated_total_impact.toFixed(2)}
                    />
                    <StatusStat
                      label="Influencing artifacts"
                      value={String(detail.data.learning_provenance.artifacts.length)}
                    />
                  </div>

                  {adaptive.errorMessage ? (
                    <Alert variant="warning">
                      <div className="min-w-0">
                        <AlertTitle>Live artifact state unavailable</AlertTitle>
                        <AlertDescription>
                          {adaptive.errorMessage}. Provenance is still visible, but inline review actions may be incomplete until the adaptive registry reloads.
                        </AlertDescription>
                      </div>
                    </Alert>
                  ) : null}

                  {adaptiveArtifacts.length ? (
                    adaptiveArtifacts.map((artifact) => (
                      <div key={artifact.key} className="rounded-md border border-border bg-panel-inset p-4">
                        <div className="flex flex-wrap items-start justify-between gap-3">
                          <div>
                            <p className="font-medium">{artifact.label}</p>
                            <p className="text-sm text-muted-foreground">
                              {humanizeAdaptiveKind(artifact.kind)} · {artifact.artifactId}
                            </p>
                          </div>
                          <div className="flex flex-wrap gap-2">
                            <Badge variant={adaptiveStatusVariant(artifact.status)}>{formatDisplayValue(artifact.status)}</Badge>
                            <Badge variant={adaptiveReviewVariant(artifact.reviewStatus)}>{formatDisplayValue(artifact.reviewStatus)}</Badge>
                            <Badge variant="outline">
                              {artifact.impactMetric === "matched_events" ? "matched events" : artifact.impactMetric || "impact"}{" "}
                              {artifact.impactValue.toFixed(2)}
                            </Badge>
                          </div>
                        </div>

                        <div className="mt-3 grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                          <StatusStat label="Confirmations" value={String(artifact.confirmations)} />
                          <StatusStat label="False positives" value={String(artifact.falsePositives)} />
                          <StatusStat label="Updated" value={formatRelativeDate(artifact.updatedAt)} />
                          <StatusStat label="Last reviewed" value={formatRelativeDate(artifact.lastReviewedAt)} />
                        </div>

                        {artifact.reason ? <p className="mt-3 text-sm text-muted-foreground">{artifact.reason}</p> : null}
                        {artifact.reviewReason ? (
                          <p className="mt-2 text-sm text-muted-foreground">Review note: {artifact.reviewReason}</p>
                        ) : null}
                        {artifact.statusReason ? (
                          <p className="mt-1 text-sm text-muted-foreground">Runtime note: {artifact.statusReason}</p>
                        ) : null}

                        {artifact.actionKind ? (
                          <>
                            <Textarea
                              value={artifactReasons[artifact.key] ?? ""}
                              onChange={(event) =>
                                setArtifactReasons((current) => ({
                                  ...current,
                                  [artifact.key]: event.target.value,
                                }))
                              }
                              className="mt-4 min-h-[88px]"
                              placeholder="Optional operator rationale for this review decision."
                            />
                            <div className="mt-4 flex flex-wrap gap-3">
                              <Button onClick={() => void runArtifactReview(artifact, "approve")} disabled={busy}>
                                <CheckCircle2 className="size-4" />
                                Approve
                              </Button>
                              <Button variant="outline" onClick={() => void runArtifactReview(artifact, "watch")} disabled={busy}>
                                <Eye className="size-4" />
                                Watch
                              </Button>
                              <Button variant="destructive" onClick={() => void runArtifactReview(artifact, "reject")} disabled={busy}>
                                <ThumbsDown className="size-4" />
                                Reject
                              </Button>
                              <Button variant="outline" onClick={() => void runArtifactReview(artifact, "reset")} disabled={busy}>
                                <Undo2 className="size-4" />
                                Reset
                              </Button>
                              <Button
                                variant="outline"
                                onClick={() => void runArtifactState(artifact, "enable")}
                                disabled={busy || !artifact.manuallyDisabled}
                              >
                                <Activity className="size-4" />
                                Enable
                              </Button>
                              <Button
                                variant="destructive"
                                onClick={() => void runArtifactState(artifact, "disable")}
                                disabled={busy || artifact.manuallyDisabled}
                              >
                                <CircleOff className="size-4" />
                                Disable
                              </Button>
                            </div>
                          </>
                        ) : (
                          <Alert className="mt-4" variant="warning">
                            <div className="min-w-0">
                              <AlertTitle>Artifact no longer in the active registry</AlertTitle>
                              <AlertDescription>
                                This incident still carries historical learned influence, but the current adaptive registry no longer exposes an actionable artifact entry.
                              </AlertDescription>
                            </div>
                          </Alert>
                        )}
                      </div>
                    ))
                  ) : (
                    <p className="text-sm text-muted-foreground">
                      This incident has learned influence, but no actionable adaptive artifact entries were resolved from the current registry.
                    </p>
                  )}

                  <div className="flex flex-wrap gap-3">
                    <Button variant="outline" size="sm" asChild>
                      <Link to="/learning">Open full learning review</Link>
                    </Button>
                    <Button variant="outline" size="sm" onClick={() => void adaptive.reload({ silent: true })}>
                      <RefreshCcw className={`size-4 ${adaptive.isRefreshing ? "animate-spin" : ""}`} />
                      Refresh artifact state
                    </Button>
                  </div>
                </>
              ) : (
                <p className="text-sm text-muted-foreground">
                  No learned artifacts are currently influencing the stored hypotheses for this incident.
                </p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Correlated events</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              {incidentEvents.length ? (
                incidentEvents.slice(0, 12).map((event) => (
                  <div key={event.event_id} className="rounded-md border border-border bg-panel-inset p-4">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge variant={formatRiskTone(formatSeverity(event.severity))}>{formatSeverityLabel(event.severity)}</Badge>
                        {event.service_id ? <Badge variant="outline">{event.service_id}</Badge> : null}
                        {event.trace_id ? (
                          <Badge variant="outline" className="font-mono text-[10px]">
                            {shortTraceId(event.trace_id)}
                          </Badge>
                        ) : null}
                      </div>
                      <span className="text-xs text-muted-foreground">{formatRelativeDate(event.timestamp)}</span>
                    </div>
                    <p className="mt-2 text-sm">{summarizeEvent(event)}</p>
                    {event.trace_id ? (
                      <div className="mt-3">
                        <Button variant="outline" size="sm" asChild>
                          <Link to={buildTracePath(event.trace_id, { from: "incident", incidentId })}>
                            <Waypoints className="size-4" />
                            Open trace
                          </Link>
                        </Button>
                      </div>
                    ) : null}
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
              <IncidentStateTimeline states={detail.data.state_log} />
              {detail.data.latest_trace ? (
                <div className="rounded-md border border-border bg-panel-inset p-4">
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
                  <div key={`${entry.changed_at ?? "audit"}-${index}`} className="rounded-md border border-border bg-panel-inset p-4">
                    <div className="flex flex-wrap items-center gap-2">
                      <Badge variant="outline">{formatDisplayValue(entry.old_state ?? "?")}</Badge>
                      <span className="text-muted-foreground">to</span>
                      <Badge variant="outline">{formatDisplayValue(entry.new_state ?? "?")}</Badge>
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

function hydrateIncidentSavedGeneration(generation: AiGeneration): InvestigationResponse {
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

type IncidentAdaptiveArtifact = {
  key: string;
  kind: string;
  artifactId: string;
  label: string;
  impactMetric?: string | null;
  impactValue: number;
  reason?: string | null;
  status: string;
  reviewStatus: string;
  reviewReason?: string | null;
  statusReason?: string | null;
  confirmations: number;
  falsePositives: number;
  updatedAt?: string | null;
  lastReviewedAt?: string | null;
  manuallyDisabled: boolean;
  actionKind: AdaptiveArtifactKind | null;
};

function resolveIncidentAdaptiveArtifacts(
  artifacts: AdaptiveInfluenceArtifact[],
  adaptive: AdaptiveLearningSummaryResponse | null,
): IncidentAdaptiveArtifact[] {
  const index = buildAdaptiveArtifactIndex(adaptive);
  return artifacts
    .map((artifact, itemIndex) => {
      const kind = artifact.kind ?? "unknown";
      const artifactId = artifact.artifact_id ?? `artifact-${itemIndex + 1}`;
      const key = `${kind}:${artifactId}`;
      const snapshot = index.get(key);
      return {
        key,
        kind,
        artifactId,
        label: artifact.label ?? artifactId,
        impactMetric: artifact.impact_metric,
        impactValue: artifact.cumulative_impact ?? artifact.impact_value ?? 0,
        reason: artifact.reason,
        status: snapshot?.status ?? "unknown",
        reviewStatus: snapshot?.reviewStatus ?? "unreviewed",
        reviewReason: snapshot?.reviewReason,
        statusReason: snapshot?.statusReason,
        confirmations: snapshot?.confirmations ?? 0,
        falsePositives: snapshot?.falsePositives ?? 0,
        updatedAt: snapshot?.updatedAt,
        lastReviewedAt: snapshot?.lastReviewedAt,
        manuallyDisabled: snapshot?.manuallyDisabled ?? false,
        actionKind: snapshot?.actionKind ?? normalizeAdaptiveArtifactKind(kind),
      };
    })
    .sort((left, right) => right.impactValue - left.impactValue || left.label.localeCompare(right.label));
}

function buildAdaptiveArtifactIndex(adaptive: AdaptiveLearningSummaryResponse | null) {
  const index = new Map<string, Omit<IncidentAdaptiveArtifact, "key" | "kind" | "artifactId" | "label" | "impactMetric" | "impactValue" | "reason">>();
  if (!adaptive) return index;

  for (const detector of adaptive.detectors) {
    index.set(`detector:${detector.detector_id}`, {
      status: detector.status ?? "unknown",
      reviewStatus: detector.review_status ?? "unreviewed",
      reviewReason: detector.review_reason,
      statusReason: detector.status_reason,
      confirmations: detector.confirmations,
      falsePositives: detector.false_positives,
      updatedAt: detector.updated_at,
      lastReviewedAt: detector.last_reviewed_at,
      manuallyDisabled: detector.manually_disabled,
      actionKind: "detector",
    });
  }
  for (const template of adaptive.templates) {
    index.set(`template:${template.template_id}`, {
      status: template.status ?? "unknown",
      reviewStatus: template.review_status ?? "unreviewed",
      reviewReason: template.review_reason,
      statusReason: template.status_reason,
      confirmations: template.confirmations,
      falsePositives: template.false_positives,
      updatedAt: template.updated_at,
      lastReviewedAt: template.last_reviewed_at,
      manuallyDisabled: template.manually_disabled,
      actionKind: "template",
    });
  }
  for (const composition of adaptive.compositions) {
    index.set(`composition:${composition.composition_id}`, {
      status: composition.status ?? "unknown",
      reviewStatus: composition.review_status ?? "unreviewed",
      reviewReason: composition.review_reason,
      statusReason: composition.status_reason,
      confirmations: composition.confirmations,
      falsePositives: composition.false_positives,
      updatedAt: composition.updated_at,
      lastReviewedAt: composition.last_reviewed_at,
      manuallyDisabled: composition.manually_disabled,
      actionKind: "composition",
    });
  }
  for (const profile of adaptive.edge_profiles) {
    index.set(`edge_profile:${profile.profile_id}`, {
      status: profile.status ?? "unknown",
      reviewStatus: profile.review_status ?? "unreviewed",
      reviewReason: profile.review_reason,
      statusReason: profile.status_reason,
      confirmations: profile.confirmations,
      falsePositives: profile.false_positives,
      updatedAt: profile.updated_at,
      lastReviewedAt: profile.last_reviewed_at,
      manuallyDisabled: profile.manually_disabled,
      actionKind: "edge_profile",
    });
  }

  return index;
}

function normalizeAdaptiveArtifactKind(value: string): AdaptiveArtifactKind | null {
  switch (value) {
    case "detector":
    case "template":
    case "composition":
    case "edge_profile":
      return value;
    default:
      return null;
  }
}

function adaptiveStatusVariant(value: string): "success" | "warning" | "destructive" | "secondary" {
  switch (value) {
    case "active":
      return "success";
    case "suppressed":
      return "warning";
    case "manually_disabled":
      return "destructive";
    default:
      return "secondary";
  }
}

function adaptiveReviewVariant(value: string): "success" | "warning" | "destructive" | "secondary" {
  switch (value) {
    case "approved":
      return "success";
    case "watch":
      return "warning";
    case "rejected":
      return "destructive";
    default:
      return "secondary";
  }
}

function humanizeAdaptiveKind(value: string): string {
  return value.replace(/_/g, " ");
}

function StatusStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border border-border bg-panel-inset px-4 py-3">
      <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">{label}</p>
      <p className="mt-2 text-sm font-medium">{value}</p>
    </div>
  );
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function arrayOfStrings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

function severityRailColor(severity: number): string {
  if (severity >= 4) return "var(--critical)";
  if (severity >= 3) return "var(--warning)";
  if (severity >= 2) return "var(--accent)";
  return "var(--border)";
}

function shortTraceId(traceId: string): string {
  if (traceId.length <= 16) return traceId;
  return `${traceId.slice(0, 8)}...${traceId.slice(-8)}`;
}

function dedupeIncidentEvents(events: EventRow[]): EventRow[] {
  const seen = new Set<string>();
  return events.filter((event) => {
    const key =
      event.event_id ??
      `${event.timestamp ?? ""}:${event.service_id ?? ""}:${event.message ?? summarizeEvent(event)}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}
