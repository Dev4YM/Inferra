# Contributing

Thanks for helping make Inferra better.

## Local Setup

```bash
python -m pip install -e ".[dev,legacy]"
python -m pytest -q
```

## Development Rules

- Keep the flattened `src/` layout. Do not add a nested `inferra/` package directory.
- Keep collectors read-only. Inferra observes systems; it does not remediate them.
- Add tests for new collector, reasoning, storage, CLI, or API behavior.
- Keep AI optional. Deterministic collection, storage, correlation, and scoring must work without Ollama.

## Before Opening a PR

```bash
python -m compileall tests deploy deprecated
python -m pytest -q
```
