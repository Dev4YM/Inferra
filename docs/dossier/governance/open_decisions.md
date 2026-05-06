# Open Decisions

These are product and implementation decisions. Items marked resolved are now reflected in the repo.

## Product Name Tagline

Resolved:

> Local-first runtime intelligence control plane.

This is now the README direction.

## Official Frontend Location

Resolved:

```text
src/web/frontend
```

`src/web/frontend` is now the official source location because package data already lives under `src/web`.

## Old Static UI Fate

Resolved:

- deleted after React route parity

Archive clutter was avoided because it works against the repo cleanup goal.

## Experience Config

Resolved initial values:

```toml
[experience]
mode = "operator"
ai_role = "investigator"
suggest_safe_actions = true
execute_actions = false
show_raw_evidence_by_default = false
```

Implemented in the config model.

## AI Provider Scope

Current provider:

- Ollama

Potential future:

- OpenAI-compatible local/remote API
- LM Studio
- llama.cpp server

Decision:

- keep Ollama first
- design provider abstraction for later

## Investigation Persistence

Question:

Should AI investigation sessions be persisted like incident chat messages?

Recommendation:

- persist incident-attached investigation sessions
- keep general dashboard AI questions ephemeral unless user exports/saves

## Workspace File Reading

Question:

How much local workspace content can Inferra inspect?

Recommendation:

- default: metadata only
- optional: read config files with redaction
- developer opt-in: deeper file context
- never send secrets to remote providers

## Demo Mode

Question:

Should Inferra ship with demo data?

Recommendation:

- yes
- add `inferra demo seed`
- add web empty-state demo option

This will make the product feel real during onboarding.
