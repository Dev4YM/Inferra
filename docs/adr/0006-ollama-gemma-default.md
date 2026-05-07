# ADR 0006: Ollama Gemma Default Model Family

## Status

Accepted

## Context

Inferra ships an optional Ollama-compatible provider. Model tags change frequently upstream; defaults must point at tags that exist in the public Ollama library while allowing aliases and resolution against the live daemon (`/api/tags`).

## Decision

- The supported provider family for defaults is Gemma on Ollama. Verified tag expectations are documented against the shipped config (`[ai].model` in `src/config/defaults.toml`) and enforced by the native Rust provider status/doctor flow.
- **Verification snapshot:** Gemma 4 is published upstream (library lists sizes, context windows, and modalities per tag). The repository default model tag must continue to match the shipped config and the native runtime's probe logic.
- If a forward-looking alias (for example a shorthand like `gemma4:e4b`) does not exist on a given daemon, operators use `ollama pull <tag>` and then point `[ai].model` at an installed tag or digest-qualified name.

## Consequences

- Changing the global default requires updating defaults, native provider checks, and documentation together.
- Operator guidance should stay aligned with the native `inferra ai status` / `inferra ai doctor` flow rather than archived Python-era model registry commands.
