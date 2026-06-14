import { useMemo } from "react";

import { type ApiError, type HealthResponse, type ProbeHealthResponse, fetchJson } from "@/api";
import { useApiQuery } from "@/lib/query";

export const INFERRA_HEALTH_QUERY_KEY = "/api/health";
export const INFERRA_READYZ_QUERY_KEY = "/readyz";

export type InferraRuntimeState =
  | "loading"
  | "online"
  | "degraded"
  | "auth_required"
  | "offline";

export type InferraRuntimeSnapshot = {
  state: InferraRuntimeState;
  health: HealthResponse | null;
  ready: ProbeHealthResponse | null;
  error: ApiError | null;
  errorMessage: string | null;
  isRefreshing: boolean;
  reload: () => Promise<HealthResponse | ProbeHealthResponse | undefined>;
};

function deriveRuntimeState(
  healthError: ApiError | null,
  health: HealthResponse | null,
  ready: ProbeHealthResponse | null,
  isLoading: boolean,
): InferraRuntimeState {
  if (isLoading && !health && !ready) return "loading";
  if (healthError?.status === 401 || healthError?.status === 403 || healthError?.status === 503) {
    return "auth_required";
  }
  if (healthError?.status === 0 || (!health && !ready && healthError)) return "offline";
  if (health?.status === "degraded" || health?.storage_writes_ok === false) return "degraded";
  if (ready?.status === "not_ready" || ready?.storage_writes_ok === false) return "degraded";
  if (health?.status === "ok" || ready?.status === "ready" || ready?.status === "ok") return "online";
  return health || ready ? "online" : "offline";
}

export function useInferraRuntime(): InferraRuntimeSnapshot {
  const healthQuery = useApiQuery<HealthResponse>(INFERRA_HEALTH_QUERY_KEY, {
    staleTime: 5_000,
    refetchInterval: 15_000,
  });

  const error = healthQuery.error;
  const state = useMemo(
    () =>
      deriveRuntimeState(
        error,
        healthQuery.data,
        null,
        healthQuery.isLoading && !healthQuery.data,
      ),
    [error, healthQuery.data, healthQuery.isLoading],
  );

  return {
    state,
    health: healthQuery.data,
    ready: null,
    error,
    errorMessage: healthQuery.errorMessage,
    isRefreshing: healthQuery.isRefreshing,
    reload: async () => healthQuery.reload({ silent: true }),
  };
}

export async function probeInferraHealth(baseUrl: string): Promise<ProbeHealthResponse> {
  const url = `${baseUrl.replace(/\/$/, "")}/healthz`;
  return fetchJson<ProbeHealthResponse>(url.startsWith("http") ? url : `http://${url}`);
}

export function runtimeStateLabel(state: InferraRuntimeState): string {
  switch (state) {
    case "loading":
      return "Checking API…";
    case "online":
      return "API online";
    case "degraded":
      return "Runtime degraded";
    case "auth_required":
      return "Auth required";
    case "offline":
      return "API offline";
    default:
      return "Unknown";
  }
}
