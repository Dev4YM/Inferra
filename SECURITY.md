# Security

Inferra is local-first and stores operational evidence in SQLite under the configured data directory.

## Reporting Issues

Please report security issues privately through the repository security advisory flow when available.

## Data Handling

- AI is optional and disabled by default.
- Ollama calls are sent only to the configured base URL.
- Prompt construction redacts common secret fields and authorization tokens.
- Inferra collectors are intended to be read-only.
