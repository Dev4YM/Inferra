import { Code2, Copy, Sparkles } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export function JsonInspector({
  data,
  title = "Structured data",
  defaultRaw = false,
  className,
}: {
  data: unknown;
  title?: string;
  defaultRaw?: boolean;
  className?: string;
}) {
  const [showRaw, setShowRaw] = useState(defaultRaw);
  const rawText = useMemo(() => JSON.stringify(data, null, 2) ?? "null", [data]);

  useEffect(() => {
    setShowRaw(defaultRaw);
  }, [defaultRaw]);

  const copyRaw = async () => {
    try {
      await navigator.clipboard.writeText(rawText);
      toast.success("JSON copied to clipboard.");
    } catch (error) {
      toast.error("Could not copy JSON", { description: error instanceof Error ? error.message : String(error) });
    }
  };

  return (
    <div className={cn("rounded-md border border-border bg-background/40 p-4", className)}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="space-y-1">
          <p className="text-sm font-semibold">{title}</p>
          <p className="text-xs text-muted-foreground">{describeJson(data)}</p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            aria-pressed={showRaw}
            onClick={() => setShowRaw((current) => !current)}
          >
            {showRaw ? <Sparkles className="size-4" /> : <Code2 className="size-4" />}
            {showRaw ? "Friendly view" : "Raw JSON"}
          </Button>
          <Button type="button" variant="outline" size="sm" onClick={() => void copyRaw()}>
            <Copy className="size-4" />
            Copy JSON
          </Button>
        </div>
      </div>

      <div className="mt-4">
        {showRaw ? (
          <pre className="overflow-auto rounded-xl border border-border bg-background/75 p-4 text-xs text-primary">
            <code>{rawText}</code>
          </pre>
        ) : (
          <FriendlyNode value={data} depth={0} />
        )}
      </div>
    </div>
  );
}

function FriendlyNode({ value, depth }: { value: unknown; depth: number }) {
  if (value === null || value === undefined || typeof value === "string" || typeof value === "number" || typeof value === "boolean") {
    return <PrimitiveValue value={value} />;
  }

  if (depth >= 4) {
    return <p className="text-sm text-muted-foreground">{describeJson(value)}</p>;
  }

  if (Array.isArray(value)) {
    if (!value.length) {
      return <p className="text-sm text-muted-foreground">Empty list.</p>;
    }

    const primitiveOnly = value.every(
      (item) => item === null || item === undefined || typeof item === "string" || typeof item === "number" || typeof item === "boolean",
    );

    if (primitiveOnly) {
      return (
        <div className="flex flex-wrap gap-2">
          {value.map((item, index) => (
            <Badge key={`${String(item)}-${index}`} variant="outline">
              {formatPrimitive(item)}
            </Badge>
          ))}
        </div>
      );
    }

    const preview = value.slice(0, 8);
    return (
      <div className="space-y-3">
        {preview.map((item, index) => (
          <div key={index} className="rounded-xl border border-border bg-card/60 p-3">
            <p className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">Item {index + 1}</p>
            <div className="mt-2">
              <FriendlyNode value={item} depth={depth + 1} />
            </div>
          </div>
        ))}
        {value.length > preview.length ? (
          <p className="text-xs text-muted-foreground">+ {value.length - preview.length} more items are available in raw JSON.</p>
        ) : null}
      </div>
    );
  }

  const entries = Object.entries(value as Record<string, unknown>);
  if (!entries.length) {
    return <p className="text-sm text-muted-foreground">Empty object.</p>;
  }

  return (
    <dl className="space-y-3">
      {entries.map(([key, entryValue]) => (
        <div key={key} className="rounded-xl border border-border bg-card/60 p-3">
          <dt className="text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">{humanizeKey(key)}</dt>
          <dd className="mt-2">
            <FriendlyNode value={entryValue} depth={depth + 1} />
          </dd>
        </div>
      ))}
    </dl>
  );
}

function PrimitiveValue({ value }: { value: unknown }) {
  if (typeof value === "string") {
    return <p className="break-words text-sm leading-6 text-foreground">{value || "Empty string"}</p>;
  }

  return (
    <span className="inline-flex rounded-lg border border-border bg-secondary/60 px-2.5 py-1 font-mono text-xs text-foreground">
      {formatPrimitive(value)}
    </span>
  );
}

function describeJson(value: unknown): string {
  if (Array.isArray(value)) {
    return `${value.length} item${value.length === 1 ? "" : "s"} in this list`;
  }
  if (value && typeof value === "object") {
    const keys = Object.keys(value as Record<string, unknown>);
    return `${keys.length} field${keys.length === 1 ? "" : "s"} in this object`;
  }
  return `Single ${value === null ? "null" : typeof value} value`;
}

function humanizeKey(key: string): string {
  const withSpaces = key.replace(/[_-]+/g, " ").replace(/([a-z0-9])([A-Z])/g, "$1 $2");
  return withSpaces.charAt(0).toUpperCase() + withSpaces.slice(1);
}

function formatPrimitive(value: unknown): string {
  if (value === null) return "null";
  if (value === undefined) return "undefined";
  if (typeof value === "string") return value || '""';
  return String(value);
}
