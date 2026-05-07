"""Deprecated PyInstaller entry retained for migration fallback only."""

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


def _frozen_windows_has_console_window() -> bool:
    """Non-zero if this process has a console (user ran from cmd/PowerShell).

    The SCM starts services without a console; those must try
    ``StartServiceCtrlDispatcher``. PyInstaller can load duplicate
    ``pywintypes`` copies, so relying only on ``except pywintypes.error`` for
    ERROR_FAILED_SERVICE_CONTROLLER_CONNECT is brittle. Skip the dispatcher
    entirely when we already know this is an interactive session.
    """
    if os.name != "nt" or not getattr(sys, "frozen", False):
        return False
    try:
        import ctypes

        return bool(ctypes.windll.kernel32.GetConsoleWindow())
    except Exception:
        return False


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
