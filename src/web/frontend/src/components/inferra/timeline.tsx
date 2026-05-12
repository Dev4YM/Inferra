import { Clock3 } from "lucide-react";

import type { EventRow } from "@/api";
import { Badge } from "@/components/ui/badge";
import { formatDisplayValue, formatRelativeDate, summarizeEvent } from "@/lib/format";
import { riskTone, SeverityIndicator } from "@/components/inferra/health";

export function TimelineView({
  events,
  limit = 8,
  compact = false,
}: {
  events: EventRow[];
  limit?: number;
  compact?: boolean;
}) {
  const visible = events.slice(0, limit);

  if (!visible.length) {
    return <p className="text-sm text-muted-foreground">No timeline events are available yet.</p>;
  }

  return (
    <div className="relative space-y-3">
      <div className="absolute bottom-4 left-4 top-4 w-px bg-border" />
      {visible.map((event, index) => {
        const tone = riskTone(event.severity);
        const dotClass =
          tone === "destructive"
            ? "bg-critical"
            : tone === "warning"
              ? "bg-warning"
              : tone === "success"
                ? "bg-success"
                : "bg-primary";
        return (
          <div key={`${event.event_id ?? "event"}-${index}`} className="relative grid gap-3 pl-10">
            <span className={`absolute left-[11px] top-3 size-2.5 rounded-full ring-4 ring-card ${dotClass}`} />
            <div className="rounded-2xl border border-border/60 bg-background/35 p-4">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <div className="flex flex-wrap items-center gap-2">
                  <SeverityIndicator value={event.severity} />
                  {event.service_id ? <Badge variant="outline">{event.service_id}</Badge> : null}
                  {event.source_ref?.source_type ? <Badge variant="outline">{formatDisplayValue(event.source_ref.source_type)}</Badge> : null}
                </div>
                <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
                  <Clock3 className="size-3.5" />
                  {formatRelativeDate(event.timestamp)}
                </span>
              </div>
              <p className={compact ? "mt-2 text-sm" : "mt-3 text-sm leading-6"}>{summarizeEvent(event)}</p>
              {!compact && event.tags?.length ? (
                <div className="mt-3 flex flex-wrap gap-2">
                  {event.tags.slice(0, 5).map((tag) => (
                    <Badge key={tag} variant="outline">
                      {tag}
                    </Badge>
                  ))}
                </div>
              ) : null}
            </div>
          </div>
        );
      })}
    </div>
  );
}

export function IncidentStateTimeline({
  states,
}: {
  states?: Array<{ old_state?: string; new_state?: string; changed_at?: string; reason?: string }>;
}) {
  if (!states?.length) {
    return <p className="text-sm text-muted-foreground">No state transitions have been recorded.</p>;
  }

  return (
    <div className="space-y-3">
      {states.slice(0, 8).map((entry, index) => (
        <div key={`${entry.changed_at ?? "state"}-${index}`} className="rounded-2xl border border-border/60 bg-background/35 p-4">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant="outline">{formatDisplayValue(entry.old_state ?? "unknown")}</Badge>
            <span className="text-xs text-muted-foreground">to</span>
            <Badge variant="info">{formatDisplayValue(entry.new_state ?? "unknown")}</Badge>
            <span className="text-xs text-muted-foreground">{formatRelativeDate(entry.changed_at)}</span>
          </div>
          {entry.reason ? <p className="mt-2 text-sm text-muted-foreground">{entry.reason}</p> : null}
        </div>
      ))}
    </div>
  );
}
