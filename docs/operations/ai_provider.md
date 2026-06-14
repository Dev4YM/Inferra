# AI Provider Setup

Inferra uses AI as a guided explanation layer only; it does not change deterministic scoring or ranking (see [AI investigation system](../dossier/ai/ai_investigation_system.md)).

## Gemma model choice

The shipped default in `src/config/defaults.toml` is `gemma4:e4b`, suitable for local Ollama deployments on developer workstations and small servers.

## Local Ollama

Prepare config and probe the daemon:

```powershell
inferra --config inferra.toml config set ai.enabled false
inferra --config inferra.toml config set ai.model gemma4:e4b
inferra --config inferra.toml ai status
inferra --config inferra.toml ai doctor
```

After `ai.enabled` is true and Ollama is reachable at `[ai].base_url`, pull the model with Ollama itself and re-check Inferra:

```powershell
inferra --config inferra.toml config set ai.enabled true
ollama pull gemma4:e4b
inferra --config inferra.toml ai status
inferra --config inferra.toml ai investigate latest
```

Inferra does not implement model download commands itself. Use `ollama pull <tag>` against the configured daemon, then validate with `inferra ai status`, `inferra ai doctor`, `inferra ai ask`, or `inferra ai investigate latest`.

## Remote Ollama-compatible servers

```powershell
inferra --config inferra.toml config set ai.enabled true
inferra --config inferra.toml config set ai.base_url http://SERVER:11434
inferra --config inferra.toml config set ai.allow_remote true
inferra --config inferra.toml config set ai.model gemma4:e4b
inferra --config inferra.toml ai status
inferra --config inferra.toml ai doctor
```

When the remote endpoint is reachable, `inferra ai status` and `inferra ai doctor` probe availability through the native Rust API.

## Bearer token environment variable

When the upstream expects `Authorization: Bearer …`, store the secret outside config files:

```powershell
$env:OLLAMA_TOKEN = "..."
inferra --config inferra.toml config set ai.token_env OLLAMA_TOKEN
```

Inferra reads the variable name from `[ai].token_env` and resolves the value at request time.

## Troubleshooting

- **Unreachable provider:** check `[ai].base_url`, TLS mismatch, and firewall paths; set `allow_remote` when not using loopback.
- **Auth failures:** confirm `token_env` names an exported variable in the service account environment (Windows Service / systemd / container).
- **Missing models:** run `ollama pull <tag>` on the configured daemon, then retry `inferra ai status`.
- **Strict environments:** keep `ai.enabled = false` and rely on template explanations; core pipelines stay offline.

See [Troubleshooting](troubleshooting.md) for Ollama-specific failure modes and collector issues.
