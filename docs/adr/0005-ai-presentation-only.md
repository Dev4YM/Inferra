# ADR 0005: AI Presentation-Only Layer

## Status

Accepted

## Context

Large language models can summarize evidence fluently but are not auditable sources of truth for incident ranking or scoring. Mixing LLM output back into deterministic pipelines would violate reproducibility and operator trust.

## Decision

- AI explains, summarizes, chats with operators, and suggests manual checks only.
- Correlation, hypothesis generation, scoring, ranking, and calibration remain rule-based. LLM output must never feed back into analysis, scoring, or ranking.
- When the provider is unavailable, explanations degrade to deterministic templates without changing incident ordering.

## Consequences

- API and CLI surfaces separate “reasoning” from “explanation”, keep prompts redacted for UI display, and persist explanation/trace artifacts without feeding them back into scoring.
- Provider configuration and investigation prompting live in the native Rust runtime; explanation output never mutates deterministic scoring or ranking.
