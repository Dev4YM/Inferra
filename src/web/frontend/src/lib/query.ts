import { useCallback, useEffect, useRef, useState } from "react";

import { type ApiError, errorMessage, fetchJson } from "@/api";

type QueryOptions = {
  deps?: readonly unknown[];
  enabled?: boolean;
};

export function useApiQuery<T>(path: string | null, options: QueryOptions = {}) {
  const { deps = [], enabled = true } = options;
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<ApiError | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const abortRef = useRef<AbortController | null>(null);
  const mountedRef = useRef(true);
  const hasLoadedRef = useRef(false);

  useEffect(() => {
    return () => {
      mountedRef.current = false;
      abortRef.current?.abort();
    };
  }, []);

  const reload = useCallback(
    async (opts?: { silent?: boolean }) => {
      if (!path || !enabled) return;
      abortRef.current?.abort();
      const controller = new AbortController();
      abortRef.current = controller;
      const showRefreshing = Boolean(hasLoadedRef.current || opts?.silent);
      if (showRefreshing) {
        setIsRefreshing(true);
      } else {
        setIsLoading(true);
      }
      try {
        const next = await fetchJson<T>(path, { signal: controller.signal });
        if (!mountedRef.current || controller.signal.aborted) return;
        hasLoadedRef.current = true;
        setData(next);
        setError(null);
        return next;
      } catch (err) {
        if (!mountedRef.current || controller.signal.aborted) return;
        setError(err as ApiError);
      } finally {
        if (!mountedRef.current || controller.signal.aborted) return;
        setIsLoading(false);
        setIsRefreshing(false);
      }
    },
    [enabled, path],
  );

  useEffect(() => {
    if (!path || !enabled) return;
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path, enabled, ...deps]);

  return {
    data,
    error,
    errorMessage: error ? errorMessage(error) : null,
    isLoading,
    isRefreshing,
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
  const [isPending, setIsPending] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);

  const run = useCallback(
    async (args: TArgs) => {
      setIsPending(true);
      setError(null);
      try {
        return await fn(args);
      } catch (err) {
        setError(err as ApiError);
        throw err;
      } finally {
        setIsPending(false);
      }
    },
    [fn],
  );

  return {
    error,
    errorMessage: error ? errorMessage(error) : null,
    isPending,
    run,
  };
}

