import { useEffect, useState } from "react";

export type Theme = "light" | "dark";

const THEME_KEY = "inferra.theme";

export function useTheme(initial: Theme = "light"): [Theme, (theme: Theme) => void] {
  const [theme, setTheme] = useState<Theme>(() => {
    if (typeof window === "undefined") return initial;
    const stored = window.localStorage.getItem(THEME_KEY);
    const resolved = stored === "dark" || stored === "light" ? stored : initial;
    document.documentElement.dataset.theme = resolved;
    return resolved;
  });

  useEffect(() => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(THEME_KEY, theme);
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  return [theme, setTheme];
}
