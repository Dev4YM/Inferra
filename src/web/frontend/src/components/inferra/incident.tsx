import { AlertTriangle, CheckCircle2, ChevronDown, FlaskConical, GitBranch, Lightbulb, ShieldQuestion } from "lucide-react";
import { useState } from "react";
import { Link } from "react-router-dom";

import type { EventRow, HypothesisRow, IncidentRow, InvestigationEvidence, InvestigationStep } from "@/api";
import { Badge } from "@/components/ui/badge";
import { ConfidenceMeter, SeverityIndicator } from "@/components/inferra/health";
import { TimelineView } from "@/components/inferra/timeline";
import { formatDisplayValue, formatRelativeDate, formatSeverityLabel, summarizeEvent } from "@/lib/format";
import { cn } from "@/lib/utils";

export function IncidentCard({ incident }: { incident: IncidentRow }) {
  return (
    <Link
      to={`/incidents/${incident.incident_id}`}
      className="block rounded-2xl border border-border/70 bg-card/70 p-4 text-foreground shadow-sm transition hover:-translate-y-0.5 hover:border-primary/30 hover:opacity-100 hover:shadow-md"
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <SeverityIndicator value={incident.severity} label={`Sev ${formatSeverityLabel(incident.severity)}`} />
            <Badge variant="outline">{formatDisplayValue(incident.state)}</Badge>
          </div>
          <h3 className="mt-3 truncate text-base font-semibold">{incident.primary_service || "Unknown service"}</h3>
          <p className="mt-1 text-sm text-muted-foreground">{incident.incident_id}</p>
        </div>
        <div className="text-right text-sm">
          <p className="font-medium">{incident.event_count ?? 0}</p>
          <p className="text-xs text-muted-foreground">events</p>
        </div>
      </div>
      <div className="mt-4 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        {(incident.affected_services ?? []).slice(0, 4).map((service) => (
          <Badge key={service} variant="outline">
            {service}
          </Badge>
        ))}
        <span>updated {formatRelativeDate(incident.updated_at)}</span>
      </div>
    </Link>
  );
}

