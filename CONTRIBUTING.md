# Contributing

Thanks for helping make Inferra better.

Inferra is **Rust-first**: the shipping runtime, API, collectors, and CLI live under `src/Cargo.toml`. Python is for CI harnesses, packaging-contract tests, and docs tooling only.

## Local setup

**Rust (required)**

```bash
cargo build --manifest-path src/Cargo.toml -p inferra-cli
cargo test --manifest-path src/Cargo.toml --workspace
```

**Web console** (when you change the React UI)

```bash
cd src/web/frontend
npm ci
npm run build
```

That writes `src/web/ui_dist/` (gitignored build output). The Rust server serves it from there in dev.

**Python dev tooling** (optional; matches CI helpers)

```bash
python -m pip install -e ".[dev]"
```

**Docs** (when you change `docs/` or `mkdocs.yml`)

```bash
python -m pip install -e ".[docs,dev]"
mkdocs build --strict
```

`deprecated/` and `docs/planning/` may exist locally for reference but are **gitignored** and not part of the repo or CI.

## Development rules

- Keep the flattened `src/` Rust layout (`src/crates/*`, not a nested product package tree).
- Keep collectors read-only. Inferra observes systems; it does not remediate them.
- Add **Rust tests** for new collector, storage, API, CLI, or core behavior.
- Keep AI optional. Deterministic collection, storage, correlation, and scoring must work without Ollama.
- Do not commit build artifacts (`src/web/ui_dist/`, `site/`, `target/`, `node_modules/`, etc.).

## Before opening a PR

Run what CI runs on the areas you touched:

```bash
# Rust
cargo fmt --manifest-path src/Cargo.toml --all --check
cargo clippy --manifest-path src/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path src/Cargo.toml --workspace

# Frontend (if you changed src/web/frontend)
cd src/web/frontend && npm ci && npm run build

# Python packaging contracts + Rust integration harness
python -m pip install -e ".[dev]"
python -m compileall tests deploy
python -m ruff check tests deploy
python -m pytest -q tests/unit/test_rust_packaging_contracts.py
INFERRA_BINARY=src/target/release/inferra python -m pytest -q tests/integration/test_rust_api.py -m integration

# Docs (if you changed docs/)
python -m pip install -e ".[docs,dev]"
mkdocs build --strict
```

See [docs/operations/ci.md](docs/operations/ci.md) for the full CI matrix.
