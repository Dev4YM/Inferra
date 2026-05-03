# ADR 0002: Flat `src/` Layout

## Status

Accepted

## Context

Inferra is built as a GitHub repository with top-level implementation packages under `src/`. The project previously considered a nested `inferra/` package directory, but the desired repository structure keeps modules immediately under `src/`.

## Decision

Inferra uses a flat `src/` layout:

```text
src/
  ai/
  collectors/
  config/
  storage/
  web/
  app.py
  cli.py
```

No nested `src/inferra/` or root `inferra/` implementation directory should be introduced.

## Consequences

- `pyproject.toml` sets `package-dir = {"" = "src"}`.
- Console entrypoint remains `inferra = "cli:main"`.
- Imports remain concise, for example `from config import load_config`.
