# ADR 0002: Flat `src/` Layout

## Status

Accepted

## Context

Inferra is built as a GitHub repository with top-level implementation packages under `src/`. The project previously considered a nested `inferra/` package directory, but the desired repository structure keeps modules immediately under `src/`.

## Decision

Inferra uses a flat `src/` layout:

```text
src/
  Cargo.toml
  crates/
  config/
  web/
deprecated/
  inferra_legacy/
    cli.py
  python_packages/
```

No nested implementation package under `src/` or root `inferra/` implementation directory should be introduced. The active runtime lives under the Rust workspace (`src/Cargo.toml`, `src/crates/`) plus frontend assets in `src/web/`; archived Python reference code remains under `deprecated/`.

## Consequences

- Active packaging, service management, and release artifacts target the Rust workspace under `src/`.
- Archived Python code may remain under `deprecated/` for reference, but it is not part of the normal runtime path.
