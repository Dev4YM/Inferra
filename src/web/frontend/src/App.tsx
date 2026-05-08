import { useCallback, useEffect, useState } from "react";
import {
  Activity,
  BrainCircuit,
  FolderKanban,
  Home,
  Logs,
  Settings,
  Shield,
  Sparkles,
  Siren,
  Workflow,
} from "lucide-react";
import { Route, Routes } from "react-router-dom";
import { Toaster, toast } from "sonner";

import { type ConfigResponse, type InferraConfigPayload, fetchJson, putJson } from "@/api";
import { AppShell, type NavItem } from "@/components/layout/app-shell";
import { configMode, type Mode, useMode } from "@/lib/experience";
import { useApiQuery } from "@/lib/query";
import { useTheme } from "@/lib/theme";
import { AiInvestigatorPage } from "@/pages/AiInvestigatorPage";
import { ControlPage } from "@/pages/ControlPage";
import { EvidencePage } from "@/pages/EvidencePage";
import { IncidentDetailPage, IncidentsPage } from "@/pages/IncidentsPage";
import { LearningReviewPage } from "@/pages/LearningReviewPage";
import { OverviewPage } from "@/pages/OverviewPage";
import { SettingsPage } from "@/pages/SettingsPage";
import { ServiceDetailPage, SystemsPage } from "@/pages/SystemsPage";
import { WorkspacePage } from "@/pages/WorkspacePage";

const NAV_ITEMS: NavItem[] = [
  { to: "/", label: "Overview", icon: Home },
  { to: "/incidents", label: "Incidents", icon: Siren },
  { to: "/learning", label: "Learning Review", icon: Sparkles },
  { to: "/systems", label: "Systems", icon: Workflow },
  { to: "/evidence", label: "Evidence", icon: Logs },
  { to: "/ai", label: "AI Investigator", icon: BrainCircuit },
  { to: "/workspace", label: "Workspace", icon: FolderKanban },
  { to: "/control", label: "Control", icon: Shield },
  { to: "/settings", label: "Settings", icon: Settings },
];

export default function App() {
  const [mode, setMode] = useMode();
  const [theme, setTheme] = useTheme();
  const [modeStatus, setModeStatus] = useState("");
  const configState = useApiQuery<ConfigResponse>("/api/config");

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
        toast.success("Experience mode updated", { description: `${nextMode} mode is now active.` });
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
        <Routes>
          <Route path="/" element={<OverviewPage mode={mode} />} />
          <Route path="/incidents" element={<IncidentsPage mode={mode} />} />
          <Route path="/incidents/:incidentId" element={<IncidentDetailPage mode={mode} />} />
          <Route path="/learning" element={<LearningReviewPage mode={mode} />} />
          <Route path="/systems" element={<SystemsPage mode={mode} />} />
          <Route path="/systems/:serviceId" element={<ServiceDetailPage mode={mode} />} />
          <Route path="/evidence" element={<EvidencePage mode={mode} />} />
          <Route path="/ai" element={<AiInvestigatorPage mode={mode} />} />
          <Route path="/workspace" element={<WorkspacePage mode={mode} />} />
          <Route path="/control" element={<ControlPage mode={mode} />} />
          <Route path="/settings" element={<SettingsPage mode={mode} theme={theme} onThemeChange={setTheme} />} />
        </Routes>
      </AppShell>
      <Toaster theme={theme} richColors position="top-right" />
    </>
  );
}

