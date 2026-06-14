import {
  BrainCircuit,
  FolderKanban,
  Home,
  Logs,
  Network,
  Settings,
  Shield,
  Sparkles,
  Siren,
  Workflow,
} from "lucide-react";
import { MemoryRouter } from "react-router-dom";

import { AppShell, type NavItem } from "@/components/layout/app-shell";
import type { InferraRuntimeSnapshot } from "@/lib/inferra-runtime";
import type { Mode } from "@/lib/experience";
import { OverviewPageContent } from "@/pages/OverviewPage";
import { showcaseCollectorsMock, showcaseOverviewMock } from "@/dev/overview-showcase-mock";

import "./readme-hero.css";

const NAV_ITEMS: NavItem[] = [
  { to: "/", label: "Overview", icon: Home },
  { to: "/incidents", label: "Incidents", icon: Siren },
  { to: "/systems", label: "Systems", icon: Workflow },
  { to: "/graph", label: "Graph", icon: Network },
  { to: "/evidence", label: "Evidence", icon: Logs },
  { to: "/ai", label: "AI Investigator", icon: BrainCircuit },
  { to: "/workspace", label: "Workspace", icon: FolderKanban },
  { to: "/learning", label: "Learning Review", icon: Sparkles },
  { to: "/control", label: "Control", icon: Shield },
  { to: "/settings", label: "Settings", icon: Settings },
];

const showcaseRuntime: InferraRuntimeSnapshot = {
  state: "online",
  health: {
    status: "ok",
    runtime: "rust",
    storage_writes_ok: true,
  },
  ready: null,
  error: null,
  errorMessage: null,
  isRefreshing: false,
  reload: async () => undefined,
};

const mode: Mode = "operator";

export function ReadmeHeroShowcase() {
  return (
    <div className="readme-hero-root" id="readme-hero-capture">
      <div className="readme-macos-wallpaper" aria-hidden />
      <div className="readme-hero-window-wrap">
        <div className="readme-mac-titlebar">
          <div className="readme-mac-lights" aria-hidden>
            <span className="readme-mac-light red" />
            <span className="readme-mac-light yellow" />
            <span className="readme-mac-light green" />
          </div>
          <p className="readme-mac-title">Inferra Console — Overview</p>
        </div>
        <div className="readme-hero-app">
          <MemoryRouter initialEntries={["/"]}>
            <AppShell
              navItems={NAV_ITEMS}
              mode={mode}
              onModeChange={() => undefined}
              theme="light"
              onThemeChange={() => undefined}
              inferraRuntime={showcaseRuntime}
            >
              <OverviewPageContent
                mode={mode}
                data={showcaseOverviewMock}
                collectorRows={showcaseCollectorsMock}
                runtimeState="online"
              />
            </AppShell>
          </MemoryRouter>
        </div>
      </div>
    </div>
  );
}
