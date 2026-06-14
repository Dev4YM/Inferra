import { Bot, Database, Moon, RadioTower, Save, Settings2, Shield, Sun } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import type { InferraConfigPayload } from "@/api";
import { errorMessage, getInferraAuthToken, putJson, setInferraAuthToken } from "@/api";
import { JsonInspector } from "@/components/ui/json-inspector";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { PageHeader } from "@/components/layout/page-header";
import { ErrorState, LoadingState } from "@/components/feedback/states";
import type { Mode } from "@/lib/experience";
import type { Theme } from "@/lib/theme";
import { useApiMutation, useApiQuery } from "@/lib/query";
import { RuntimeStatusCard } from "@/components/inferra/health";
import { formatDisplayValue } from "@/lib/format";

export function SettingsPage({
  mode,
  theme,
  onThemeChange,
}: {
  mode: Mode;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
}) {
  const settings = useApiQuery<{ config: Record<string, unknown> }>("/api/config");
  const saveMutation = useApiMutation(async (payload: Record<string, unknown>) =>
    putJson("/api/config", { config: payload }),
  );
  const [text, setText] = useState("");
  const [apiToken, setApiToken] = useState(() => getInferraAuthToken());

  useEffect(() => {
    if (settings.data) {
      setText(JSON.stringify(settings.data.config, null, 2));
    }
  }, [settings.data]);

  if (settings.isLoading && !settings.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Settings" subtitle="Inferra configuration and persisted experience modes." mode={mode} />
        <LoadingState title="Loading configuration" />
      </div>
    );
  }

  if (settings.errorMessage && !settings.data) {
    return (
      <div className="space-y-6">
        <PageHeader title="Settings" subtitle="Inferra configuration and persisted experience modes." mode={mode} />
        <ErrorState description={settings.errorMessage} onRetry={() => void settings.reload()} />
      </div>
    );
  }

  const save = async () => {
    try {
      const parsed = JSON.parse(text) as InferraConfigPayload;
      await saveMutation.run(parsed);
      toast.success("Configuration saved.");
      void settings.reload({ silent: true });
    } catch (error) {
      toast.error("Could not save configuration", { description: error instanceof Error ? error.message : String(error) });
    }
  };
  const testApiToken = async () => {
    try {
      await settings.reload();
      toast.success("API token accepted.");
    } catch (error) {
      toast.error("API token test failed", { description: errorMessage(error) });
    }
  };
  const config = settings.data?.config ?? {};
  const experience = asRecord(config.experience);
  const ai = asRecord(config.ai);
  const retention = asRecord(config.retention);
  const collectors = asRecord(config.collectors);
  const scanner = asRecord(config.scanner);
  const workspaceScanInterval = Number(scanner?.workspace_interval_seconds ?? 120);

  const updateWorkspaceScanInterval = (value: number) => {
    const nextValue = Math.min(3600, Math.max(15, value || 120));
    try {
      const parsed = JSON.parse(text || "{}") as Record<string, unknown>;
      const nextScanner = {
        ...(asRecord(parsed.scanner) ?? {}),
        workspace_interval_seconds: nextValue,
      };
      setText(JSON.stringify({ ...parsed, scanner: nextScanner }, null, 2));
    } catch {
      toast.error("Config JSON must be valid before editing scanner settings.");
    }
  };

  return (
    <div className="space-y-6">
      <PageHeader title="Settings" subtitle="Inspect and edit Inferra's local configuration." mode={mode} />

      <div className="dashboard-grid">
        <RuntimeStatusCard icon={RadioTower} label="Collectors" value={collectors ? "Configured" : "Default"} tone="info" detail="Runtime, logs, containers, and process inputs." />
        <RuntimeStatusCard icon={Bot} label="LLM provider" value={formatDisplayValue(String(ai?.provider ?? "local"))} tone="info" detail={formatDisplayValue(String(ai?.model ?? ai?.investigate_model ?? "model resolved by backend"))} />
        <RuntimeStatusCard icon={Database} label="Retention" value={formatDisplayValue(String(retention?.days ?? retention?.max_days ?? "default"))} tone="secondary" detail="Local-first storage retention policy." />
        <RuntimeStatusCard icon={Shield} label="Developer mode" value={mode === "developer" ? "Enabled" : "Hidden"} tone={mode === "developer" ? "warning" : "success"} detail={String(experience?.show_raw_evidence_by_default ?? false) === "true" ? "Raw evidence shown by default." : "Progressive disclosure enabled."} />
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
        <Card>
          <CardHeader>
            <CardTitle>Appearance & defaults</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <div className="rounded-md border border-border bg-panel-inset p-4">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <p className="font-medium">Theme</p>
                  <p className="text-muted-foreground">Light is the default, with a persistent dark option for operators who prefer it.</p>
                </div>
                <div className="flex gap-2">
                  <Button
                    type="button"
                    size="sm"
                    variant={theme === "light" ? "default" : "outline"}
                    aria-pressed={theme === "light"}
                    onClick={() => onThemeChange("light")}
                  >
                    <Sun className="size-4" />
                    Light
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    variant={theme === "dark" ? "default" : "outline"}
                    aria-pressed={theme === "dark"}
                    onClick={() => onThemeChange("dark")}
                  >
                    <Moon className="size-4" />
                    Dark
                  </Button>
                </div>
              </div>
            </div>
            <div className="grid gap-3 md:grid-cols-2">
              <SettingTile icon={Settings2} label="Experience mode" value={formatDisplayValue(String(experience?.mode ?? mode))} />
              <SettingTile icon={Shield} label="Safe actions" value={formatDisplayValue(Boolean(experience?.suggest_safe_actions ?? true))} />
              <SettingTile icon={Bot} label="AI role" value={formatDisplayValue(String(experience?.ai_role ?? "investigator"))} />
              <SettingTile icon={Database} label="Storage policy" value={retention ? "Custom" : "Default"} />
            </div>
            <div className="rounded-md border border-border bg-panel-inset p-4">
              <div className="space-y-2">
                <p className="font-medium">API bearer token</p>
                <p className="text-sm text-muted-foreground">
                  When the server uses server.auth_token_env, paste the matching token here. Stored only in this
                  browser session.
                </p>
                <Input
                  aria-label="API bearer token"
                  type="password"
                  autoComplete="off"
                  placeholder="Bearer token (optional)"
                  value={apiToken}
                  onChange={(event) => {
                    const value = event.target.value;
                    setApiToken(value);
                    setInferraAuthToken(value);
                  }}
                />
                <Button type="button" variant="outline" size="sm" onClick={() => void testApiToken()}>
                  Test API token
                </Button>
              </div>
            </div>
            <div className="rounded-md border border-border bg-panel-inset p-4">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div>
                  <p className="font-medium">Workspace scan interval</p>
                  <p className="text-sm text-muted-foreground">
                    Used by the scanner cache. Limits are 15 seconds to 3600 seconds.
                  </p>
                </div>
                <Input
                  aria-label="Workspace scan interval seconds"
                  className="w-32"
                  min={15}
                  max={3600}
                  type="number"
                  value={Number.isFinite(workspaceScanInterval) ? workspaceScanInterval : 120}
                  onChange={(event) => updateWorkspaceScanInterval(Number(event.target.value))}
                />
              </div>
            </div>
            {settings.data?.config ? <JsonInspector data={settings.data.config} title="Configuration preview" /> : null}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Configuration editor</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <p className="text-sm text-muted-foreground">
              The preview on the left is human-friendly. Use this editor when you need the full raw JSON payload for precise changes.
            </p>
            <Textarea
              aria-label="Raw Inferra configuration editor"
              className="min-h-[520px] font-mono text-xs leading-6"
              value={text}
              onChange={(event) => setText(event.target.value)}
            />
            <div className="flex flex-wrap items-center gap-3">
              <Button onClick={() => void save()} disabled={saveMutation.isPending}>
                <Save className="size-4" />
                Save config
              </Button>
              {saveMutation.errorMessage ? <p className="text-sm text-destructive">{saveMutation.errorMessage}</p> : null}
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function SettingTile({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Settings2;
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-md border border-border bg-panel-inset p-4">
      <div className="flex items-center gap-2">
        <Icon className="size-4 text-primary" />
        <p className="text-xs font-semibold uppercase tracking-[0.18em] text-muted-foreground">{label}</p>
      </div>
      <p className="mt-2 break-words text-sm font-medium">{value}</p>
    </div>
  );
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}
