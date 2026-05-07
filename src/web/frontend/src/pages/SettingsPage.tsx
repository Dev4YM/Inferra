import { Moon, Save, Sun } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import type { InferraConfigPayload } from "@/api";
import { putJson } from "@/api";
import { JsonInspector } from "@/components/ui/json-inspector";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Textarea } from "@/components/ui/textarea";
import { PageHeader } from "@/components/layout/page-header";
import { ErrorState, LoadingState } from "@/components/feedback/states";
import type { Mode } from "@/lib/experience";
import type { Theme } from "@/lib/theme";
import { useApiMutation, useApiQuery } from "@/lib/query";

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

  return (
    <div className="space-y-6">
      <PageHeader title="Settings" subtitle="Inspect and edit Inferra's local configuration." mode={mode} />

      <div className="grid gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
        <Card>
          <CardHeader>
            <CardTitle>Appearance & defaults</CardTitle>
          </CardHeader>
          <CardContent className="space-y-4 text-sm">
            <div className="rounded-2xl border border-border/60 bg-background/30 p-4">
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

