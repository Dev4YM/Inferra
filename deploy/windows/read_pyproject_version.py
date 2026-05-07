"""Compatibility shim for the deprecated PyInstaller version helper."""

from __future__ import annotations

import importlib.util
from pathlib import Path

_IMPL_PATH = (
    Path(__file__).resolve().parents[2]
    / "deprecated"
    / "windows-pyinstaller"
    / "read_pyproject_version.py"
)
_SPEC = importlib.util.spec_from_file_location("inferra_deprecated_read_version", _IMPL_PATH)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Could not load deprecated version helper: {_IMPL_PATH}")
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)

main = _MODULE.main


if __name__ == "__main__":
    main()
