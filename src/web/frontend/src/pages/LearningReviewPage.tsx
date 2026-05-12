import {
  Activity,
  CheckCircle2,
  CircleOff,
  Clock3,
  Eye,
  RefreshCcw,
  Search,
  ThumbsDown,
  Undo2,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { toast } from "sonner";

import type {
  AdaptiveArtifactKind,
  AdaptiveComparisonRow,
  AdaptiveComposition,
  AdaptiveDetector,
  AdaptiveEdgeProfile,
  AdaptiveLearningReviewResponse,
  AdaptiveReviewDecision,
  AdaptiveRuntimeAction,
  AdaptiveSavedReviewView,
  AdaptiveTemplate,
  AdaptiveTrendDrilldown,
} from "@/api";
import {
  deleteAdaptiveReviewView,
  reviewAdaptiveArtifact,
  reviewAdaptiveArtifactsBulk,
  saveAdaptiveReviewView,
  setAdaptiveArtifactState,
  setAdaptiveArtifactsStateBulk,
  useAdaptiveReviewView,
} from "@/api";
import { PageHeader } from "@/components/layout/page-header";
import { EmptyState, ErrorState, LoadingState } from "@/components/feedback/states";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { JsonInspector } from "@/components/ui/json-inspector";
import { Table, TableWrap, Td, Th } from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import type { Mode } from "@/lib/experience";
import { formatDisplayValue, formatRelativeDate } from "@/lib/format";
import { useApiMutation, useApiQuery } from "@/lib/query";

type ArtifactDetail = AdaptiveDetector | AdaptiveTemplate | AdaptiveComposition | AdaptiveEdgeProfile;

type ArtifactRecord = {
  key: string;
  kind: AdaptiveArtifactKind;
  artifactId: string;
  label: string;
  status: string;
  reviewStatus: string;
  reviewReason?: string | null;
  statusReason?: string | null;
  lastReviewedAt?: string | null;
  updatedAt?: string | null;
  confirmations: number;
  falsePositives: number;
  manuallyDisabled: boolean;
  detail: ArtifactDetail;
};

export function LearningReviewPage({ mode }: { mode: Mode }) {
  const review = useApiQuery<AdaptiveLearningReviewResponse>("/api/learning/adaptive/review");
  const reviewMutation = useApiMutation(
    async (args: { kind: string; artifactId: string; decision: AdaptiveReviewDecision; reason?: string }) =>
      reviewAdaptiveArtifact(args.kind, args.artifactId, { decision: args.decision, reason: args.reason }),
  );
  const stateMutation = useApiMutation(
    async (args: { kind: string; artifactId: string; action: AdaptiveRuntimeAction; reason?: string }) =>
      setAdaptiveArtifactState(args.kind, args.artifactId, { action: args.action, reason: args.reason }),
  );
  const bulkReviewMutation = useApiMutation(
    async (args: {
      artifacts: Array<{ artifact_kind: AdaptiveArtifactKind; artifact_id: string }>;
      decision: AdaptiveReviewDecision;
      reason?: string;
    }) => reviewAdaptiveArtifactsBulk(args.artifacts, { decision: args.decision, reason: args.reason }),
  );
  const bulkStateMutation = useApiMutation(
    async (args: {
      artifacts: Array<{ artifact_kind: AdaptiveArtifactKind; artifact_id: string }>;
      action: AdaptiveRuntimeAction;
      reason?: string;
    }) => setAdaptiveArtifactsStateBulk(args.artifacts, { action: args.action, reason: args.reason }),
  );
  const saveViewMutation = useApiMutation(
    async (args: {
      view_id?: string;
      name: string;
      description?: string;
      search_text?: string;
      assigned_reviewer?: string;
      artifacts: Array<{ artifact_kind: AdaptiveArtifactKind; artifact_id: string }>;
    }) => saveAdaptiveReviewView(args),
  );
  const deleteViewMutation = useApiMutation(async (viewId: string) => deleteAdaptiveReviewView(viewId));
  const useViewMutation = useApiMutation(async (viewId: string) => useAdaptiveReviewView(viewId));
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [selectedKeys, setSelectedKeys] = useState<string[]>([]);
  const [search, setSearch] = useState("");
  const [reason, setReason] = useState("");
  const [activeViewId, setActiveViewId] = useState<string | null>(null);
  const [viewName, setViewName] = useState("");
  const [viewDescription, setViewDescription] = useState("");
  const [viewReviewer, setViewReviewer] = useState("");

  const artifacts = useMemo(() => flattenArtifacts(review.data), [review.data]);
  const comparisonRows = review.data?.comparison_rows ?? [];
  const savedViews = review.data?.saved_views ?? [];
  const trendDrilldowns = review.data?.trend_drilldowns ?? [];
  const comparisonByKey = useMemo(
    () => new Map(comparisonRows.map((row) => [`${row.artifact_kind}:${row.artifact_id}`, row])),
    [comparisonRows],
  );
  const trendByKey = useMemo(
    () => new Map(trendDrilldowns.map((item) => [`${item.artifact_kind}:${item.artifact_id}`, item])),
    [trendDrilldowns],
  );
  const historyByKey = useMemo(() => {
    const entries = review.data?.history_summary.artifacts ?? [];
    return new Map(entries.map((entry) => [`${entry.artifact_kind}:${entry.artifact_id}`, entry]));
  }, [review.data?.history_summary.artifacts]);
  const attentionKeys = useMemo(() => {
    const items = review.data?.artifacts_requiring_attention ?? [];
    return new Set(
      items
        .map((item) => attentionArtifactKey(item))
        .filter((value): value is string => Boolean(value)),
    );
  }, [review.data?.artifacts_requiring_attention]);
  const filteredArtifacts = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return artifacts;
    return artifacts.filter((artifact) =>
      [artifact.label, artifact.artifactId, artifact.kind, artifact.status, artifact.reviewStatus]
        .join(" ")
        .toLowerCase()
        .includes(needle),
    );
  }, [artifacts, search]);
  const selectedKeySet = useMemo(() => new Set(selectedKeys), [selectedKeys]);
  const selectedComparisonRows = useMemo(
    () => selectedKeys.map((key) => comparisonByKey.get(key)).filter((row): row is AdaptiveComparisonRow => Boolean(row)),
    [comparisonByKey, selectedKeys],
  );
  const selectedTotals = useMemo(
    () =>
      selectedComparisonRows.reduce(
        (acc, row) => {
          acc.confirmations += row.confirmations;
          acc.falsePositives += row.false_positives;
          acc.activeIncidents += row.active_incident_count;
          acc.impact += row.active_cumulative_impact;
          return acc;
        },
        { confirmations: 0, falsePositives: 0, activeIncidents: 0, impact: 0 },
      ),
    [selectedComparisonRows],
  );
  const selectedTrendDrilldowns = useMemo(
    () => selectedKeys.map((key) => trendByKey.get(key)).filter((row): row is AdaptiveTrendDrilldown => Boolean(row)),
    [selectedKeys, trendByKey],
  );
  const selectedArtifact =
    artifacts.find((artifact) => artifact.key === selectedKey) ?? filteredArtifacts[0] ?? artifacts[0] ?? null;
  const selectedHistory = selectedArtifact ? historyByKey.get(selectedArtifact.key) ?? null : null;
  const selectedComparison = selectedArtifact ? comparisonByKey.get(selectedArtifact.key) ?? null : null;
  const busy =
    reviewMutation.isPending ||
    stateMutation.isPending ||
    bulkReviewMutation.isPending ||
    bulkStateMutation.isPending ||
    saveViewMutation.isPending ||
    deleteViewMutation.isPending ||
    useViewMutation.isPending;

  useEffect(() => {
    if (!artifacts.length) {
      setSelectedKey(null);
      return;
    }
    setSelectedKey((current) => {
      if (current && artifacts.some((artifact) => artifact.key === current)) {
        return current;
      }
      const queued = review.data?.review_queue[0];
      const preferred = queued ? `${queued.artifact_kind}:${queued.artifact_id}` : artifacts[0]?.key;
      return preferred ?? artifacts[0]?.key ?? null;
    });
  }, [artifacts, review.data?.review_queue]);

  useEffect(() => {
    setSelectedKeys((current) => current.filter((key) => artifacts.some((artifact) => artifact.key === key)));
  }, [artifacts]);

  useEffect(() => {
    setActiveViewId((current) => (current && savedViews.some((view) => view.view_id === current) ? current : null));
  }, [savedViews]);

  useEffect(() => {
    const activeView = savedViews.find((view) => view.view_id === activeViewId) ?? null;
    if (!activeView) {
      setViewName("");
      setViewDescription("");
      setViewReviewer("");
      return;
    }
    setViewName(activeView.name);
    setViewDescription(activeView.description ?? "");
    setViewReviewer(activeView.assigned_reviewer ?? "");
  }, [activeViewId, savedViews]);

  useEffect(() => {
    if (!selectedArtifact) {
      setReason("");
      return;
    }
    setReason(selectedArtifact.reviewReason ?? selectedArtifact.statusReason ?? "");
  }, [selectedArtifact?.key]);

  const submitDecision = async (decision: AdaptiveReviewDecision) => {
    if (!selectedArtifact) return;
    const trimmed = reason.trim();
    try {
      const payload = await reviewMutation.run({
        kind: selectedArtifact.kind,
        artifactId: selectedArtifact.artifactId,
        decision,
        reason: trimmed || undefined,
      });
      review.setData(payload.review);
      if (decision === "reset") {
        setReason("");
      }
      toast.success(`Review decision recorded`, {
        description: `${selectedArtifact.label} is now ${decision}.`,
      });
    } catch (error) {
      toast.error("Could not save review decision", {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const submitRuntimeAction = async (action: AdaptiveRuntimeAction) => {
    if (!selectedArtifact) return;
    const trimmed = reason.trim();
    try {
      await stateMutation.run({
        kind: selectedArtifact.kind,
        artifactId: selectedArtifact.artifactId,
        action,
        reason: trimmed || undefined,
      });
      toast.success(`Artifact ${action}d`, {
        description: `${selectedArtifact.label} runtime state was updated.`,
      });
      await review.reload({ silent: true });
    } catch (error) {
      toast.error(`Could not ${action} artifact`, {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const toggleSelected = (key: string) => {
    setSelectedKeys((current) => (current.includes(key) ? current.filter((value) => value !== key) : [...current, key]));
  };

  const selectFilteredArtifacts = () => {
    setSelectedKeys((current) => {
      const next = new Set(current);
      for (const artifact of filteredArtifacts) {
        next.add(artifact.key);
      }
      return Array.from(next);
    });
  };

  const clearSelection = () => setSelectedKeys([]);

  const submitBulkDecision = async (decision: AdaptiveReviewDecision) => {
    if (!selectedComparisonRows.length) return;
    const trimmed = reason.trim();
    try {
      const payload = await bulkReviewMutation.run({
        artifacts: selectedComparisonRows.map((row) => ({
          artifact_kind: row.artifact_kind,
          artifact_id: row.artifact_id,
        })),
        decision,
        reason: trimmed || undefined,
      });
      review.setData(payload.review);
      toast.success("Bulk review saved", {
        description: `${payload.updated_count} artifacts updated with ${decision}.`,
      });
      if (decision === "reset") {
        setReason("");
      }
    } catch (error) {
      toast.error("Could not save bulk review", {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const submitBulkRuntimeAction = async (action: AdaptiveRuntimeAction) => {
    if (!selectedComparisonRows.length) return;
    const trimmed = reason.trim();
    try {
      const payload = await bulkStateMutation.run({
        artifacts: selectedComparisonRows.map((row) => ({
          artifact_kind: row.artifact_kind,
          artifact_id: row.artifact_id,
        })),
        action,
        reason: trimmed || undefined,
      });
      review.setData(payload.review);
      toast.success("Bulk runtime update saved", {
        description: `${payload.updated_count} artifacts ${action}d.`,
      });
    } catch (error) {
      toast.error("Could not update bulk runtime state", {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const applySavedView = async (view: AdaptiveSavedReviewView) => {
    setActiveViewId(view.view_id);
    setSearch(view.search_text ?? "");
    setSelectedKeys(view.artifact_selections.map((selection) => `${selection.artifact_kind}:${selection.artifact_id}`));
    const preferred =
      view.artifact_selections[0] &&
      `${view.artifact_selections[0].artifact_kind}:${view.artifact_selections[0].artifact_id}`;
    if (preferred) {
      setSelectedKey(preferred);
    }
    try {
      const payload = await useViewMutation.run(view.view_id);
      review.setData(payload.review);
    } catch (error) {
      toast.error("Could not mark saved view as used", {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const saveCurrentView = async () => {
    const trimmedName = viewName.trim();
    if (!trimmedName) {
      toast.error("Saved view needs a name");
      return;
    }
    try {
      const payload = await saveViewMutation.run({
        view_id: activeViewId ?? undefined,
        name: trimmedName,
        description: viewDescription.trim() || undefined,
        search_text: search.trim() || undefined,
        assigned_reviewer: viewReviewer.trim() || undefined,
        artifacts: selectedKeys
          .map((key) => {
            const [artifact_kind, ...rest] = key.split(":");
            const artifact_id = rest.join(":");
            if (!artifact_kind || !artifact_id) return null;
            return { artifact_kind: artifact_kind as AdaptiveArtifactKind, artifact_id };
          })
          .filter((value): value is { artifact_kind: AdaptiveArtifactKind; artifact_id: string } => Boolean(value)),
      });
      review.setData(payload.review);
      setActiveViewId(payload.view_id);
      toast.success("Saved review view updated", {
        description: `${trimmedName} is now persisted for later triage.`,
      });
    } catch (error) {
      toast.error("Could not save review view", {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  const deleteCurrentView = async () => {
    if (!activeViewId) return;
    const currentName = savedViews.find((view) => view.view_id === activeViewId)?.name ?? "Saved view";
    try {
      const payload = await deleteViewMutation.run(activeViewId);
      review.setData(payload.review);
      setActiveViewId(null);
      toast.success("Saved review view deleted", {
        description: `${currentName} was removed.`,
      });
    } catch (error) {
      toast.error("Could not delete review view", {
        description: error instanceof Error ? error.message : String(error),
      });
    }
  };

  if (review.isLoading && !review.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Learning review" subtitle="Operator review queue for adaptive learning artifacts." mode={mode} />
        <LoadingState title="Loading adaptive review workflow" />
      </div>
    );
  }

  if (review.errorMessage && !review.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Learning review" subtitle="Operator review queue for adaptive learning artifacts." mode={mode} />
        <ErrorState description={review.errorMessage} onRetry={() => void review.reload()} />
      </div>
    );
  }

  if (!review.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Learning review" subtitle="Operator review queue for adaptive learning artifacts." mode={mode} />
        <EmptyState title="No review data" description="Inferra did not return any adaptive-learning review state." />
      </div>
    );
  }

  const reviewCounts = review.data.review_counts;
  const approvedCount = reviewCounts.approved ?? 0;
  const watchCount = reviewCounts.watch ?? 0;
  const rejectedCount = reviewCounts.rejected ?? 0;
  const unreviewedCount = reviewCounts.unreviewed ?? 0;
  const recentActivity = review.data.recent_review_activity.slice(-8).reverse();
  const incidents = review.data.active_incident_influence.slice(0, 8);

  return (
    <div className="space-y-6">
      <PageHeader
        title="Learning review"
        subtitle="Review, approve, watch, or retire learned detectors and compositions with live incident context and audited history."
        mode={mode}
        actions={
          <Button variant="outline" size="sm" onClick={() => void review.reload({ silent: true })}>
            <RefreshCcw className={`size-4 ${review.isRefreshing ? "animate-spin" : ""}`} />
            Refresh
          </Button>
        }
      />

      <div className="dashboard-grid">
        <MetricCard
          title="Pending review"
          value={String(review.data.review_queue.length)}
          note={`${unreviewedCount} artifacts still unreviewed`}
        />
        <MetricCard
          title="Needs attention"
          value={String(review.data.artifacts_requiring_attention.length)}
          note={`${review.data.summary.counts.manually_disabled} manually disabled right now`}
        />
        <MetricCard
          title="Reviewed"
          value={String(approvedCount + watchCount + rejectedCount)}
          note={`${approvedCount} approved · ${watchCount} watch · ${rejectedCount} rejected`}
        />
        <MetricCard
          title="Active influence"
          value={String(review.data.active_incident_influence.length)}
          note={`${review.data.history_summary.count} artifacts have recorded movement history`}
        />
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.2fr)_minmax(360px,0.8fr)]">
        <Card>
          <CardHeader>
            <CardTitle>Bulk triage</CardTitle>
            <CardDescription>Compare multiple artifacts side-by-side, then apply one review or runtime decision across the selected set.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid gap-3 md:grid-cols-4">
              <StatusStat label="Selected" value={String(selectedComparisonRows.length)} />
              <StatusStat label="Confirmations" value={String(selectedTotals.confirmations)} />
              <StatusStat label="False positives" value={String(selectedTotals.falsePositives)} />
              <StatusStat label="Active incidents" value={String(selectedTotals.activeIncidents)} />
            </div>
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" onClick={selectFilteredArtifacts}>
                Select filtered
              </Button>
              <Button variant="outline" size="sm" onClick={clearSelection} disabled={!selectedComparisonRows.length}>
                Clear selection
              </Button>
              <Button onClick={() => void submitBulkDecision("approve")} disabled={busy || !selectedComparisonRows.length}>
                <CheckCircle2 className="size-4" />
                Approve selected
              </Button>
              <Button variant="outline" onClick={() => void submitBulkDecision("watch")} disabled={busy || !selectedComparisonRows.length}>
                <Eye className="size-4" />
                Watch selected
              </Button>
              <Button variant="destructive" onClick={() => void submitBulkDecision("reject")} disabled={busy || !selectedComparisonRows.length}>
                <ThumbsDown className="size-4" />
                Reject selected
              </Button>
              <Button variant="outline" onClick={() => void submitBulkDecision("reset")} disabled={busy || !selectedComparisonRows.length}>
                <Undo2 className="size-4" />
                Reset selected
              </Button>
              <Button variant="outline" onClick={() => void submitBulkRuntimeAction("enable")} disabled={busy || !selectedComparisonRows.length}>
                <Activity className="size-4" />
                Enable selected
              </Button>
              <Button variant="destructive" onClick={() => void submitBulkRuntimeAction("disable")} disabled={busy || !selectedComparisonRows.length}>
                <CircleOff className="size-4" />
                Disable selected
              </Button>
            </div>
            {selectedComparisonRows.length ? (
              <TableWrap>
                <Table>
                  <thead>
                    <tr>
                      <Th>Artifact</Th>
                      <Th>Review</Th>
                      <Th className="text-right">Conf</Th>
                      <Th className="text-right">False+</Th>
                      <Th className="text-right">Incidents</Th>
                      <Th className="text-right">Impact</Th>
                    </tr>
                  </thead>
                  <tbody>
                    {selectedComparisonRows.map((row) => (
                      <tr key={`${row.artifact_kind}:${row.artifact_id}`}>
                        <Td>
                          <button
                            type="button"
                            className="text-left hover:underline"
                            onClick={() => setSelectedKey(`${row.artifact_kind}:${row.artifact_id}`)}
                          >
                            <div className="font-medium">{row.label}</div>
                            <div className="text-xs text-muted-foreground">
                              {humanizeKind(row.artifact_kind)} · {row.artifact_id}
                            </div>
                          </button>
                        </Td>
                        <Td>
                          <div className="flex flex-wrap gap-2">
                            <Badge variant={reviewVariant(row.review_status)}>{formatDisplayValue(row.review_status)}</Badge>
                            {row.attention ? <Badge variant="warning">attention</Badge> : null}
                            {(row.review_status === "unreviewed" || row.review_status === "watch") ? (
                              <Badge variant={agingVariant(row.aging_bucket)}>{row.aging_bucket}</Badge>
                            ) : null}
                          </div>
                        </Td>
                        <Td className="text-right">{row.confirmations}</Td>
                        <Td className="text-right">{row.false_positives}</Td>
                        <Td className="text-right">{row.active_incident_count}</Td>
                        <Td className="text-right">{row.active_cumulative_impact.toFixed(2)}</Td>
                      </tr>
                    ))}
                  </tbody>
                </Table>
              </TableWrap>
            ) : (
              <Alert variant="info">
                <Eye className="size-4" />
                <div className="min-w-0">
                  <AlertTitle>No bulk selection yet</AlertTitle>
                  <AlertDescription>Select artifacts from the lists below to compare them and apply one operator decision across the set.</AlertDescription>
                </div>
              </Alert>
            )}
          </CardContent>
        </Card>

        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Saved views</CardTitle>
              <CardDescription>Persist bulk selections, search state, and reviewer ownership so triage queues survive past this session.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="grid gap-3">
                <Input value={viewName} onChange={(event) => setViewName(event.target.value)} placeholder="Queue name" />
                <Input value={viewReviewer} onChange={(event) => setViewReviewer(event.target.value)} placeholder="Assigned reviewer" />
                <Textarea
                  value={viewDescription}
                  onChange={(event) => setViewDescription(event.target.value)}
                  className="min-h-[88px]"
                  placeholder="Why this saved cohort exists and what it is for."
                />
              </div>
              <div className="flex flex-wrap gap-2">
                <Button onClick={() => void saveCurrentView()} disabled={busy}>
                  Save current cohort
                </Button>
                <Button variant="outline" onClick={() => setActiveViewId(null)} disabled={busy || !activeViewId}>
                  New view
                </Button>
                <Button variant="destructive" onClick={() => void deleteCurrentView()} disabled={busy || !activeViewId}>
                  Delete view
                </Button>
              </div>
              <div className="space-y-3">
                {savedViews.length ? (
                  savedViews.map((view) => (
                    <button
                      key={view.view_id}
                      type="button"
                      onClick={() => void applySavedView(view)}
                      className={`w-full rounded-2xl border p-4 text-left transition ${
                        activeViewId === view.view_id
                          ? "border-sky-400/35 bg-sky-400/10"
                          : "border-border/60 bg-background/30 hover:border-border hover:bg-background/50"
                      }`}
                    >
                      <div className="flex flex-wrap items-start justify-between gap-3">
                        <div>
                          <p className="font-medium">{view.name}</p>
                          <p className="mt-1 text-sm text-muted-foreground">
                            {view.assigned_reviewer || "unassigned"} · {view.match_count} artifacts · {view.pending_review_count} pending
                          </p>
                        </div>
                        <div className="flex flex-wrap gap-2">
                          <Badge variant="outline">{view.aging_bucket}</Badge>
                          {view.stale_pending ? <Badge variant="warning">stale pending</Badge> : null}
                        </div>
                      </div>
                      <p className="mt-2 text-sm text-muted-foreground">
                        {view.description || "No description recorded."}
                      </p>
                      <p className="mt-2 text-xs text-muted-foreground">
                        Last used {formatRelativeDate(view.last_used_at)} · oldest pending {formatHours(view.oldest_pending_age_hours)}
                      </p>
                    </button>
                  ))
                ) : (
                  <p className="text-sm text-muted-foreground">No saved review views yet. Save the current cohort once it becomes worth revisiting.</p>
                )}
              </div>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Compare-many analytics</CardTitle>
              <CardDescription>Sorted cohorts that surface which artifacts are strongest, noisiest, or shaping active incidents.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="grid gap-3 sm:grid-cols-2">
                {review.data.analytics.kind_breakdown.map((entry) => (
                  <div key={entry.artifact_kind} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                    <div className="flex items-center justify-between gap-2">
                      <p className="font-medium">{humanizeKind(entry.artifact_kind)}</p>
                      <Badge variant="outline">{entry.total}</Badge>
                    </div>
                    <p className="mt-2 text-sm text-muted-foreground">
                      {entry.unreviewed} unreviewed · {entry.attention} attention · {entry.manually_disabled} disabled
                    </p>
                  </div>
                ))}
              </div>
              <ArtifactCohort title="Highest impact" rows={review.data.analytics.top_impact} onFocus={setSelectedKey} />
              <ArtifactCohort title="Noisiest" rows={review.data.analytics.top_noisy} onFocus={setSelectedKey} />
              <ArtifactCohort title="Most confirmed" rows={review.data.analytics.top_confirmed} onFocus={setSelectedKey} />
              <ArtifactCohort title="Recently changed" rows={review.data.analytics.recently_changed} onFocus={setSelectedKey} />
            </CardContent>
          </Card>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Cohort trend drilldowns</CardTitle>
          <CardDescription>Recent score, rank, and impact movement for the currently selected comparison cohort.</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {selectedTrendDrilldowns.length ? (
            selectedTrendDrilldowns.map((drilldown) => (
              <TrendDrilldownCard key={`${drilldown.artifact_kind}:${drilldown.artifact_id}`} drilldown={drilldown} onFocus={setSelectedKey} />
            ))
          ) : (
            <Alert variant="info">
              <Clock3 className="size-4" />
              <div className="min-w-0">
                <AlertTitle>No trend cohort selected</AlertTitle>
                <AlertDescription>Select one or more artifacts for comparison to inspect their recent longitudinal movement together.</AlertDescription>
              </div>
            </Alert>
          )}
        </CardContent>
      </Card>

      <div className="grid gap-4 xl:grid-cols-[minmax(320px,0.9fr)_minmax(0,1.1fr)]">
        <div className="space-y-4">
          <Card>
            <CardHeader>
              <CardTitle>Review queue</CardTitle>
              <CardDescription>Fresh artifacts that still need an explicit operator decision.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {review.data.review_queue.length ? (
                review.data.review_queue.map((item) => {
                  const key = `${item.artifact_kind}:${item.artifact_id}`;
                  return (
                    <ArtifactListItem
                      key={key}
                      active={selectedArtifact?.key === key}
                      selected={selectedKeySet.has(key)}
                      title={item.label}
                      subtitle={`${humanizeKind(item.artifact_kind)} · ${item.confirmations} confirmations · ${item.false_positives} false positives`}
                      badges={[
                        <Badge key="status" variant={statusVariant(item.status)}>
                          {formatDisplayValue(item.status)}
                        </Badge>,
                        <Badge key="review" variant={reviewVariant(item.review_status)}>
                          {formatDisplayValue(item.review_status)}
                        </Badge>,
                      ]}
                      onClick={() => setSelectedKey(key)}
                      onToggleSelect={() => toggleSelected(key)}
                    />
                  );
                })
              ) : (
                <p className="text-sm text-muted-foreground">No artifacts are waiting in the review queue right now.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Needs attention</CardTitle>
              <CardDescription>Artifacts that are noisy, suppressed, or manually disabled.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              {review.data.artifacts_requiring_attention.length ? (
                review.data.artifacts_requiring_attention.slice(0, 10).map((item, index) => {
                  const record = attentionArtifactSummary(item, index);
                  return (
                    <ArtifactListItem
                      key={record.key}
                      active={selectedArtifact?.key === record.key}
                      selected={selectedKeySet.has(record.key)}
                      title={record.label}
                      subtitle={`${humanizeKind(record.kind)} · ${record.confirmations} confirmations · ${record.falsePositives} false positives`}
                      badges={[
                        <Badge key="status" variant={statusVariant(record.status)}>
                          {formatDisplayValue(record.status)}
                        </Badge>,
                        <Badge key="review" variant={reviewVariant(record.reviewStatus)}>
                          {record.reviewStatus}
                        </Badge>,
                      ]}
                      onClick={() => setSelectedKey(record.key)}
                      onToggleSelect={() => toggleSelected(record.key)}
                    />
                  );
                })
              ) : (
                <p className="text-sm text-muted-foreground">No artifacts currently require operator attention.</p>
              )}
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>All artifacts</CardTitle>
              <CardDescription>Search across every learned artifact, not only the pending queue.</CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="relative">
                <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                  value={search}
                  onChange={(event) => setSearch(event.target.value)}
                  className="pl-9"
                  placeholder="Search by label, id, kind, or status"
                />
              </div>
              <div className="space-y-3">
                {filteredArtifacts.length ? (
                  filteredArtifacts.slice(0, 14).map((artifact) => (
                    <ArtifactListItem
                      key={artifact.key}
                      active={selectedArtifact?.key === artifact.key}
                      selected={selectedKeySet.has(artifact.key)}
                      title={artifact.label}
                      subtitle={`${humanizeKind(artifact.kind)} · updated ${formatRelativeDate(artifact.updatedAt)}`}
                      badges={[
                        attentionKeys.has(artifact.key) ? (
                          <Badge key="attention" variant="warning">
                            attention
                          </Badge>
                        ) : null,
                        <Badge key="review" variant={reviewVariant(artifact.reviewStatus)}>
                          {artifact.reviewStatus}
                        </Badge>,
                      ].filter(Boolean)}
                      onClick={() => setSelectedKey(artifact.key)}
                      onToggleSelect={() => toggleSelected(artifact.key)}
                    />
                  ))
                ) : (
                  <p className="text-sm text-muted-foreground">No learned artifacts match this search.</p>
                )}
              </div>
            </CardContent>
          </Card>
        </div>

        <div className="space-y-4">
          {selectedArtifact ? (
            <Card>
              <CardHeader>
                <CardTitle>Selected artifact</CardTitle>
                <CardDescription>Make the review decision and runtime state explicit for this learned artifact.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-5">
                <div className="space-y-3 rounded-2xl border border-border/60 bg-background/30 p-4">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div>
                      <p className="text-lg font-semibold">{selectedArtifact.label}</p>
                      <p className="text-sm text-muted-foreground">
                        {humanizeKind(selectedArtifact.kind)} · id {selectedArtifact.artifactId}
                      </p>
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <Badge variant={statusVariant(selectedArtifact.status)}>{formatDisplayValue(selectedArtifact.status)}</Badge>
                      <Badge variant={reviewVariant(selectedArtifact.reviewStatus)}>{selectedArtifact.reviewStatus}</Badge>
                      {attentionKeys.has(selectedArtifact.key) ? <Badge variant="warning">attention</Badge> : null}
                    </div>
                  </div>

                  <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                    <StatusStat label="Confirmations" value={String(selectedArtifact.confirmations)} />
                    <StatusStat label="False positives" value={String(selectedArtifact.falsePositives)} />
                    <StatusStat label="Updated" value={formatRelativeDate(selectedArtifact.updatedAt)} />
                    <StatusStat label="Last reviewed" value={formatRelativeDate(selectedArtifact.lastReviewedAt)} />
                  </div>

                  {selectedComparison && (selectedComparison.review_status === "unreviewed" || selectedComparison.review_status === "watch") ? (
                    <Alert variant={agingAlertVariant(selectedComparison.aging_bucket)}>
                      <div className="min-w-0">
                        <AlertTitle>Review aging cue</AlertTitle>
                        <AlertDescription>
                          {selectedComparison.review_status === "unreviewed"
                            ? `This artifact has been waiting ${formatHours(selectedComparison.pending_review_age_hours)} for an explicit review decision.`
                            : `This watch-state artifact has not been revisited for ${formatHours(selectedComparison.watch_age_hours)}.`}
                        </AlertDescription>
                      </div>
                    </Alert>
                  ) : null}

                  {selectedArtifact.reviewReason ? (
                    <Alert variant={reviewAlertVariant(selectedArtifact.reviewStatus)}>
                      <div className="min-w-0">
                        <AlertTitle>Current review rationale</AlertTitle>
                        <AlertDescription>{selectedArtifact.reviewReason}</AlertDescription>
                      </div>
                    </Alert>
                  ) : null}

                  {selectedArtifact.statusReason ? (
                    <Alert variant="warning">
                      <div className="min-w-0">
                        <AlertTitle>Runtime status note</AlertTitle>
                        <AlertDescription>{selectedArtifact.statusReason}</AlertDescription>
                      </div>
                    </Alert>
                  ) : null}

                  <ArtifactSignals artifact={selectedArtifact} />
                </div>

                <div className="space-y-3">
                  <div>
                    <p className="text-sm font-medium">Operator rationale</p>
                    <p className="text-sm text-muted-foreground">
                      Record why you are approving, watching, rejecting, or changing runtime state.
                    </p>
                  </div>
                  <Textarea
                    value={reason}
                    onChange={(event) => setReason(event.target.value)}
                    className="min-h-[120px]"
                    placeholder="Explain why this artifact is good, risky, noisy, or worth watching."
                  />
                </div>

                <div className="grid gap-3 xl:grid-cols-2">
                  <div className="rounded-2xl border border-border/60 bg-background/30 p-4">
                    <p className="text-sm font-semibold">Review decisions</p>
                    <p className="mt-1 text-sm text-muted-foreground">Review state stays explicit and durable. Rejection also retires runtime influence.</p>
                    <div className="mt-4 flex flex-wrap gap-3">
                      <Button onClick={() => void submitDecision("approve")} disabled={busy}>
                        <CheckCircle2 className="size-4" />
                        Approve
                      </Button>
                      <Button variant="outline" onClick={() => void submitDecision("watch")} disabled={busy}>
                        <Eye className="size-4" />
                        Watch
                      </Button>
                      <Button variant="destructive" onClick={() => void submitDecision("reject")} disabled={busy}>
                        <ThumbsDown className="size-4" />
                        Reject
                      </Button>
                      <Button variant="outline" onClick={() => void submitDecision("reset")} disabled={busy}>
                        <Undo2 className="size-4" />
                        Reset
                      </Button>
                    </div>
                  </div>

                  <div className="rounded-2xl border border-border/60 bg-background/30 p-4">
                    <p className="text-sm font-semibold">Runtime state</p>
                    <p className="mt-1 text-sm text-muted-foreground">Use this when you need to hard-disable or restore runtime influence regardless of review state.</p>
                    <div className="mt-4 flex flex-wrap gap-3">
                      <Button
                        variant="outline"
                        onClick={() => void submitRuntimeAction("enable")}
                        disabled={busy || !selectedArtifact.manuallyDisabled}
                      >
                        <Activity className="size-4" />
                        Enable
                      </Button>
                      <Button
                        variant="destructive"
                        onClick={() => void submitRuntimeAction("disable")}
                        disabled={busy || selectedArtifact.manuallyDisabled}
                      >
                        <CircleOff className="size-4" />
                        Disable
                      </Button>
                    </div>
                  </div>
                </div>

                {selectedHistory ? (
                  <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                    <StatusStat label="History observations" value={String(selectedHistory.observations)} />
                    <StatusStat
                      label="Latest score"
                      value={selectedHistory.latest_score == null ? "—" : selectedHistory.latest_score.toFixed(2)}
                    />
                    <StatusStat label="Best rank" value={selectedHistory.best_rank == null ? "—" : String(selectedHistory.best_rank)} />
                    <StatusStat
                      label="Score movement"
                      value={selectedHistory.cumulative_score_delta == null ? "—" : signedNumber(selectedHistory.cumulative_score_delta)}
                    />
                  </div>
                ) : (
                  <Alert variant="info">
                    <Clock3 className="size-4" />
                    <div className="min-w-0">
                      <AlertTitle>No history yet</AlertTitle>
                      <AlertDescription>This artifact has not accumulated longitudinal score or edge movement observations yet.</AlertDescription>
                    </div>
                  </Alert>
                )}

                <JsonInspector data={selectedArtifact.detail} title="Artifact details" />
              </CardContent>
            </Card>
          ) : (
            <EmptyState
              title="No learned artifacts yet"
              description="Submit feedback and let adaptive learning observe enough evidence before expecting a review workflow."
            />
          )}

          <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
            <Card>
              <CardHeader>
                <CardTitle>Recent review activity</CardTitle>
                <CardDescription>The latest explicit operator review decisions.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {recentActivity.length ? (
                  recentActivity.map((entry) => (
                    <div key={entry.audit_id} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge variant="outline">{entry.artifact_kind}</Badge>
                        <Badge variant={reviewVariant(entry.review_status_after ?? "unreviewed")}>
                          {formatDisplayValue(entry.review_status_after ?? entry.action)}
                        </Badge>
                      </div>
                      <p className="mt-2 text-sm font-medium">{entry.artifact_id}</p>
                      <p className="mt-1 text-sm text-muted-foreground">{entry.reason || entry.runtime_effect || "No rationale recorded."}</p>
                      <p className="mt-2 text-xs text-muted-foreground">{formatRelativeDate(entry.created_at)}</p>
                    </div>
                  ))
                ) : (
                  <p className="text-sm text-muted-foreground">No explicit review activity has been recorded yet.</p>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle>Incident influence</CardTitle>
                <CardDescription>Where learned artifacts are shaping active incidents right now.</CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {incidents.length ? (
                  incidents.map((item) => (
                    <div key={item.incident_id} className="rounded-2xl border border-border/60 bg-background/30 p-4">
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <div>
                          <Link to={`/incidents/${item.incident_id}`} className="font-medium hover:underline">
                            {item.incident_id}
                          </Link>
                          <p className="text-sm text-muted-foreground">
                            {item.primary_service || "Unknown Service"} · {formatDisplayValue(item.state || "unknown state")}
                          </p>
                        </div>
                        <Badge variant="info">{item.learning.influenced_hypotheses} influenced hypotheses</Badge>
                      </div>
                      <p className="mt-2 text-sm text-muted-foreground">
                        Estimated learned impact {item.learning.estimated_total_impact.toFixed(2)}
                      </p>
                      {item.learning.artifacts.length ? (
                        <div className="mt-3 flex flex-wrap gap-2">
                          {item.learning.artifacts.slice(0, 4).map((artifact, index) => (
                            <Badge key={`${artifact.artifact_id ?? index}`} variant="outline">
                              {artifact.label ?? artifact.artifact_id ?? "unknown artifact"}
                            </Badge>
                          ))}
                        </div>
                      ) : null}
                    </div>
                  ))
                ) : (
                  <p className="text-sm text-muted-foreground">No active incidents are currently showing learned-artifact influence.</p>
                )}
              </CardContent>
            </Card>
          </div>
        </div>
      </div>
    </div>
  );
}

function MetricCard({ title, value, note }: { title: string; value: string; note: string }) {
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

function StatusStat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-2xl border border-border/60 bg-background/30 px-4 py-3">
      <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">{label}</p>
      <p className="mt-2 text-sm font-medium">{value}</p>
    </div>
  );
}

function ArtifactListItem({
  title,
  subtitle,
  badges,
  active,
  selected,
  onClick,
  onToggleSelect,
}: {
  title: string;
  subtitle: string;
  badges: ReactNode[];
  active: boolean;
  selected: boolean;
  onClick: () => void;
  onToggleSelect: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`w-full rounded-2xl border p-4 text-left transition ${
        active
          ? "border-sky-400/35 bg-sky-400/10"
          : "border-border/60 bg-background/30 hover:border-border hover:bg-background/50"
      }`}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="font-medium">{title}</p>
          <p className="mt-1 text-sm text-muted-foreground">{subtitle}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <label
            className="flex items-center gap-2 rounded-full border border-border/60 px-2 py-1 text-xs text-muted-foreground"
            onClick={(event) => event.stopPropagation()}
          >
            <input type="checkbox" checked={selected} onChange={onToggleSelect} />
            Compare
          </label>
          <div className="flex flex-wrap gap-2">{badges}</div>
        </div>
      </div>
    </button>
  );
}

function ArtifactCohort({
  title,
  rows,
  onFocus,
}: {
  title: string;
  rows: AdaptiveComparisonRow[];
  onFocus: (key: string) => void;
}) {
  return (
    <div className="space-y-3 rounded-2xl border border-border/60 bg-background/30 p-4">
      <div className="flex items-center justify-between gap-2">
        <p className="text-sm font-semibold">{title}</p>
        <Badge variant="outline">{rows.length}</Badge>
      </div>
      {rows.length ? (
        <div className="space-y-2">
          {rows.map((row) => (
            <button
              key={`${row.artifact_kind}:${row.artifact_id}`}
              type="button"
              className="w-full rounded-2xl border border-border/60 px-3 py-2 text-left hover:border-border hover:bg-background/50"
              onClick={() => onFocus(`${row.artifact_kind}:${row.artifact_id}`)}
            >
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div>
                  <p className="font-medium">{row.label}</p>
                  <p className="text-xs text-muted-foreground">
                    {humanizeKind(row.artifact_kind)} · {row.confirmations} confirmations · {row.false_positives} false positives
                  </p>
                </div>
                <div className="flex flex-wrap gap-2">
                  <Badge variant={reviewVariant(row.review_status)}>{formatDisplayValue(row.review_status)}</Badge>
                  <Badge variant="outline">{row.active_incident_count} incidents</Badge>
                </div>
              </div>
            </button>
          ))}
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">No artifacts in this cohort yet.</p>
      )}
    </div>
  );
}

function TrendDrilldownCard({
  drilldown,
  onFocus,
}: {
  drilldown: AdaptiveTrendDrilldown;
  onFocus: (key: string) => void;
}) {
  return (
    <div className="rounded-2xl border border-border/60 bg-background/30 p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <button
            type="button"
            className="text-left hover:underline"
            onClick={() => onFocus(`${drilldown.artifact_kind}:${drilldown.artifact_id}`)}
          >
            <p className="font-medium">{drilldown.artifact_label}</p>
          </button>
          <p className="text-sm text-muted-foreground">
            {humanizeKind(drilldown.artifact_kind)} · {drilldown.observation_count} observations · total movement{" "}
            {drilldown.total_abs_delta.toFixed(2)}
          </p>
        </div>
        <Badge variant="outline">{drilldown.artifact_id}</Badge>
      </div>
      <div className="mt-3 grid gap-3 lg:grid-cols-2">
        {drilldown.observations.map((item, index) => (
          <div key={`${drilldown.artifact_id}-${item.observed_at}-${index}`} className="rounded-2xl border border-border/60 bg-background/40 p-3">
            <div className="flex items-center justify-between gap-2">
              <p className="text-sm font-medium">{formatRelativeDate(item.observed_at)}</p>
              <Badge variant="outline">{item.incident_id}</Badge>
            </div>
            <p className="mt-2 text-xs text-muted-foreground">
              score {item.score == null ? "—" : item.score.toFixed(2)} · rank {item.rank ?? "—"} · impact {item.estimated_impact.toFixed(2)}
            </p>
            <p className="mt-1 text-xs text-muted-foreground">
              delta {item.score_delta == null ? "—" : signedNumber(item.score_delta)} · edge {item.edge_delta == null ? "—" : signedNumber(item.edge_delta)}
            </p>
          </div>
        ))}
      </div>
    </div>
  );
}

function ArtifactSignals({ artifact }: { artifact: ArtifactRecord }) {
  if (artifact.kind === "detector") {
    const detail = artifact.detail as AdaptiveDetector;
    return (
      <div className="grid gap-3 md:grid-cols-2">
        <SignalGroup title="Positive terms" items={detail.positive_terms} />
        <SignalGroup title="Tags" items={detail.tags} />
        <SignalGroup title="Source types" items={detail.source_types} />
        <StatusStat label="Min severity" value={detail.min_severity == null ? "—" : String(detail.min_severity)} />
      </div>
    );
  }

  if (artifact.kind === "edge_profile") {
    const detail = artifact.detail as AdaptiveEdgeProfile;
    return (
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <StatusStat label="Edge type" value={detail.edge_type} />
        <StatusStat label="Source service" value={detail.source_service || "any"} />
        <StatusStat label="Target service" value={detail.target_service || "any"} />
        <StatusStat label="Avg plausibility" value={detail.average_plausibility.toFixed(2)} />
      </div>
    );
  }

  const detail = artifact.detail as AdaptiveTemplate | AdaptiveComposition;
  return (
    <div className="space-y-3">
      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <StatusStat label="Confidence" value={detail.confidence.toFixed(2)} />
        <StatusStat label="Cause type" value={detail.cause_type || "unknown"} />
        <StatusStat label="Cause subtype" value={detail.cause_subtype || "—"} />
        <StatusStat label="Same service only" value={detail.requires_same_service ? "yes" : "no"} />
      </div>
      <div className="rounded-2xl border border-border/60 bg-background/30 p-4">
        <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Title template</p>
        <p className="mt-2 text-sm">{detail.title_template}</p>
      </div>
      <div className="grid gap-3 md:grid-cols-2">
        <SignalGroup title="Required detectors" items={detail.requires} />
        {"preferred_edge_types" in detail ? (
          <SignalGroup title="Preferred edge types" items={detail.preferred_edge_types} />
        ) : (
          <StatusStat label="Temporal order required" value={detail.requires_temporal_order ? "yes" : "no"} />
        )}
      </div>
    </div>
  );
}

function SignalGroup({ title, items }: { title: string; items: string[] }) {
  return (
    <div className="rounded-2xl border border-border/60 bg-background/30 p-4">
      <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">{title}</p>
      <div className="mt-3 flex flex-wrap gap-2">
        {items.length ? (
          items.map((item) => (
            <Badge key={item} variant="outline">
              {item}
            </Badge>
          ))
        ) : (
          <span className="text-sm text-muted-foreground">None recorded.</span>
        )}
      </div>
    </div>
  );
}

function flattenArtifacts(data: AdaptiveLearningReviewResponse | null): ArtifactRecord[] {
  if (!data) return [];

  return [
    ...data.summary.detectors.map(
      (detail): ArtifactRecord => ({
        key: `detector:${detail.detector_id}`,
        kind: "detector",
        artifactId: detail.detector_id,
        label: detail.requirement_name,
        status: detail.status ?? "unknown",
        reviewStatus: detail.review_status ?? "unreviewed",
        reviewReason: detail.review_reason,
        statusReason: detail.status_reason,
        lastReviewedAt: detail.last_reviewed_at,
        updatedAt: detail.updated_at,
        confirmations: detail.confirmations,
        falsePositives: detail.false_positives,
        manuallyDisabled: detail.manually_disabled,
        detail,
      }),
    ),
    ...data.summary.templates.map(
      (detail): ArtifactRecord => ({
        key: `template:${detail.template_id}`,
        kind: "template",
        artifactId: detail.template_id,
        label: detail.template_name,
        status: detail.status ?? "unknown",
        reviewStatus: detail.review_status ?? "unreviewed",
        reviewReason: detail.review_reason,
        statusReason: detail.status_reason,
        lastReviewedAt: detail.last_reviewed_at,
        updatedAt: detail.updated_at,
        confirmations: detail.confirmations,
        falsePositives: detail.false_positives,
        manuallyDisabled: detail.manually_disabled,
        detail,
      }),
    ),
    ...data.summary.compositions.map(
      (detail): ArtifactRecord => ({
        key: `composition:${detail.composition_id}`,
        kind: "composition",
        artifactId: detail.composition_id,
        label: detail.composition_name,
        status: detail.status ?? "unknown",
        reviewStatus: detail.review_status ?? "unreviewed",
        reviewReason: detail.review_reason,
        statusReason: detail.status_reason,
        lastReviewedAt: detail.last_reviewed_at,
        updatedAt: detail.updated_at,
        confirmations: detail.confirmations,
        falsePositives: detail.false_positives,
        manuallyDisabled: detail.manually_disabled,
        detail,
      }),
    ),
    ...data.summary.edge_profiles.map(
      (detail): ArtifactRecord => ({
        key: `edge_profile:${detail.profile_id}`,
        kind: "edge_profile",
        artifactId: detail.profile_id,
        label: detail.profile_id,
        status: detail.status ?? "unknown",
        reviewStatus: detail.review_status ?? "unreviewed",
        reviewReason: detail.review_reason,
        statusReason: detail.status_reason,
        lastReviewedAt: detail.last_reviewed_at,
        updatedAt: detail.updated_at,
        confirmations: detail.confirmations,
        falsePositives: detail.false_positives,
        manuallyDisabled: detail.manually_disabled,
        detail,
      }),
    ),
  ].sort((left, right) => {
    if (left.reviewStatus === "unreviewed" && right.reviewStatus !== "unreviewed") return -1;
    if (right.reviewStatus === "unreviewed" && left.reviewStatus !== "unreviewed") return 1;
    return right.confirmations - left.confirmations || left.label.localeCompare(right.label);
  });
}

function statusVariant(value: string): "success" | "warning" | "destructive" | "secondary" {
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

function reviewVariant(value: string): "success" | "warning" | "destructive" | "secondary" {
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

function reviewAlertVariant(value: string): "info" | "warning" | "destructive" | "success" {
  switch (value) {
    case "approved":
      return "success";
    case "watch":
      return "warning";
    case "rejected":
      return "destructive";
    default:
      return "info";
  }
}

function agingVariant(value: string): "secondary" | "warning" | "destructive" {
  switch (value) {
    case "aged":
      return "destructive";
    case "stale":
      return "warning";
    default:
      return "secondary";
  }
}

function agingAlertVariant(value: string): "info" | "warning" | "destructive" | "success" {
  switch (value) {
    case "aged":
      return "destructive";
    case "stale":
      return "warning";
    default:
      return "info";
  }
}

function humanizeKind(value: string): string {
  return value.replace(/_/g, " ");
}

function signedNumber(value: number): string {
  return `${value >= 0 ? "+" : ""}${value.toFixed(2)}`;
}

function formatHours(value?: number | null): string {
  if (value == null) return "n/a";
  if (value >= 24 * 7) return `${(value / (24 * 7)).toFixed(1)}w`;
  if (value >= 24) return `${(value / 24).toFixed(1)}d`;
  return `${value.toFixed(1)}h`;
}

function attentionArtifactSummary(value: Record<string, unknown>, index: number) {
  const kind = asString(value.kind) || "detector";
  const artifactId =
    asString(value.detector_id) ||
    asString(value.template_id) ||
    asString(value.composition_id) ||
    asString(value.profile_id) ||
    `artifact-${index + 1}`;
  const label =
    asString(value.requirement_name) ||
    asString(value.template_name) ||
    asString(value.composition_name) ||
    asString(value.profile_id) ||
    artifactId;

  return {
    key: `${kind}:${artifactId}`,
    kind,
    artifactId,
    label,
    status: asString(value.status) || "unknown",
    reviewStatus: asString(value.review_status) || "unreviewed",
    confirmations: asNumber(value.confirmations),
    falsePositives: asNumber(value.false_positives),
  };
}

function attentionArtifactKey(value: Record<string, unknown>): string | null {
  const record = attentionArtifactSummary(value, 0);
  return record.artifactId ? `${record.kind}:${record.artifactId}` : null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function asNumber(value: unknown): number {
  return typeof value === "number" ? value : 0;
}
