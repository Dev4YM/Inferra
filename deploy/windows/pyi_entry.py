"""PyInstaller entry: Windows SCM verbs dispatch to windows_service; otherwise CLI."""

from __future__ import annotations

import os
import sys

_WINDOWS_SCM_VERBS = frozenset(
    {
        "install",
        "remove",
        "update",
        "start",
        "stop",
        "restart",
        "debug",
        "pause",
        "continue",
    }
)


def _argv_dispatches_to_windows_service(argv: list[str]) -> bool:
    if len(argv) < 2:
        return False
    if argv[1] in _WINDOWS_SCM_VERBS:
        return True
    if argv[1].startswith("-"):
        return any(token in _WINDOWS_SCM_VERBS for token in argv[2:])
    return False


def main() -> int:
    if _argv_dispatches_to_windows_service(sys.argv):
        import windows_service

        return windows_service.main()
    # SCM starts the frozen exe with argv == [exe] only; pywin32 must host via
    # servicemanager (see win32serviceutil.HandleCommandLine vs frozen dispatch).
    if (
        os.name == "nt"
        and getattr(sys, "frozen", False)
        and len(sys.argv) == 1
    ):
        import windows_service

        if windows_service.try_run_frozen_windows_service():
            return 0
    import cli

    return cli.main()


if __name__ == "__main__":
    raise SystemExit(main())
