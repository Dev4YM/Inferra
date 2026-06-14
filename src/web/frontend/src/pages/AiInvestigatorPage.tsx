import { Activity, Bot, RotateCcw, Send, Square } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { toast } from "sonner";

import type { AiDoctorResponse, AiGeneration, AiGenerationsResponse, IncidentRow, InvestigationResponse, ServiceRow } from "@/api";
import { errorMessage, fetchJson, postInvestigateStream, postJson } from "@/api";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { PageHeader } from "@/components/layout/page-header";
import { ErrorState, LoadingState } from "@/components/feedback/states";
import { InvestigationView } from "@/components/investigation/investigation-view";
import type { Mode } from "@/lib/experience";
import { isAdvancedMode } from "@/lib/experience";
import { formatDisplayValue } from "@/lib/format";
import { useApiMutation, useApiQuery } from "@/lib/query";

export function AiInvestigatorPage({ mode }: { mode: Mode }) {
  const [searchParams] = useSearchParams();
  const [question, setQuestion] = useState("What should I inspect first?");
  const [scope, setScope] = useState(() => {
    const incident = searchParams.get("incident");
    return incident ? `incident:${incident}` : "overview";
  });
  const [monitorSeconds, setMonitorSeconds] = useState(5);
  const [result, setResult] = useState<InvestigationResponse | null>(null);
  const [streamTranscript, setStreamTranscript] = useState("");
  const [streamBusy, setStreamBusy] = useState(false);
  const streamAbortRef = useRef<AbortController | null>(null);
  const lastInvestigationMethodRef = useRef<"ask" | "stream" | "report">("ask");
  const doctor = useApiQuery<AiDoctorResponse>("/api/ai/doctor");
  const incidents = useApiQuery<{ incidents: IncidentRow[] }>("/api/incidents");
  const services = useApiQuery<{ services: ServiceRow[] }>("/api/services");
  const savedGenerations = useApiQuery<AiGenerationsResponse>(
    `/api/ai/generations?scope=${encodeURIComponent(scope)}&limit=8`,
    { deps: [scope], staleTime: 5_000 },
  );
  const askMutation = useApiMutation(
    async (payload: { question: string; scope: string; mode: Mode; monitor_seconds: number }) =>
      postJson<InvestigationResponse>("/api/ai/ask", payload),
  );
  const reportMutation = useApiMutation(
    async ({ incidentId, monitor_seconds }: { incidentId: string; monitor_seconds: number }) =>
      fetchJson<InvestigationResponse>(`/api/ai/report/${encodeURIComponent(incidentId)}?mode=${mode}&monitor_seconds=${monitor_seconds}`),
  );

  const ask = useCallback(async () => {
    lastInvestigationMethodRef.current = "ask";
    try {
      setStreamTranscript("");
      const next = await askMutation.run({ question, scope, mode, monitor_seconds: monitorSeconds });
      setResult(next);
      void savedGenerations.reload({ silent: true });
      if (next.warnings?.length) {
        toast.warning("The AI response needed recovery before it could be shown.", { description: next.warnings.join(" ") });
      } else if (!next.used_ai) {
        toast.message("Deterministic fallback used.", { description: next.fallback_reason || "AI was unavailable." });
      }
    } catch (error) {
      toast.error("Investigation request failed", { description: errorMessage(error) });
    }
  }, [askMutation, mode, monitorSeconds, question, savedGenerations, scope]);

  const askStream = useCallback(async () => {
    lastInvestigationMethodRef.current = "stream";
    streamAbortRef.current?.abort();
    const controller = new AbortController();
    streamAbortRef.current = controller;
    setStreamTranscript("");
    setStreamBusy(true);
    try {
      const next = await postInvestigateStream(
        { question, scope, mode, monitor_seconds: monitorSeconds },
        {
          onDelta: (text) => {
            setStreamTranscript((prev) => prev + text);
          },
        },
        controller.signal,
      );
      setResult(next);
      void savedGenerations.reload({ silent: true });
      if (next.warnings?.length) {
        toast.warning("The AI response needed recovery before it could be shown.", { description: next.warnings.join(" ") });
      } else if (!next.used_ai) {
        toast.message("Deterministic fallback used.", { description: next.fallback_reason || "AI was unavailable." });
      } else {
        toast.success("Streamed investigation complete");
      }
    } catch (error) {
      if (error instanceof Error && error.name === "AbortError") {
        toast.message("Stream cancelled");
      } else {
        toast.error("Stream investigation failed", { description: errorMessage(error) });
      }
    } finally {
      setStreamBusy(false);
      streamAbortRef.current = null;
    }
  }, [mode, monitorSeconds, question, savedGenerations, scope]);

  const cancelStream = useCallback(() => {
    streamAbortRef.current?.abort();
  }, []);

  const runReport = useCallback(async () => {
    lastInvestigationMethodRef.current = "report";
    const incidentId = scope.startsWith("incident:") ? scope.slice("incident:".length) : null;
    if (!incidentId) return;
    try {
      const report = await reportMutation.run({ incidentId, monitor_seconds: monitorSeconds });
      setResult(report);
      void savedGenerations.reload({ silent: true });
      toast.success("Incident report generated");
    } catch (error) {
      toast.error("Report request failed", { description: errorMessage(error) });
    }
  }, [monitorSeconds, reportMutation, savedGenerations, scope]);

  const refreshInvestigation = useCallback(() => {
    const method = lastInvestigationMethodRef.current;
    if (method === "stream") {
      void askStream();
      return;
    }
    if (method === "report" && scope.startsWith("incident:")) {
      void runReport();
      return;
    }
    void ask();
  }, [ask, askStream, runReport, scope]);

  useEffect(() => {
    const incident = searchParams.get("incident");
    if (incident) {
      setScope(`incident:${incident}`);
      setQuestion(`What caused incident ${incident} and what should I check first?`);
    }
  }, [searchParams]);

  useEffect(() => {
    setResult(null);
    setStreamTranscript("");
  }, [scope]);

  useEffect(() => {
    if (result) return;
    const saved = savedGenerations.data?.generations?.[0];
    if (saved?.response) {
      setResult(hydrateSavedGeneration(saved));
    }
  }, [result, savedGenerations.data]);

  return (
    <div className="space-y-6">
      <PageHeader title="AI Investigator" subtitle="Read-only investigation with cited evidence and retry-aware fallback handling." mode={mode} />

      {doctor.isLoading && !doctor.data ? <LoadingState title="Checking AI provider" /> : null}
      {doctor.errorMessage && !doctor.data ? <ErrorState description={doctor.errorMessage} onRetry={() => void doctor.reload()} /> : null}

      {doctor.data ? (
        <Alert variant={doctor.data.warnings?.length ? "warning" : "info"}>
          <Bot className="size-4" />
          <div className="space-y-2">
            <AlertTitle>Provider health</AlertTitle>
            <AlertDescription>
              {formatDisplayValue(doctor.data.provider)} · {doctor.data.base_url} · status model {doctor.data.model}
              {doctor.data.investigate_model && doctor.data.investigate_model !== doctor.data.model
                ? ` · investigate model ${doctor.data.investigate_model}`
                : ""}{" "}
              · {doctor.data.allow_remote ? "remote allowed" : "local only"}
            </AlertDescription>
            <div className="flex flex-wrap gap-2">
              <Badge variant={doctor.data.available ? "success" : "warning"}>{doctor.data.available ? "available" : "degraded"}</Badge>
              <Badge variant={doctor.data.redact_raw_logs ? "success" : "destructive"}>
                {doctor.data.redact_raw_logs ? "redaction on" : "redaction off"}
              </Badge>
            </div>
            {doctor.data.warnings?.length ? (
              <div className="space-y-1 text-sm">
                {doctor.data.warnings.map((warning, index) => (
                  <p key={index}>• {warning}</p>
                ))}
              </div>
            ) : null}
          </div>
        </Alert>
      ) : null}

      <Card>
        <CardHeader>
          <CardTitle>Ask the investigator</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <Textarea
            aria-label="AI investigator question"
            value={question}
            onChange={(event) => setQuestion(event.target.value)}
            placeholder="Ask about the runtime, an incident, or a service…"
          />
          <div className="flex flex-wrap items-center gap-3">
            <label className="flex items-center gap-2 text-sm text-muted-foreground">
              <span className="whitespace-nowrap">Runtime monitor (s)</span>
              <Input
                aria-label="Investigation monitor seconds"
                className="h-11 w-20"
                type="number"
                min={0}
                max={180}
                value={monitorSeconds}
                onChange={(e) => setMonitorSeconds(Math.min(180, Math.max(0, Number(e.target.value) || 0)))}
              />
            </label>
            <select
              aria-label="AI investigation scope"
              className="h-11 min-w-[220px] rounded-xl border border-input bg-secondary/50 px-3 text-sm"
              value={scope}
              onChange={(event) => setScope(event.target.value)}
            >
              <option value="overview">Overview</option>
              <option value="latest">Latest incident</option>
              {(incidents.data?.incidents ?? []).map((incident) => (
                <option key={incident.incident_id} value={`incident:${incident.incident_id}`}>
                  Incident · {incident.incident_id}
                </option>
              ))}
              {(services.data?.services ?? []).slice(0, 20).map((service) => (
                <option key={service.service_id} value={`service:${service.service_id}`}>
                  Service · {service.service_id}
                </option>
              ))}
            </select>
            <Button onClick={() => void ask()} disabled={askMutation.isPending || streamBusy || !question.trim()}>
              {askMutation.isPending ? <RotateCcw className="size-4 animate-spin" /> : <Send className="size-4" />}
              Ask
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={() => void askStream()}
              disabled={streamBusy || askMutation.isPending || !question.trim()}
            >
              {streamBusy ? <RotateCcw className="size-4 animate-spin" /> : <Activity className="size-4" />}
              Ask (stream)
            </Button>
            {streamBusy ? (
              <Button type="button" variant="ghost" size="sm" onClick={() => cancelStream()}>
                <Square className="size-4" />
                Cancel stream
              </Button>
            ) : null}
            {scope.startsWith("incident:") ? (
              <Button variant="outline" onClick={() => void runReport()} disabled={reportMutation.isPending || streamBusy}>
                {reportMutation.isPending ? <RotateCcw className="size-4 animate-spin" /> : <Bot className="size-4" />}
                Incident report
              </Button>
            ) : null}
            {askMutation.errorMessage ? <p className="text-sm text-destructive">{askMutation.errorMessage}</p> : null}
          </div>
        </CardContent>
      </Card>

      {streamTranscript || streamBusy ? (
        <Card>
          <CardHeader>
            <CardTitle>Live stream (model tokens)</CardTitle>
          </CardHeader>
          <CardContent>
            <pre className="max-h-56 overflow-auto rounded-xl border border-border bg-muted/40 p-3 text-xs leading-relaxed text-muted-foreground">
              {streamTranscript || (streamBusy ? "… waiting for tokens" : "")}
            </pre>
            <p className="mt-2 text-xs text-muted-foreground">
              Raw deltas from the provider; the structured answer appears below when the stream completes.
            </p>
          </CardContent>
        </Card>
      ) : null}

      {result ? (
        <InvestigationView result={result} showRaw={isAdvancedMode(mode)} onRefresh={() => void refreshInvestigation()} />
      ) : null}

      {savedGenerations.data?.generations?.length ? (
        <Card>
          <CardHeader>
            <CardTitle>Saved AI runs</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            {savedGenerations.data.generations.slice(0, 5).map((generation) => (
              <button
                key={generation.generation_id}
                type="button"
                className="flex w-full items-center justify-between gap-3 rounded-xl border border-border bg-panel-inset p-3 text-left text-sm transition hover:bg-secondary/40"
                onClick={() => setResult(hydrateSavedGeneration(generation))}
              >
                <span className="min-w-0">
                  <span className="block truncate font-medium">{generation.question || generation.focus}</span>
                  <span className="block truncate text-xs text-muted-foreground">{generation.scope_key}</span>
                </span>
                <Badge variant={generation.used_ai ? "success" : "outline"}>{generation.created_at}</Badge>
              </button>
            ))}
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}

function hydrateSavedGeneration(generation: AiGeneration): InvestigationResponse {
  return {
    ...generation.response,
    cached: true,
    ai_generation: {
      generation_id: generation.generation_id,
      scope_key: generation.scope_key,
      focus: generation.focus,
      mode: generation.mode,
      question: generation.question,
      bundle_hash: generation.bundle_hash,
      used_ai: generation.used_ai,
      created_at: generation.created_at,
    },
  };
}
