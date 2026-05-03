# Inferra Implementation Roadmap

This file is the living checklist for building Inferra in deep, complete slices while keeping the repository ready for GitHub.

## Current Slice: AI + CLI Foundation

- [x] Keep the flattened `src/` layout with no nested `inferra/` package.
- [x] Add an AI subsystem for provider logic, model registry, prompting, redaction, and explanation service.
- [x] Add Ollama local/remote HTTP support with optional bearer token from an environment variable.
- [x] Add Gemma 4 model registry from the official Ollama tag list.
- [x] Add first-run setup and AI control commands to the CLI.
- [x] Add web API endpoints for AI status, model registry, incident explanation, and incident chat.
- [x] Add tests for config, registry, redaction, Ollama provider behavior, CLI setup, and AI web endpoints.

## Next Slices

- [x] Collector depth: Windows Event Log, service/process state, performance counters, Linux journald/syslog, Kubernetes events.
  - [x] Portable process snapshot collector with CPU/memory thresholds and CLI one-shot ingest.
  - [x] Windows service collector and host performance snapshot CLI ingest.
  - [x] Windows Event Log bookmark persistence with CLI one-shot ingest.
  - [x] Windows Event Log channel metadata enrichment.
  - [x] Linux journald/syslog collectors.
  - [x] Kubernetes event/workload collectors.
- [x] Reasoning depth: anomaly scoring, causal graph traversal, contradiction handling, confidence calibration.
  - [x] Deterministic anomaly scoring for severity, resource tags, host metrics, and process metrics.
  - [x] Causal graph traversal for dependency proximity and root-cause event selection.
  - [x] Contradiction handling and invalidation rules for health checks, resource state, and timeline conflicts.
  - [x] Confidence calibration tests against scenario fixtures.
- [x] Web UI: AI settings panel, model health view, incident explanation/chat surface.
- [x] Packaging: Windows service/installer, Linux systemd unit, Docker image, Helm chart, macOS launch agent.
  - [x] Windows service helper scripts.
  - [x] Linux systemd unit.
  - [x] Dockerfile and Compose config.
  - [x] Helm chart baseline.
  - [x] macOS launch agent.
- [x] Documentation: install guide, operator guide, provider setup guide, architecture decision records.
  - [x] Install guide.
  - [x] AI provider setup guide.
  - [x] Collector command guide.
  - [x] Architecture decision records.
- [x] Service supervision and health.
  - [x] Long-running collector supervisor with retry/backoff.
  - [x] `run-collectors` CLI command.
  - [x] Collector health API and dashboard panel.
  - [x] Configurable service mode presets.
- [x] Operator controls.
  - [x] Start/stop supervised collectors from API and web UI.
  - [x] CLI collector status command.

## Operating Rules

- AI is guided only: it explains, summarizes, chats, and suggests checks.
- Deterministic evidence, scoring, and incident ranking remain authoritative.
- No model weight is pulled automatically without explicit CLI confirmation or `--yes --pull`.
- Remote AI means an Ollama-compatible HTTP server configured by base URL and optional token environment variable.
