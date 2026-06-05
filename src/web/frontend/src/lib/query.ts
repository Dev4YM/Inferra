import { useCallback, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { type ApiError, errorMessage, fetchJson } from "@/api";

type QueryOptions = {
  deps?: readonly unknown[];
  enabled?: boolean;
  staleTime?: number;
  refetchInterval?: number | false;
};

export function useApiQuery<T>(path: string | null, options: QueryOptions = {}) {
  const { deps = [], enabled = true, staleTime, refetchInterval } = options;
  const queryClient = useQueryClient();
  const [silentReloads, setSilentReloads] = useState(0);
  const queryKey = useMemo(() => ["api", path, ...deps], [deps, path]);
  const query = useQuery<T, ApiError>({
    queryKey,
    enabled: Boolean(path && enabled),
    staleTime,
    refetchInterval,
    queryFn: ({ signal }) => fetchJson<T>(path as string, { signal }),
  });

  const reload = useCallback(
    async (opts?: { silent?: boolean }) => {
      if (!path || !enabled) return undefined;
      if (opts?.silent) setSilentReloads((count) => count + 1);
      try {
        const result = await queryClient.fetchQuery({
          queryKey,
          staleTime: 0,
          queryFn: ({ signal }) => fetchJson<T>(path, { signal }),
        });
        return result;
      } finally {
        if (opts?.silent) setSilentReloads((count) => Math.max(0, count - 1));
      }
    },
    [enabled, path, queryClient, queryKey],
  );

  const setData = useCallback(
    (next: T | null | ((current: T | null) => T | null)) => {
      queryClient.setQueryData<T | null>(queryKey, (current) =>
        typeof next === "function" ? (next as (current: T | null) => T | null)(current ?? null) : next,
      );
    },
    [queryClient, queryKey],
  );

  return {
    data: query.data ?? null,
    error: query.error ?? null,
    errorMessage: query.error ? errorMessage(query.error) : null,
    isLoading: query.isLoading,
    isRefreshing: query.isFetching && !query.isLoading && silentReloads === 0,
    reload,
    setData,
  };
}

export function useApiMutation<TArgs, TResult>(
  fn: (args: TArgs) => Promise<TResult>,
): {
  error: ApiError | null;
  errorMessage: string | null;
  isPending: boolean;
  run: (args: TArgs) => Promise<TResult>;
} {
  const mutation = useMutation<TResult, ApiError, TArgs>({ mutationFn: fn });

  return {
    error: mutation.error ?? null,
    errorMessage: mutation.error ? errorMessage(mutation.error) : null,
    isPending: mutation.isPending,
    run: mutation.mutateAsync,
  };
}
