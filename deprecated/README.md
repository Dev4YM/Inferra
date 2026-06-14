# Deprecated Surfaces

This folder holds code and build paths that are no longer part of the active
Inferra runtime.

## Active runtime (outside this folder)

- **Rust workspace:** `src/Cargo.toml` and `src/crates/` own the live CLI, API,
  collectors, storage, service integration, and investigation runtime.
- **Frontend:** `src/web/frontend/` builds the shipped React UI, and
  `src/web/ui_dist/` is the Vite build output consumed by the Rust server (gitignored — run `scripts/build-web.ps1` or `npm run build` in `src/web/frontend`).
- **Shared config/assets:** `src/config/defaults.toml` and related runtime assets
  remain live because the Rust runtime reads and ships them.

Nothing in `deprecated/` is required for the default Rust-first runtime.

## Archived here

- `deprecated/windows-pyinstaller/` — legacy PyInstaller-based Windows build flow.
- `deprecated/python-runtime/README.md` — historical notes on the Python-first surface.
- `deprecated/inferra_legacy/` — legacy compatibility CLI / service helpers exposed
  only through the archived `inferra-python-legacy` entry point.

## Boundary rule

Do not treat `deprecated/` as a runtime dependency tree. If a path is needed by
the shipped product, it belongs under the active Rust/frontend `src/` layout.
If a path exists only for historical reference, migration context, or legacy
compatibility, it belongs here.
