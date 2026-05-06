# AI Provider Setup

Inferra uses AI as a guided explanation layer only; it does not change deterministic scoring or ranking (see [ADR 0005](../adr/0005-ai-presentation-only.md)).

## Gemma model choice

The shipped default in `src/config/defaults.toml` is a Gemma 4 tag suitable for local workstations. Verified sizes, context windows, and modalities for Gemma 3 and Gemma 4 are recorded in `src/ai/registry.py`, sourced from the public Ollama library (`https://ollama.com/library/gemma3/tags`, `https://ollama.com/library/gemma4/tags`). As of the last registry refresh, Gemma 4 offers multiple quantized variants (for example `gemma4:e2b`, `gemma4:e4b`, `gemma4:26b`, `gemma4:31b`) with 128K or 256K context where listed.

Alias resolution maps shorthand tags to concrete pulls (`GEMMA4_DEFAULT_ALIASES` / `GEMMA3_DEFAULT_ALIASES`). At runtime, `choose_available_gemma_model` can pick an installed digest if your daemon lacks a given alias.

## Local Ollama

Prepare config and probe the daemon:

```powershell
inferra --config inferra.toml config set ai.enabled false
inferra --config inferra.toml config set ai.model gemma4:e4b
inferra --config inferra.toml ai models
inferra --config inferra.toml ai status
```

After `ai.enabled` is true and Ollama is reachable at `[ai].base_url`, pull and smoke-test:

```powershell
inferra --config inferra.toml config set ai.enabled true
inferra --config inferra.toml ai pull --help
inferra --config inferra.toml ai test --help
```

Run `inferra ai pull <tag>` once Ollama is running to download weights; streams progress to the terminal. Run `inferra ai test` for a short health prompt against the configured model.

`inferra setup --yes` enables AI and probes Ollama unless `--skip-connection-test` is set. No model downloads occur without an explicit `ai pull` or upstream `ollama pull`.

## Remote Ollama-compatible servers

```powershell
inferra --config inferra.toml config set ai.enabled true
inferra --config inferra.toml config set ai.base_url http://SERVER:11434
inferra --config inferra.toml config set ai.allow_remote true
inferra --config inferra.toml config set ai.model gemma4:e4b
inferra --config inferra.toml ai status --help
```

When the remote endpoint is reachable, run `inferra ai status` without `--help` to probe availability.

## Bearer token environment variable

When the upstream expects `Authorization: Bearer …`, store the secret outside config files:

```powershell
$env:OLLAMA_TOKEN = "..."
inferra --config inferra.toml config set ai.token_env OLLAMA_TOKEN
```

Inferra reads the variable name from `[ai].token_env` and resolves the value at request time.

## Streaming

`[ai].stream = true` (default) streams chat and generate calls where supported (`src/ai/ollama.py`), including aggregated explanation paths in the web UI. Disable streaming for debugging unstable proxies by setting `stream = false` in `inferra.toml`.

## Troubleshooting

- **Unreachable provider:** check `[ai].base_url`, TLS mismatch, and firewall paths; set `allow_remote` when not using loopback.
- **Auth failures:** confirm `token_env` names an exported variable in the service account environment (Windows Service / systemd / container).
- **Missing models:** run `inferra ai models` and install a listed tag; aliases fall back when the daemon exposes resolved names via `/api/tags`.
- **Strict environments:** keep `ai.enabled = false` and rely on template explanations; core pipelines stay offline.

See [Troubleshooting](troubleshooting.md) for Ollama-specific failure modes and collector issues.
