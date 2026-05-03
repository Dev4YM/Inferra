from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


DEFAULT_CONFIG = Path(os.environ.get("PROGRAMDATA", r"C:\ProgramData")) / "Inferra" / "inferra.toml"


def main() -> int:
    try:
        import servicemanager  # type: ignore
        import win32event  # type: ignore
        import win32service  # type: ignore
        import win32serviceutil  # type: ignore
    except ImportError:
        print("pywin32 is required. Install with: python -m pip install -e .[windows]")
        return 1

    class InferraWindowsService(win32serviceutil.ServiceFramework):  # type: ignore[misc]
        _svc_name_ = "Inferra"
        _svc_display_name_ = "Inferra"
        _svc_description_ = "Local-first runtime failure explanation service"

        def __init__(self, args):
            win32serviceutil.ServiceFramework.__init__(self, args)
            self.stop_event = win32event.CreateEvent(None, 0, 0, None)
            self.process: subprocess.Popen | None = None

        def SvcStop(self):
            self.ReportServiceStatus(win32service.SERVICE_STOP_PENDING)
            if self.process and self.process.poll() is None:
                self.process.terminate()
            win32event.SetEvent(self.stop_event)

        def SvcDoRun(self):
            servicemanager.LogInfoMsg("Inferra service starting")
            config_path = Path(os.environ.get("INFERRA_CONFIG", str(DEFAULT_CONFIG)))
            command = [
                sys.executable,
                "-m",
                "cli",
                "--config",
                str(config_path),
                "serve",
                "--host",
                "0.0.0.0",
            ]
            self.process = subprocess.Popen(command)
            win32event.WaitForSingleObject(self.stop_event, win32event.INFINITE)
            if self.process and self.process.poll() is None:
                self.process.terminate()
                self.process.wait(timeout=15)
            servicemanager.LogInfoMsg("Inferra service stopped")

    win32serviceutil.HandleCommandLine(InferraWindowsService)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
