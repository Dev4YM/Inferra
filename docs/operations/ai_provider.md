# AI Provider Setup

Inferra uses AI as a guided explanation layer. It does not mutate systems or change deterministic scoring.

## Ollama

Local setup:

```powershell
ollama pull gemma4:e4b
inferra setup --yes --model gemma4:e4b
inferra ai test
```

Remote setup:

```powershell
inferra setup --yes --remote-url http://SERVER:11434 --model gemma4:e4b
```

Optional bearer-token auth:

```powershell
$env:OLLAMA_TOKEN = "..."
inferra config set ai.token_env OLLAMA_TOKEN
```

No large model is pulled unless `inferra ai pull --yes` or setup is run with `--pull --yes`.
