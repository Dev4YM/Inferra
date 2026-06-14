import type { ReactNode } from "react";

import { formatModeLabel } from "@/lib/format";
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
    <header className="mb-6 border-b border-border pb-5">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0 space-y-1">
          {eyebrow ? <p className="label-caps">{eyebrow}</p> : null}
          <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
            <h1 className="text-2xl font-semibold tracking-tight text-foreground">{title}</h1>
            {mode ? (
              <span className="font-data text-xs text-muted-foreground">{formatModeLabel(mode)}</span>
            ) : null}
          </div>
          {subtitle ? <p className="max-w-3xl text-sm text-muted-foreground">{subtitle}</p> : null}
        </div>
        {actions ? <div className={cn("flex flex-wrap items-center gap-2")}>{actions}</div> : null}
      </div>
    </header>
  );
}
