# Continuous integration

Reference runners (documented 2026-06): GitHub Actions `ubuntu-latest`, Python 3.12, Node 20, Docker, Helm, and the stable Rust toolchain.

## Default gate

- Python tooling and Rust packaging contracts: `python -m pip install -e ".[dev]"`, `compileall`, `ruff`, and `pytest tests/unit/test_rust_packaging_contracts.py`.
- Frontend build: `npm ci && npm run build` in `src/web/frontend`.
- Docs: `python -m pip install -e ".[docs,dev]"` then `mkdocs build --strict`.
- Rust checks: `cargo fmt --manifest-path src/Cargo.toml --all --check`, `cargo clippy --manifest-path src/Cargo.toml --workspace --all-targets -- -D warnings`, `cargo test --manifest-path src/Cargo.toml --workspace`.
- Native runtime smoke: build the release CLI binary, run `tests/scripts/rust_runtime_smoke.py`, then run `tests/integration/test_rust_api.py` against the built executable.
- Helm: `helm lint`, render the chart, and assert rendered auth/probe/security fields.
- Docker: build the image, run it with the container config, and probe `/healthz`.
- Coverage: Rust coverage is measured by `cargo llvm-cov`; Python archive coverage is not the product coverage signal.

## Archived Python Runtime

The `legacy-archive` job installs `.[dev,legacy]` and runs the deprecated Python runtime tests under an explicitly named job. These tests protect archived reference behavior only; shipping runtime confidence comes from Rust workspace tests and Rust black-box integration.

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
