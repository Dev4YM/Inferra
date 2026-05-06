# ADR 0006: Ollama Gemma Default Model Family

## Status

Accepted

## Context

Inferra ships an optional Ollama-compatible provider. Model tags change frequently upstream; defaults must point at tags that exist in the public Ollama library while allowing aliases and resolution against the live daemon (`/api/tags`).

## Decision

- The supported provider family for defaults is Gemma on Ollama. Verified tag metadata is recorded in `src/ai/registry.py` from the official library pages (`https://ollama.com/library/gemma3/tags`, `https://ollama.com/library/gemma4/tags`).
- **Verification snapshot:** Gemma 4 is published upstream (library lists sizes, context windows, and modalities per tag). The repository default model tag matches the shipped config (`[ai].model` in `src/config/defaults.toml`) and resolves through `resolve_gemma_model_alias` and runtime tag checks when needed.
- If a forward-looking alias (for example a shorthand like `gemma4:e4b`) does not exist on a given daemon, operators choose an installed tag via `inferra ai models` and `inferra ai pull`, or point `[ai].model` at an available digest-qualified name.

## Consequences

- Changing the global default requires updating defaults, registry entries, and documentation together.
- CLI `inferra ai models` merges the static registry with installed models without requiring outbound calls for offline installs once weights are cached.