export function HypothesisPanel({
  hypothesis,
  rank,
  events,
  defaultOpen,
  advanced,
}: {
  hypothesis: HypothesisRow;
  rank: number;
  events: EventRow[];
  defaultOpen?: boolean;
  advanced?: boolean;
}) {
  const [open, setOpen] = useState(Boolean(defaultOpen));
  const score = typeof hypothesis.total_score === "number" ? Math.max(0, Math.min(1, hypothesis.total_score)) : undefined;
  const learnedImpact = hypothesis.provenance?.estimated_total_impact ?? 0;

  return (
    <div className="rounded-2xl border border-border/70 bg-card/75 shadow-sm">
      <button
        type="button"
        className="flex w-full items-start justify-between gap-4 p-5 text-left"
        onClick={() => setOpen((value) => !value)}
      >
        <div className="min-w-0 space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="info">rank {rank}</Badge>
            {hypothesis.confidence_label ? <Badge variant="outline">{hypothesis.confidence_label}</Badge> : null}
            {learnedImpact > 0 ? <Badge variant="success">learned +{learnedImpact.toFixed(2)}</Badge> : null}
          </div>
          <div>
            <h3 className="text-lg font-semibold tracking-tight">{humanize(hypothesis.cause_type)}</h3>
            {hypothesis.description ? <p className="mt-2 max-w-3xl text-sm leading-6 text-muted-foreground">{hypothesis.description}</p> : null}
          </div>
        </div>
        <ChevronDown className={cn("mt-1 size-5 shrink-0 text-muted-foreground transition", open ? "rotate-180" : "")} />
      </button>

      <div className={cn("grid transition-all", open ? "grid-rows-[1fr]" : "grid-rows-[0fr]")}>
        <div className="overflow-hidden">
          <div className="border-t border-border/60 p-5">
            <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_minmax(280px,0.6fr)]">
              <div className="space-y-4">
                <ConfidenceMeter value={score ?? confidenceFromLabel(hypothesis.confidence_label)} />
                <div className="grid gap-3 md:grid-cols-2">
                  <ExplanationBlock
                    icon={CheckCircle2}
                    title="Evidence supporting"
                    items={events.slice(0, 3).map((event) => summarizeEvent(event))}
                    empty="No direct supporting evidence is attached yet."
                  />
                  <ExplanationBlock
                    icon={AlertTriangle}
                    title="Contradictions"
                    items={advanced ? ["No explicit contradictions were stored for this hypothesis."] : []}
                    empty="No contradiction has been surfaced."
                  />
                </div>
                {hypothesis.suggested_checks?.length ? (
                  <ExplanationBlock icon={Lightbulb} title="Suggested checks" items={hypothesis.suggested_checks} />
                ) : null}
              </div>

              <div className="space-y-4">
                <div className="rounded-2xl border border-border/60 bg-background/35 p-4">
                  <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Why it is plausible</p>
                  <p className="mt-2 text-sm leading-6 text-muted-foreground">
                    Inferra ranks this hypothesis from correlated event severity, service proximity, learned patterns, and evidence density.
                  </p>
                </div>
                {hypothesis.provenance?.artifacts?.length ? (
                  <div className="rounded-2xl border border-border/60 bg-background/35 p-4">
                    <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">Learned signals</p>
                    <div className="mt-3 flex flex-wrap gap-2">
                      {hypothesis.provenance.artifacts.slice(0, 6).map((artifact, index) => (
                        <Badge key={`${artifact.artifact_id ?? artifact.label ?? index}`} variant="outline">
                          {artifact.label ?? artifact.artifact_id ?? "artifact"}
                        </Badge>
                      ))}
                    </div>
                  </div>
                ) : null}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export function EvidenceViewer({
  evidence,
  events,
}: {
  evidence?: InvestigationEvidence[];
  events: EventRow[];
}) {
  return (
    <div className="space-y-4">
      {evidence?.length ? (
        <div className="grid gap-3 md:grid-cols-2">
          {evidence.slice(0, 6).map((item) => (
            <div key={`${item.type}-${item.id}`} className="rounded-2xl border border-border/60 bg-background/35 p-4">
              <div className="mb-2 flex flex-wrap items-center gap-2">
                <Badge variant="outline">{item.type}</Badge>
                <span className="text-xs text-muted-foreground">{item.id}</span>
              </div>
              <p className="text-sm leading-6">{item.summary}</p>
            </div>
          ))}
        </div>
      ) : null}
      <TimelineView events={events} limit={8} />
    </div>
  );
}

export function SuggestedChecks({ checks }: { checks: InvestigationStep[] }) {
  if (!checks.length) {
    return <p className="text-sm text-muted-foreground">No suggested checks are available yet.</p>;
  }

  return (
    <div className="space-y-3">
      {checks.slice(0, 6).map((check, index) => (
        <div key={`${check.title}-${index}`} className="rounded-2xl border border-border/60 bg-background/35 p-4">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="success">{check.safety || "read-only"}</Badge>
            {check.requires_user_action ? <Badge variant="warning">manual</Badge> : null}
          </div>
          <p className="mt-2 font-medium">{check.title}</p>
          {check.reason ? <p className="mt-1 text-sm leading-6 text-muted-foreground">{check.reason}</p> : null}
          {check.command ? (
            <pre className="mt-3 overflow-x-auto rounded-xl border border-border/70 bg-background/75 p-3 text-xs text-primary">
              <code>{check.command}</code>
            </pre>
          ) : null}
        </div>
      ))}
    </div>
  );
}

export function AlternativeHypotheses({ items }: { items: string[] }) {
  if (!items.length) {
    return <p className="text-sm text-muted-foreground">No explicit alternatives were generated.</p>;
  }

  return (
    <div className="space-y-2">
      {items.slice(0, 6).map((item, index) => (
        <div key={index} className="flex gap-3 rounded-2xl border border-border/60 bg-background/35 p-4">
          <ShieldQuestion className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
          <p className="text-sm leading-6 text-muted-foreground">{item}</p>
        </div>
      ))}
    </div>
  );
}

export function ExplanationBlock({
  icon: Icon = FlaskConical,
  title,
  items,
  empty,
}: {
  icon?: typeof GitBranch;
  title: string;
  items: string[];
  empty?: string;
}) {
  return (
    <div className="rounded-2xl border border-border/60 bg-background/35 p-4">
      <div className="flex items-center gap-2">
        <Icon className="size-4 text-primary" />
        <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">{title}</p>
      </div>
      <div className="mt-3 space-y-2">
        {items.length ? (
          items.slice(0, 5).map((item, index) => (
            <p key={index} className="text-sm leading-6 text-muted-foreground">
              {item}
            </p>
          ))
        ) : (
          <p className="text-sm leading-6 text-muted-foreground">{empty ?? "No entries recorded."}</p>
        )}
      </div>
    </div>
  );
}

function humanize(value: string): string {
  return value.replace(/[_-]/g, " ").replace(/\b\w/g, (char) => char.toUpperCase());
}

function confidenceFromLabel(label?: string | null): number {
  const text = String(label ?? "").toLowerCase();
  if (text.includes("high")) return 0.82;
  if (text.includes("medium")) return 0.58;
  if (text.includes("low")) return 0.32;
  return 0.45;
}
