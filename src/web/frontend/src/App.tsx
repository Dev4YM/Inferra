import { useCallback, useEffect, useState } from "react";
import {
  Activity,
  AlertTriangle,
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
import { Link, Route, Routes } from "react-router-dom";
import { Toaster, toast } from "sonner";

import { ApiError, type ConfigResponse, type InferraConfigPayload, fetchJson, putJson } from "@/api";
import { AppShell, type NavItem } from "@/components/layout/app-shell";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { configMode, type Mode, useMode } from "@/lib/experience";
import { formatModeLabel } from "@/lib/format";
import { useApiQuery } from "@/lib/query";
import { useTheme } from "@/lib/theme";
import { AiInvestigatorPage } from "@/pages/AiInvestigatorPage";
import { ControlPage } from "@/pages/ControlPage";
import { EvidencePage } from "@/pages/EvidencePage";
import { GraphPage } from "@/pages/GraphPage";
import { IncidentDetailPage, IncidentsPage } from "@/pages/IncidentsPage";
import { LearningReviewPage } from "@/pages/LearningReviewPage";
import { OverviewPage } from "@/pages/OverviewPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { TracePage } from "@/pages/TracePage";
import { ServiceDetailPage, SystemsPage } from "@/pages/SystemsPage";
import { WorkspaceAppPage, WorkspacePage } from "@/pages/WorkspacePage";

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

export default function App() {
  const [mode, setMode] = useMode();
  const [theme, setTheme] = useTheme();
  const [modeStatus, setModeStatus] = useState("");
  const configState = useApiQuery<ConfigResponse>("/api/config");
  const authError =
    configState.error instanceof ApiError && [401, 403, 503].includes(configState.error.status)
      ? configState.errorMessage
      : null;

  useEffect(() => {
    const persisted = configMode(configState.data?.config);
    if (persisted && persisted !== mode) {
      setMode(persisted);
    }
  }, [configState.data, mode, setMode]);

  const persistMode = useCallback(
    async (nextMode: Mode) => {
      setMode(nextMode);
      setModeStatus("Saving mode to config…");
      try {
        const current = configState.data?.config ?? (await fetchJson<ConfigResponse>("/api/config")).config;
        const nextConfig: InferraConfigPayload = {
          ...current,
          experience: {
            ...(current.experience ?? {}),
            mode: nextMode,
            show_raw_evidence_by_default: nextMode !== "operator",
          },
        };
        const saved = await putJson<ConfigResponse>("/api/config", { config: nextConfig });
        setModeStatus(saved.applied ? "Mode saved to config." : "Mode updated locally.");
        toast.success("Experience mode updated", { description: `${formatModeLabel(nextMode)} mode is now active.` });
        void configState.reload({ silent: true });
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        setModeStatus(`Mode is local only: ${message}`);
        toast.warning("Mode could not be persisted to config", { description: message });
      }
    },
    [configState, setMode],
  );

  return (
    <>
      <AppShell
        navItems={NAV_ITEMS}
        mode={mode}
        onModeChange={(next) => void persistMode(next)}
        modeStatus={modeStatus}
        theme={theme}
        onThemeChange={setTheme}
      >
        {configState.isRefreshing ? (
          <div className="mb-4 flex items-center gap-2 rounded-2xl border border-border/70 bg-card/60 px-4 py-2 text-sm text-muted-foreground">
            <Activity className="size-4 animate-pulse text-primary" />
            Syncing config and refreshing control-plane state…
          </div>
        ) : null}
        {authError ? (
          <Alert variant="warning" className="mb-4">
            <AlertTriangle className="size-4" />
            <div className="min-w-0">
              <AlertTitle>API access needs attention</AlertTitle>
              <AlertDescription>{authError}</AlertDescription>
              <div className="mt-3">
                <Button asChild variant="outline" size="sm">
                  <Link to="/settings">Open token settings</Link>
                </Button>
              </div>
            </div>
          </Alert>
        ) : null}
        <Routes>
          <Route path="/" element={<OverviewPage mode={mode} />} />
          <Route path="/incidents" element={<IncidentsPage mode={mode} />} />
          <Route path="/incidents/:incidentId" element={<IncidentDetailPage mode={mode} />} />
          <Route path="/traces/:traceId" element={<TracePage mode={mode} />} />
          <Route path="/learning" element={<LearningReviewPage mode={mode} />} />
          <Route path="/systems" element={<SystemsPage mode={mode} />} />
          <Route path="/systems/:serviceId" element={<ServiceDetailPage mode={mode} />} />
          <Route path="/graph" element={<GraphPage mode={mode} />} />
          <Route path="/evidence" element={<EvidencePage mode={mode} />} />
          <Route path="/ai" element={<AiInvestigatorPage mode={mode} />} />
          <Route path="/workspace" element={<WorkspacePage mode={mode} />} />
          <Route path="/workspace/apps" element={<WorkspaceAppPage mode={mode} />} />
          <Route path="/control" element={<ControlPage mode={mode} />} />
          <Route path="/settings" element={<SettingsPage mode={mode} theme={theme} onThemeChange={setTheme} />} />
          <Route path="*" element={<NotFoundPage mode={mode} />} />
        </Routes>
      </AppShell>
      <Toaster theme={theme} richColors position="top-right" />
    </>
  );
}

function NotFoundPage({ mode }: { mode: Mode }) {
  return (
    <div className="space-y-6">
      <div className="rounded-3xl border border-dashed border-border bg-card/70 p-8 text-center shadow-sm">
        <p className="text-xs font-semibold uppercase tracking-[0.25em] text-muted-foreground">{formatModeLabel(mode)} mode</p>
        <h1 className="mt-3 text-3xl font-semibold tracking-tight">Page not found</h1>
        <p className="mx-auto mt-3 max-w-xl text-sm leading-6 text-muted-foreground">
          This route is not part of the Inferra console. Return to the overview or use the sidebar navigation.
        </p>
        <Button asChild className="mt-6">
          <Link to="/">Back to overview</Link>
        </Button>
      </div>
    </div>
  );
}
