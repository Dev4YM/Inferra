import type { ReactNode } from "react";
import { AlertTriangle, LoaderCircle, RefreshCcw, SearchX } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";

export function LoadingState({
  title = "Loading",
  description = "Fetching the latest control-plane snapshot…",
}: {
  title?: string;
  description?: string;
}) {
  return (
    <Card>
      <CardContent className="flex items-center gap-4 p-6">
        <div className="rounded-2xl border border-border/80 bg-secondary/60 p-3">
          <LoaderCircle className="size-5 animate-spin text-primary" />
        </div>
        <div className="space-y-1">
          <p className="font-medium">{title}</p>
          <p className="text-sm text-muted-foreground">{description}</p>
        </div>
      </CardContent>
    </Card>
  );
}

export function ErrorState({
  title = "Something went wrong",
  description,
  onRetry,
}: {
  title?: string;
  description: string;
  onRetry?: () => void;
}) {
  return (
    <Alert variant="destructive">
      <div className="flex items-start justify-between gap-4">
        <div className="flex gap-3">
          <AlertTriangle className="mt-0.5 size-5" />
          <div>
            <AlertTitle>{title}</AlertTitle>
            <AlertDescription>{description}</AlertDescription>
          </div>
        </div>
        {onRetry ? (
          <Button variant="outline" size="sm" onClick={onRetry}>
            <RefreshCcw className="size-4" />
            Retry
          </Button>
        ) : null}
      </div>
    </Alert>
  );
}

export function EmptyState({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <Card className="border-dashed">
      <CardContent className="flex flex-col items-center gap-3 p-10 text-center">
        <div className="rounded-full border border-border/70 bg-secondary/60 p-4">
          <SearchX className="size-5 text-muted-foreground" />
        </div>
        <div className="space-y-1">
          <p className="text-lg font-medium">{title}</p>
          <p className="max-w-xl text-sm text-muted-foreground">{description}</p>
        </div>
        {action}
      </CardContent>
    </Card>
  );
}

export function MetricGridSkeleton() {
  return (
    <div className="dashboard-grid">
      {Array.from({ length: 4 }).map((_, index) => (
        <Card key={index}>
          <CardContent className="space-y-4 p-6">
            <Skeleton className="h-4 w-28" />
            <Skeleton className="h-9 w-24" />
            <Skeleton className="h-4 w-full" />
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

