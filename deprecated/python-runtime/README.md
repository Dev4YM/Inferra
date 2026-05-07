# Legacy Python Runtime Surface

The preferred public runtime is the Rust workspace (`src/Cargo.toml`, `src/crates/`).
Python remains for the internal AI worker, compatibility, and migration:

- **Legacy CLI / pywin32 helpers:** `deprecated/inferra_legacy/` (`cli.py`,
  `windows_service.py`, `app.py`). Console script `inferra-python-legacy` targets
  `inferra_legacy.cli:main`.
- **Legacy HTTP:** `src/web/api.py` while Rust parity and proxy removal finish.

Current ownership:

- `src/crates/inferra-cli` — public CLI binary sources.
- `src/crates/inferra-windows-service` — Windows SCM integration.
- `src/crates/inferra-api` — local HTTP API (Rust).
- `src/ai/worker/` — internal FastAPI worker (`ai.worker`).
