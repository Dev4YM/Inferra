# Continuous integration

Reference runner (documented 2026-05): GitHub Actions `ubuntu-latest` / `windows-latest`, Python 3.11 and 3.12.

## Default gate

- Install: `python -m pip install -e ".[dev]"`.
- Compile: `python -m compileall src tests`.
- Lint: `python -m ruff check src tests`.
- Tests: matrix-specific pytest invocation (see below).
- Coverage (optional local / CI): `python -m pytest --cov=src --cov-report=term` — scoped packages use `[tool.coverage.report]` in `pyproject.toml` with `fail_under = 80`.

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

## UI smoke

Playwright UI tests stay optional (`pip install -e ".[ui]"`). Not required for the standard CI gate unless enabled in workflow.

## Windows tagged releases (`inferra.exe`)

Tag pushes (`v*`) build the Windows artifact on **`windows-latest`** using **`deploy/windows/build-exe.ps1`** after `pip install -e ".[windows,build-windows]"`. The script writes PyInstaller output to **`dist/_inferra_exe_stage/`**, promotes **`dist/inferra.exe`**, and smoke-tests **`inferra.exe --version`**. Details: [Windows exe build](windows_exe_build.md).
