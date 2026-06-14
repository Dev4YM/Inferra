import { Menu, Moon, Sun, X } from "lucide-react";
import { type ComponentType, type ReactNode, useState } from "react";
import { NavLink, useLocation } from "react-router-dom";

import type { Mode } from "@/lib/experience";
import type { InferraRuntimeSnapshot } from "@/lib/inferra-runtime";
import type { Theme } from "@/lib/theme";
import { cn } from "@/lib/utils";
import { InferraRuntimeRail } from "@/components/inferra/runtime-console";
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
  inferraRuntime,
}: {
  navItems: NavItem[];
  children: ReactNode;
  mode: Mode;
  onModeChange: (mode: Mode) => void;
  modeStatus?: string;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  inferraRuntime?: InferraRuntimeSnapshot;
}) {
  const [open, setOpen] = useState(false);
  const location = useLocation();
  const isGraphPage = location.pathname === "/graph";

  const investigate = navItems.filter((item) => ["/", "/incidents", "/systems", "/graph", "/evidence"].includes(item.to));
  const tools = navItems.filter((item) => ["/ai", "/workspace", "/learning"].includes(item.to));
  const runtimeNav = navItems.filter((item) => ["/control", "/settings"].includes(item.to));

  const groups = [
    { title: "Investigate", items: investigate },
    { title: "Tools", items: tools },
    { title: "Runtime", items: runtimeNav },
  ].filter((group) => group.items.length > 0);

  return (
    <div className="min-h-screen bg-background lg:grid lg:h-screen lg:grid-cols-[15.5rem_minmax(0,1fr)]">
      <div className="flex items-center justify-between border-b border-border bg-sidebar px-4 py-3 lg:hidden">
        <div>
          <p className="font-data text-xs font-semibold tracking-[0.2em] text-sidebar-foreground">INFERRA</p>
          <p className="text-xs text-sidebar-muted">local runtime console</p>
        </div>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            type="button"
            aria-label={theme === "light" ? "Switch to dark theme" : "Switch to light theme"}
            onClick={() => onThemeChange(theme === "light" ? "dark" : "light")}
          >
            {theme === "light" ? <Moon className="size-4" /> : <Sun className="size-4" />}
          </Button>
          <Button variant="ghost" size="icon" type="button" aria-label="Toggle navigation" onClick={() => setOpen((v) => !v)}>
            {open ? <X className="size-4" /> : <Menu className="size-4" />}
          </Button>
        </div>
      </div>

      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-40 flex w-[15.5rem] flex-col border-r border-border bg-sidebar transition-transform duration-150 lg:static lg:h-screen lg:translate-x-0",
          open ? "translate-x-0" : "-translate-x-full lg:translate-x-0",
        )}
      >
        <div className="border-b border-border px-4 py-4">
          <div className="flex items-center gap-2.5">
            <div className="flex size-8 items-center justify-center rounded-sm border border-border bg-panel-inset font-data text-sm font-bold text-accent">
              I
            </div>
            <div>
              <p className="font-data text-[11px] font-semibold tracking-[0.2em] text-accent">INFERRA</p>
              <h1 className="text-base font-semibold leading-tight text-sidebar-foreground">Console</h1>
            </div>
          </div>
          <p className="mt-2 text-xs leading-5 text-sidebar-muted">Read-only runtime investigation.</p>
        </div>

        <nav className="min-h-0 flex-1 overflow-y-auto px-2 py-3">
          {groups.map((group) => (
            <div key={group.title} className="mb-4">
              <p className="px-2 pb-1 text-[10px] font-semibold uppercase tracking-[0.18em] text-sidebar-muted">{group.title}</p>
              <div className="space-y-0.5">
                {group.items.map((item) => {
                  const Icon = item.icon;
                  return (
                    <NavLink
                      key={item.to}
                      end={item.to === "/"}
                      onClick={() => setOpen(false)}
                      to={item.to}
                      className={({ isActive }) =>
                        cn(
                          "flex items-center gap-2.5 rounded-sm px-2.5 py-2 text-sm font-medium transition-colors",
                          isActive
                            ? "nav-rail-active bg-panel-inset text-sidebar-foreground"
                            : "text-sidebar-muted hover:bg-panel-inset/70 hover:text-sidebar-foreground",
                        )
                      }
                    >
                      <Icon className="size-4 shrink-0 opacity-80" />
                      <span>{item.label}</span>
                    </NavLink>
                  );
                })}
              </div>
            </div>
          ))}
        </nav>

        <div className="mt-auto space-y-3 border-t border-border p-3">
          {inferraRuntime ? <InferraRuntimeRail runtime={inferraRuntime} /> : null}
          <div>
            <p className="mb-1.5 px-1 text-[10px] font-semibold uppercase tracking-[0.18em] text-sidebar-muted">Experience</p>
            <div className="grid grid-cols-3 gap-1">
              {MODES.map((value) => (
                <Button
                  key={value}
                  variant={value === mode ? "default" : "ghost"}
                  size="sm"
                  type="button"
                  className="h-7 px-1 text-[11px]"
                  aria-pressed={value === mode}
                  onClick={() => onModeChange(value)}
                >
                  {value === "operator" ? "Op" : value === "expert" ? "Exp" : "Dev"}
                </Button>
              ))}
            </div>
            {modeStatus ? <p className="mt-2 px-1 text-[11px] leading-4 text-sidebar-muted">{modeStatus}</p> : null}
          </div>
          <div className="grid grid-cols-2 gap-1">
            <Button
              variant={theme === "light" ? "default" : "ghost"}
              size="sm"
              type="button"
              className="h-7"
              aria-pressed={theme === "light"}
              onClick={() => onThemeChange("light")}
            >
              <Sun className="size-3.5" />
              Light
            </Button>
            <Button
              variant={theme === "dark" ? "default" : "ghost"}
              size="sm"
              type="button"
              className="h-7"
              aria-pressed={theme === "dark"}
              onClick={() => onThemeChange("dark")}
            >
              <Moon className="size-3.5" />
              Dark
            </Button>
          </div>
        </div>
      </aside>

      {open ? (
        <button type="button" aria-label="Close navigation overlay" className="fixed inset-0 z-30 bg-black/50 lg:hidden" onClick={() => setOpen(false)} />
      ) : null}

      <main className="min-w-0 lg:h-screen lg:overflow-hidden">
        <div className={cn("h-full", isGraphPage ? "overflow-hidden" : "overflow-y-auto px-4 py-5 md:px-8 md:py-6")}>
          <div className={cn(isGraphPage ? "h-full" : "mx-auto max-w-[1480px] pb-8")}>{children}</div>
        </div>
      </main>
    </div>
  );
}
