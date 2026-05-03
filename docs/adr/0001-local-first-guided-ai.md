# ADR 0001: Local-First Guided AI

## Status

Accepted

## Context

Inferra explains runtime failures using local evidence. The product must support sensitive environments, Windows/Windows Server first, and optional Linux/Kubernetes/macOS deployments.

## Decision

Inferra keeps deterministic collection, storage, correlation, hypothesis ranking, and confidence scoring authoritative. AI providers are optional guided explanation layers. The first provider is Ollama, with Gemma 4 as the default model family.

## Consequences

- Inferra can operate without network or cloud AI.
- AI output can explain, summarize, chat, and suggest manual checks.
- AI output must not mutate monitored systems or silently change deterministic scores.
- Provider failures must degrade to deterministic template explanations.
