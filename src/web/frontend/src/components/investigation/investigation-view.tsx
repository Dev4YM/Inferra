import { AlertTriangle, Brain, RefreshCcw, ShieldCheck, Sparkles } from "lucide-react";

import type { InvestigationResponse } from "@/api";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { JsonInspector } from "@/components/ui/json-inspector";
import { formatRelativeDate, formatRiskTone, investigationHasSignal } from "@/lib/format";

export function InvestigationView({
  result,
  showRaw,
  onRefresh,
}: {
  result: InvestigationResponse;
  showRaw: boolean;
  onRefresh?: () => void;
}) {
  const output = result.output;
  const hasSignal = investigationHasSignal(output);

  return (
    <div className="space-y-4">
      <Card className="overflow-hidden">
        <CardHeader className="border-b border-border/60 bg-gradient-to-r from-sky-500/8 via-transparent to-emerald-400/8">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant={formatRiskTone(output.risk_level)}>
                  risk {output.risk_level}
                </Badge>
                <Badge variant={result.used_ai ? "info" : "warning"}>
                  {result.used_ai ? (
                    <>
                      <Sparkles className="size-3.5" />
                      AI-backed
                    </>
                  ) : (
                    <>
                      <ShieldCheck className="size-3.5" />
                      deterministic fallback
                    </>
                  )}
                </Badge>
                <Badge variant="outline">{Math.round((output.confidence ?? 0) * 100)}% confidence</Badge>
                {result.attempts && result.attempts > 1 ? (
                  <Badge variant="warning">
                    <RefreshCcw className="size-3.5" />
                    succeeded after {result.attempts} attempts
                  </Badge>
                ) : null}
              </div>
              <h2 className="text-xl font-semibold">{output.headline || "Investigation in progress"}</h2>
              {!hasSignal ? (
                <p className="text-sm text-muted-foreground">
                  The response was structurally valid but contained no useful content. Inferra is treating this as a degraded answer.
                </p>
              ) : null}
            </div>
            {onRefresh ? (
              <Button variant="outline" size="sm" onClick={onRefresh}>
                <RefreshCcw className="size-4" />
                Re-run
              </Button>
            ) : null}
          </div>
        </CardHeader>
        <CardContent className="space-y-6 p-6">
          {result.warnings?.length ? (
            <Alert variant="warning">
              <AlertTriangle className="size-4" />
              <div className="min-w-0">
                <AlertTitle>AI response needed recovery</AlertTitle>
                <AlertDescription>{result.warnings.join(" ")}</AlertDescription>
              </div>
            </Alert>
          ) : null}

          {result.grounding &&
          (result.grounding.removed_evidence_ids.length > 0 || result.grounding.removed_citation_ids.length > 0) ? (
            <Alert variant="info">
              <ShieldCheck className="size-4" />
              <div className="min-w-0 space-y-2">
                <AlertTitle>Grounding cleanup</AlertTitle>
                <AlertDescription>
                  Citations or evidence entries that did not match IDs in the investigation bundle were removed before
                  display.
                </AlertDescription>
                {result.grounding.removed_evidence_ids.length ? (
                  <p className="text-xs text-muted-foreground">
                    Removed evidence: {result.grounding.removed_evidence_ids.join(", ")}
                  </p>
                ) : null}
                {result.grounding.removed_citation_ids.length ? (
                  <p className="text-xs text-muted-foreground">
                    Removed citations: {result.grounding.removed_citation_ids.join(", ")}
                  </p>
                ) : null}
              </div>
            </Alert>
          ) : null}

          {!result.used_ai && result.fallback_reason ? (
            <Alert variant="info">
              <ShieldCheck className="size-4" />
              <div className="min-w-0">
                <AlertTitle>Fallback explanation used</AlertTitle>
                <AlertDescription>{result.fallback_reason}</AlertDescription>
              </div>
            </Alert>
          ) : null}

          <Section title="What happened" items={output.what_happened} empty="No concise summary was available." />
          <Section title="Why it matters" items={output.why_it_matters} empty="No impact summary was supplied." />
          <Section title="Likely causes" items={output.likely_causes} empty="No likely causes were proposed." />

          <div className="grid gap-4 xl:grid-cols-[minmax(0,1.3fr)_minmax(320px,1fr)]">
            <Card className="border-border/60 bg-background/30">
              <CardHeader>
                <CardTitle>Safe next steps</CardTitle>
              </CardHeader>
              <CardContent className="space-y-3">
                {output.next_steps.length ? (
                  output.next_steps.map((step, index) => (
                    <div key={`${step.title}-${index}`} className="rounded-2xl border border-border/60 bg-card/60 p-4">
                      <div className="mb-2 flex flex-wrap items-center gap-2">
                        <p className="font-medium">{step.title}</p>
                        <Badge variant="success">{step.safety || "read_only"}</Badge>
                      </div>
                      {step.reason ? <p className="text-sm text-muted-foreground">{step.reason}</p> : null}
                      {step.command ? (
                        <pre className="mt-3 overflow-x-auto rounded-xl border border-border/70 bg-background/70 p-3 text-xs text-primary">
                          <code>{step.command}</code>
                        </pre>
                      ) : null}
                    </div>
                  ))
                ) : (
                  <p className="text-sm text-muted-foreground">No next steps proposed.</p>
                )}
              </CardContent>
            </Card>

            <div className="space-y-4">
              <Card className="border-border/60 bg-background/30">
                <CardHeader>
                  <CardTitle>Evidence</CardTitle>
                </CardHeader>
                <CardContent className="space-y-2">
                  {output.evidence.length ? (
                    output.evidence.map((item) => (
                      <div key={`${item.type}-${item.id}`} className="rounded-xl border border-border/60 bg-card/60 p-3">
                        <div className="mb-1 flex items-center gap-2">
                          <Badge variant="outline">{item.type}</Badge>
                          <span className="text-xs font-medium text-muted-foreground">{item.id}</span>
                        </div>
                        <p className="text-sm">{item.summary}</p>
                      </div>
                    ))
                  ) : (
                    <p className="text-sm text-muted-foreground">No evidence cited.</p>
                  )}
                </CardContent>
              </Card>

              <Card className="border-border/60 bg-background/30">
                <CardHeader>
                  <CardTitle>Uncertainty</CardTitle>
                </CardHeader>
                <CardContent className="space-y-2 text-sm text-muted-foreground">
                  {output.uncertainty.length ? (
                    output.uncertainty.map((item, index) => <p key={index}>• {item}</p>)
                  ) : (
                    <p>No uncertainty notes were included.</p>
                  )}
                </CardContent>
              </Card>

              <Card className="border-border/60 bg-background/30">
                <CardHeader>
                  <CardTitle>Missing evidence</CardTitle>
                </CardHeader>
                <CardContent className="space-y-2 text-sm text-muted-foreground">
                  {output.missing_evidence.length ? (
                    output.missing_evidence.map((item, index) => <p key={index}>• {item}</p>)
                  ) : (
                    <p>No missing-evidence gaps were reported.</p>
                  )}
                </CardContent>
              </Card>

              <Card className="border-border/60 bg-background/30">
                <CardHeader>
                  <CardTitle>Citations</CardTitle>
                </CardHeader>
                <CardContent className="flex flex-wrap gap-2">
                  {output.citations.length ? (
                    output.citations.map((citation, index) => (
                      <Badge key={`${citation}-${index}`} variant="outline">
                        {citation}
                      </Badge>
                    ))
                  ) : (
                    <p className="text-sm text-muted-foreground">No citations were attached.</p>
                  )}
                </CardContent>
              </Card>
            </div>
          </div>

          {result.audit ? (
            <div className="grid gap-4 xl:grid-cols-[minmax(0,1.1fr)_minmax(320px,0.9fr)]">
              <Card className="border-border/60 bg-background/30">
                <CardHeader>
                  <CardTitle>Persisted investigation</CardTitle>
                </CardHeader>
                <CardContent className="space-y-3 text-sm">
                  {result.audit.explanation ? (
                    <>
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge variant="outline">{result.audit.explanation.quality}</Badge>
                        <Badge variant="outline">{result.audit.explanation.model_used}</Badge>
                        <span className="text-muted-foreground">
                          stored {formatRelativeDate(result.audit.explanation.created_at)}
                        </span>
                      </div>
                      <p className="font-medium">{result.audit.explanation.summary}</p>
                      {result.audit.explanation.actions.length ? (
                        <div className="space-y-1 text-muted-foreground">
                          {result.audit.explanation.actions.slice(0, 3).map((action, index) => (
                            <p key={index}>• {action}</p>
                          ))}
                        </div>
                      ) : null}
                    </>
                  ) : (
                    <p className="text-muted-foreground">No persisted explanation is attached to this investigation yet.</p>
                  )}
                  {result.audit.latest_trace ? (
                    <div className="rounded-xl border border-border/60 bg-card/60 p-3 text-muted-foreground">
                      <p className="font-medium text-foreground">Latest trace</p>
                      <p>{result.audit.latest_trace.trace_kind}</p>
                      <p>stored {formatRelativeDate(result.audit.latest_trace.created_at)}</p>
                    </div>
                  ) : null}
                </CardContent>
              </Card>

              <Card className="border-border/60 bg-background/30">
                <CardHeader>
                  <CardTitle>Audit trail</CardTitle>
                </CardHeader>
                <CardContent className="space-y-3 text-sm">
                  {result.audit.state_log?.length ? (
                    result.audit.state_log.slice(0, 5).map((entry, index) => (
                      <div key={`${entry.changed_at ?? "state"}-${index}`} className="rounded-xl border border-border/60 bg-card/60 p-3">
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
                    <p className="text-muted-foreground">No audit state transitions were recorded.</p>
                  )}
                  {result.audit.feedback?.length ? (
                    <div className="rounded-xl border border-border/60 bg-card/60 p-3">
                      <p className="font-medium text-foreground">Operator feedback</p>
                      <p className="mt-1 text-muted-foreground">
                        {result.audit.feedback.length} feedback item{result.audit.feedback.length === 1 ? "" : "s"} recorded.
                      </p>
                    </div>
                  ) : null}
                </CardContent>
              </Card>
            </div>
          ) : null}
        </CardContent>
      </Card>

      {showRaw ? (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Brain className="size-4" />
              Raw diagnostics
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            {result.trace || result.audit?.latest_trace ? (
              <details className="rounded-2xl border border-border/60 bg-background/40 p-4">
                <summary className="cursor-pointer text-sm font-medium text-muted-foreground">Prompt trace</summary>
                <JsonInspector
                  className="mt-3"
                  data={result.trace ?? result.audit?.latest_trace}
                  defaultRaw={showRaw}
                  title="Prompt trace"
                />
              </details>
            ) : null}
            {result.bundle ? (
              <details className="rounded-2xl border border-border/60 bg-background/40 p-4">
                <summary className="cursor-pointer text-sm font-medium text-muted-foreground">Evidence bundle</summary>
                <JsonInspector className="mt-3" data={result.bundle} defaultRaw={showRaw} title="Evidence bundle" />
              </details>
            ) : null}
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}

function Section({ title, items, empty }: { title: string; items: string[]; empty: string }) {
  return (
    <Card className="border-border/60 bg-background/30">
      <CardHeader>
        <CardTitle>{title}</CardTitle>
      </CardHeader>
      <CardContent>
        {items.length ? (
          <ul className="space-y-2 text-sm leading-6">
            {items.map((item, index) => (
              <li key={index} className="text-foreground">
                • {item}
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted-foreground">{empty}</p>
        )}
      </CardContent>
    </Card>
  );
}

