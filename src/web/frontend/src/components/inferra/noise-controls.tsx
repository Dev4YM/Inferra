import { VolumeX } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";

import { errorMessage, putJson, type ConfigResponse } from "@/api";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";

const WINDOWS_UPDATER_EXCLUDES = [
  "edgeupdate",
  "edgeupdatem",
  "googleupdater*",
  "dropboxupdater*",
  "sppsvc",
  "gupdate",
  "gupdatem",
];

const WINDOWS_UPDATER_BLOCKLIST = [
  {
    pattern: "windows service *updater* transitioned",
    service_id: "",
    severity_max: "INFO",
    reason: "benign windows updater lifecycle",
  },
  {
    pattern: "windows service sppsvc transitioned stopped -> stopped",
    service_id: "",
    severity_max: "INFO",
    reason: "software protection idle state",
  },
];

export function WindowsUpdaterNoiseControl() {
  const [pending, setPending] = useState(false);

  const muteUpdaters = async () => {
    setPending(true);
    try {
      await putJson<ConfigResponse>("/api/config", {
        collectors: {
          windows_service: {
            exclude_names: WINDOWS_UPDATER_EXCLUDES,
          },
        },
        noise_filter: {
          blocklist: WINDOWS_UPDATER_BLOCKLIST,
        },
      });
      toast.success("Windows updater noise filters applied", {
        description: "Restart collectors from Control if windows_service was already running.",
      });
    } catch (error) {
      toast.error("Could not update noise filters", { description: errorMessage(error) });
    } finally {
      setPending(false);
    }
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <VolumeX className="size-4" />
          Signal quality
        </CardTitle>
        <CardDescription>
          Mute benign Windows updater and software-protection lifecycle events that flood incidents on desktop hosts.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <Alert>
          <div className="min-w-0">
            <AlertTitle>Windows updater services</AlertTitle>
            <AlertDescription>
              Adds collector excludes and noise blocklist entries for edgeupdate, Google/Dropbox updaters, and sppsvc.
            </AlertDescription>
          </div>
        </Alert>
        <Button variant="outline" size="sm" onClick={() => void muteUpdaters()} disabled={pending}>
          {pending ? "Applying…" : "Mute Windows updater noise"}
        </Button>
      </CardContent>
    </Card>
  );
}
