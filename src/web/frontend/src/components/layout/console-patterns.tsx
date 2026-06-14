import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export function FilterBar({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return <div className={cn("flex flex-wrap gap-1 border-b border-border pb-4", className)}>{children}</div>;
}

export function FilterChip({
  active,
  children,
  onClick,
}: {
  active: boolean;
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "rounded-sm border px-2.5 py-1 text-xs font-medium transition-colors",
        active
          ? "border-foreground bg-foreground text-background"
          : "border-border bg-card text-muted-foreground hover:border-foreground/40 hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}

export function DataRow({
  label,
  value,
  hint,
  mono,
}: {
  label: string;
  value: ReactNode;
  hint?: string;
  mono?: boolean;
}) {
  return (
    <div className="flex items-start justify-between gap-3 border-b border-border py-2 last:border-b-0">
      <span className="text-sm text-muted-foreground">{label}</span>
      <div className="text-right">
        <div className={cn("text-sm font-medium", mono && "font-data")}>{value}</div>
        {hint ? <p className="mt-0.5 text-xs text-muted-foreground">{hint}</p> : null}
      </div>
    </div>
  );
}

export function CodeBlock({ children }: { children: string }) {
  return (
    <pre className="mt-2 overflow-x-auto rounded-sm border border-border bg-panel-inset px-2.5 py-2 font-data text-xs text-foreground">
      <code>{children}</code>
    </pre>
  );
}
