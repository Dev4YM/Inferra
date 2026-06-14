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
      <CardContent className="flex items-center gap-3 p-4">
        <LoaderCircle className="size-5 shrink-0 animate-spin text-muted-foreground" />
        <div className="space-y-0.5">
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
      <CardContent className="flex flex-col items-center gap-3 p-8 text-center">
        <SearchX className="size-5 text-muted-foreground" />
        <div className="space-y-1">
          <p className="font-medium">{title}</p>
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
          <CardContent className="space-y-3 p-4">
            <Skeleton className="h-3 w-24" />
            <Skeleton className="h-8 w-20" />
            <Skeleton className="h-3 w-full" />
          </CardContent>
        </Card>
      ))}
    </div>
  );
}
