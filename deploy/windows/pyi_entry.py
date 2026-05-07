"""Compatibility shim for the deprecated PyInstaller entry."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import os
import sys

_IMPL_PATH = (
    Path(__file__).resolve().parents[2] / "deprecated" / "windows-pyinstaller" / "pyi_entry.py"
)
_SPEC = importlib.util.spec_from_file_location("inferra_deprecated_pyi_entry", _IMPL_PATH)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Could not load deprecated PyInstaller entry: {_IMPL_PATH}")
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)

_frozen_windows_has_console_window = _MODULE._frozen_windows_has_console_window
_argv_dispatches_to_windows_service = _MODULE._argv_dispatches_to_windows_service


def main() -> int:
    if _argv_dispatches_to_windows_service(sys.argv):
        import inferra_legacy.windows_service as windows_service

        return windows_service.main()
    if (
        os.name == "nt"
        and getattr(sys, "frozen", False)
        and len(sys.argv) == 1
        and not _frozen_windows_has_console_window()
    ):
        import inferra_legacy.windows_service as windows_service

        if windows_service.try_run_frozen_windows_service():
            return 0
    import inferra_legacy.cli as cli

    return cli.main()

if __name__ == "__main__":
    raise SystemExit(main())
