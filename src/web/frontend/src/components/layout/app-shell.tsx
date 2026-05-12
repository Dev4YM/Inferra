import { Menu, Moon, Sun, X } from "lucide-react";
import { type ComponentType, type ReactNode, useState } from "react";
import { NavLink } from "react-router-dom";

import type { Mode } from "@/lib/experience";
import type { Theme } from "@/lib/theme";
import { formatDisplayValue, formatModeLabel } from "@/lib/format";
import { cn } from "@/lib/utils";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

export type NavItem = {
  to: string;
  label: string;
  icon: ComponentType<{ className?: string }>;
};

const MODES: Mode[] = ["operator", "expert", "developer"];

export function AppShell({
  navItems,
  children,
  mode,
  onModeChange,
  modeStatus,
  theme,
  onThemeChange,
}: {
  navItems: NavItem[];
  children: ReactNode;
  mode: Mode;
  onModeChange: (mode: Mode) => void;
  modeStatus?: string;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
}) {
  const [open, setOpen] = useState(false);

  return (
    <div className="min-h-screen lg:grid lg:h-screen lg:grid-cols-[288px_minmax(0,1fr)] lg:overflow-hidden">
      <div className="flex items-center justify-between border-b border-border/70 bg-background/70 px-4 py-3 backdrop-blur lg:hidden">
        <div>
          <p className="text-sm font-semibold uppercase tracking-[0.28em] text-primary/80">Inferra</p>
          <p className="text-xs text-muted-foreground">runtime intelligence control plane</p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="icon"
            type="button"
            aria-label={theme === "light" ? "Switch to dark theme" : "Switch to light theme"}
            onClick={() => onThemeChange(theme === "light" ? "dark" : "light")}
          >
            {theme === "light" ? <Moon className="size-4" /> : <Sun className="size-4" />}
          </Button>
          <Button variant="outline" size="icon" type="button" aria-label="Toggle navigation" onClick={() => setOpen((v) => !v)}>
          {open ? <X className="size-4" /> : <Menu className="size-4" />}
          </Button>
        </div>
      </div>

      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-40 w-[288px] transform border-r border-border/80 bg-background/95 p-4 shadow-2xl backdrop-blur-xl transition-transform duration-200 lg:static lg:h-screen lg:translate-x-0 lg:overflow-hidden",
          open ? "translate-x-0" : "-translate-x-full lg:translate-x-0",
        )}
      >
        <div className="glass-panel flex h-full max-h-full flex-col overflow-hidden rounded-2xl border border-border/70 p-4">
          <div className="space-y-2">
            <p className="text-sm font-semibold uppercase tracking-[0.32em] text-primary/90">Inferra</p>
            <div>
              <h1 className="text-2xl font-semibold">Control Plane</h1>
              <p className="text-sm text-muted-foreground">Local-first runtime investigation with safe AI guidance.</p>
            </div>
          </div>

          <div className="mt-6 space-y-4 rounded-2xl border border-border/60 bg-background/35 p-3">
            <div className="flex items-center justify-between">
              <p className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">Experience</p>
              <Badge variant="outline">{formatModeLabel(mode)}</Badge>
            </div>
            <div className="grid grid-cols-3 gap-1.5">
              {MODES.map((value) => (
                <Button
                  key={value}
                  variant={value === mode ? "default" : "outline"}
                  size="sm"
                  type="button"
                  aria-pressed={value === mode}
                  onClick={() => onModeChange(value)}
                >
                  {formatModeLabel(value)}
                </Button>
              ))}
            </div>
            <div className="flex items-center justify-between">
              <p className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">Appearance</p>
              <Badge variant="outline">{formatDisplayValue(theme)}</Badge>
            </div>
            <div className="grid grid-cols-2 gap-1.5">
              <Button
                variant={theme === "light" ? "default" : "outline"}
                size="sm"
                type="button"
                aria-pressed={theme === "light"}
                onClick={() => onThemeChange("light")}
              >
                <Sun className="size-4" />
                Light
              </Button>
              <Button
                variant={theme === "dark" ? "default" : "outline"}
                size="sm"
                type="button"
                aria-pressed={theme === "dark"}
                onClick={() => onThemeChange("dark")}
              >
                <Moon className="size-4" />
                Dark
              </Button>
            </div>
            {modeStatus ? <p className="text-xs leading-5 text-muted-foreground">{modeStatus}</p> : null}
          </div>

          <nav className="mt-6 min-h-0 flex-1 space-y-1 overflow-y-auto pr-1">
            {navItems.map((item) => {
              const Icon = item.icon;
              return (
                <NavLink
                  key={item.to}
                  end={item.to === "/"}
                  onClick={() => setOpen(false)}
                  to={item.to}
                  className={({ isActive }) =>
                    cn(
                      "flex items-center gap-3 rounded-xl px-3.5 py-2.5 text-sm font-medium transition hover:bg-secondary/80 hover:text-foreground",
                      isActive
                        ? "bg-primary/10 text-foreground shadow-[inset_0_0_0_1px_var(--ring)]"
                        : "text-muted-foreground",
                    )
                  }
                >
                  <Icon className="size-4" />
                  <span>{item.label}</span>
                </NavLink>
              );
            })}
          </nav>

          <div className="mt-auto rounded-2xl border border-border/70 bg-secondary/40 p-4 text-xs leading-6 text-muted-foreground">
            Local-first and read-only toward observed systems.
            <br />
            AI suggests, retries when needed, and never executes commands.
          </div>
        </div>
      </aside>

      {open ? <button type="button" aria-label="Close navigation overlay" className="fixed inset-0 z-30 bg-black/40 lg:hidden" onClick={() => setOpen(false)} /> : null}

      <main className="min-w-0 px-4 py-5 md:px-6 lg:h-screen lg:overflow-hidden lg:px-8 lg:py-8">
        <div className="mx-auto h-full max-w-[1680px] overflow-hidden rounded-2xl border border-border/70 bg-background/70 p-4 shadow-[0_24px_90px_-56px_rgba(15,23,42,0.55)] backdrop-blur md:p-5">
          <div className="h-full overflow-y-auto pr-1">
            <div className="mx-auto max-w-[1520px] pb-4">{children}</div>
          </div>
        </div>
      </main>
    </div>
  );
}
