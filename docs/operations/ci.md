# Continuous integration

Reference runners (documented 2026-05): GitHub Actions `ubuntu-latest` / `windows-latest` / `macos-latest`, Python 3.11 and 3.12, Node 20, and the stable Rust toolchain.

## Default gate

- Install Python support tooling: `python -m pip install -e ".[dev,legacy]"` (the `legacy` extra pulls archived `deprecated/` runtime deps used by pytest).
- Frontend build: `npm ci && npm run build` in `src/web/frontend`.
- Docs: `python -m pip install -e ".[docs,dev]"` then `mkdocs build --strict`.
- Rust checks: `cargo fmt --manifest-path src/Cargo.toml --all --check`, `cargo clippy --manifest-path src/Cargo.toml --workspace --all-targets -- -D warnings`, `cargo test --manifest-path src/Cargo.toml --workspace`.
- Native runtime smoke: build the release CLI binary and run `tests/scripts/rust_runtime_smoke.py` against the built executable plus `src/web/ui_dist`.
- Compile: `python -m compileall tests deploy deprecated`.
- Lint: `python -m ruff check tests deploy deprecated`.
- Tests: matrix-specific pytest invocation (see below).
- Coverage (optional local only): `python -m pytest --cov --cov-report=term` — coverage config in `pyproject.toml` now scopes the remaining Python support surfaces (`tests`, `deploy/windows`, `deprecated/inferra_legacy`) rather than the active Rust runtime.

## Platform matrix

- **Linux and macOS runners**: `python -m pytest -q -m "not windows and not chaos"` — skips tests marked `@pytest.mark.windows` (Windows-oriented collectors and mocks) and `@pytest.mark.chaos` (POSIX-heavy failure injection; see dedicated chaos job).
- **Windows runner**: `python -m pytest -q -m "not linux and not chaos"` — skips tests marked `@pytest.mark.linux` (journald / Linux syslog harness) and chaos tests.

Unmarked tests always execute on every OS.

## Chaos (`pytest.mark.chaos`)

Linux CI job (or local on macOS/Linux):

```bash
python -m pytest -q -m chaos --tb=short
```

Covers SQLite SIGKILL mid-transaction, Ollama mid-stream failure, disk-full degradation, and clock-skew normalization flags.

## Performance budgets (`pytest.mark.perf`)

Job or optional workflow step:

```bash
python -m pytest -q -m perf --tb=short
```

Budgets are asserted in `tests/perf/test_budgets.py` (same thresholds as product targets):

| Area | Metric | Budget |
|------|--------|--------|
| Normalization | p99 time per `NormalizationPipeline.normalize` sample | <= 2 ms |
| Analysis | Wall time for `aggregate_events_into_bucket_rows` over 500 normalized events | <= 50 ms |
| Scoring | p99 time for `compute_score_breakdown` per hypothesis | <= 5 ms |

Artifact: set `PERF_REPORT_PATH` to write JSON (defaults to `./perf_report.json` in the working directory). CI should upload that path as a workflow artifact.

## Determinism

`tests/determinism/` is marked `@pytest.mark.determinism`. It runs in the default suite: ranking tuples (rank, cause type, rounded score) must stay identical across repeated runs with fixed timestamps.

## Frontend build

The standard CI gate now requires a production frontend build:

```bash
cd src/web/frontend
npm ci
npm run build
```

Playwright UI tests still stay optional (`pip install -e ".[ui]"`). They are not part of the default gate unless added explicitly.

## Runtime smoke

The binary-level smoke gate builds the actual Rust CLI and boots it against a temp config/data directory:

```bash
cargo build --manifest-path src/Cargo.toml -p inferra-cli --release
python tests/scripts/rust_runtime_smoke.py --binary ./target/release/inferra --repo-root .
```

The script verifies `setup`, `init-db`, `serve`, `/api/health`, `/api/overview`, `/api/collectors`, and the CLI-to-local-API collector status path.

## Windows tagged releases (`inferra.exe`)

Tag pushes (`v*`) now build the Windows artifact on **`windows-latest`** using the Rust-native path. The old PyInstaller flow has been archived under `deprecated/windows-pyinstaller/` and remains documented only as a fallback in [Windows exe build](windows_exe_build.md).
