# Legacy PyInstaller Windows Build

This directory holds the deprecated Python-first Windows executable path.

## Why deprecated

Inferra now builds and ships a native Rust runtime shell via
`deploy/windows/build-rust-exe.ps1`. The PyInstaller path remains archived only
for migration fallback and forensic reference.

## Archived entrypoints

- `build-exe.ps1`
- `prepare-build-venv.ps1`
- `pyi_entry.py`
- `read_pyproject_version.py`

Compatibility shims may still exist under `deploy/windows/` so old operator
habits do not fail abruptly, but the implementation of record for the legacy
path now lives here.
