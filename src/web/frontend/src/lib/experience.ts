import { useEffect, useState } from "react";

import type { InferraConfigPayload } from "@/api";

export type Mode = "operator" | "expert" | "developer";

const MODE_KEY = "inferra.mode";

export function useMode(initial: Mode = "operator"): [Mode, (mode: Mode) => void] {
  const [mode, setMode] = useState<Mode>(() => {
    if (typeof window === "undefined") return initial;
    const stored = window.localStorage.getItem(MODE_KEY);
    if (stored === "operator" || stored === "expert" || stored === "developer") return stored;
    return initial;
  });
  useEffect(() => {
    if (typeof window !== "undefined") {
      window.localStorage.setItem(MODE_KEY, mode);
    }
  }, [mode]);
  return [mode, setMode];
}

export function isAdvancedMode(mode: Mode): boolean {
  return mode === "expert" || mode === "developer";
}

export function configMode(config: InferraConfigPayload | null | undefined): Mode | null {
  const value = config?.experience?.mode;
  return value === "operator" || value === "expert" || value === "developer" ? value : null;
}

