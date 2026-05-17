import { Waypoints } from "lucide-react";
import { Link } from "react-router-dom";

import type { TraceSummary } from "@/api";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { formatDisplayValue, formatRelativeDate, formatSeverityLabel } from "@/lib/format";
import { buildTracePath, shortTraceId } from "@/lib/observability";
import { cn } from "@/lib/utils";

type TraceSummaryInlineProps = {
  summary?: TraceSummary | null;
  context?: Record<string, string | number | undefined>;
  className?: string;
  emptyLabel?: string;
  openLabel?: string;
  showMessage?: boolean;
};

export function TraceSummaryInline({
  summary,
  context,
  className,
  emptyLabel = "No trace",
  openLabel = "Open trace",
  showMessage = false,
}: TraceSummaryInlineProps) {
  if (!summary) {
    return <span className={cn("text-sm text-muted-foreground", className)}>{emptyLabel}</span>;
  }

  return (
    <div className={cn("space-y-2", className)}>
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant="info" className="font-mono">
          {shortTraceId(summary.trace_id)}
        </Badge>
        <Badge variant="outline">{summary.event_count} row{summary.event_count === 1 ? "" : "s"}</Badge>
        {summary.source_type ? <Badge variant="outline">{formatDisplayValue(summary.source_type)}</Badge> : null}
        {summary.severity != null ? (
          <Badge variant="outline">Sev {formatSeverityLabel(summary.severity)}</Badge>
        ) : null}
        <span className="text-xs text-muted-foreground">{formatRelativeDate(summary.last_seen_at)}</span>
        <Button variant="outline" size="sm" asChild>
          <Link to={buildTracePath(summary.trace_id, context)}>
            <Waypoints className="size-4" />
            {openLabel}
          </Link>
        </Button>
      </div>
      {showMessage && summary.sample_message ? (
        <p className="text-sm text-muted-foreground">{summary.sample_message}</p>
      ) : null}
    </div>
  );
}
