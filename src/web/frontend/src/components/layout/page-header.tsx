import type { ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

export function PageHeader({
  title,
  subtitle,
  eyebrow,
  mode,
  actions,
}: {
  title: string;
  subtitle?: string;
  eyebrow?: string;
  mode?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="mb-6 flex flex-wrap items-start justify-between gap-4">
      <div className="space-y-2">
        {eyebrow ? <p className="text-xs font-semibold uppercase tracking-[0.25em] text-primary/80">{eyebrow}</p> : null}
        <div className="flex flex-wrap items-center gap-3">
          <h1 className="text-3xl font-semibold tracking-tight text-foreground md:text-4xl">{title}</h1>
          {mode ? (
            <Badge
              variant={mode === "operator" ? "success" : mode === "expert" ? "warning" : "info"}
              className={cn("px-3 py-1 text-[10px]")}
            >
              {mode} mode
            </Badge>
          ) : null}
        </div>
        {subtitle ? <p className="max-w-3xl text-sm leading-6 text-muted-foreground">{subtitle}</p> : null}
      </div>
      {actions ? <div className="flex flex-wrap items-center gap-3">{actions}</div> : null}
    </div>
  );
}

