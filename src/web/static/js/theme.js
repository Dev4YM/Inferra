const STORAGE_KEY = "inferra-theme";

export function getStoredTheme() {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return v === "dark" || v === "light" ? v : null;
  } catch {
    return null;
  }
}

export function applyTheme(mode) {
  const root = document.documentElement;
  if (mode === "dark") {
    root.classList.add("dark");
  } else {
    root.classList.remove("dark");
  }
  try {
    localStorage.setItem(STORAGE_KEY, mode);
  } catch {
    /* ignore */
  }
}

export function initTheme(toggleEl) {
  const stored = getStoredTheme();
  const prefersDark = window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
  applyTheme(stored || (prefersDark ? "dark" : "light"));
  syncToggle(toggleEl);
}

function syncToggle(toggleEl) {
  if (!toggleEl) return;
  const dark = document.documentElement.classList.contains("dark");
  toggleEl.textContent = dark ? "Light theme" : "Dark theme";
  toggleEl.setAttribute("aria-pressed", dark ? "true" : "false");
}

export function cycleTheme(toggleEl) {
  const next = document.documentElement.classList.contains("dark") ? "light" : "dark";
  applyTheme(next);
  syncToggle(toggleEl);
}
