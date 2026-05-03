from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

try:
    import servicemanager  # type: ignore
    import win32event  # type: ignore
    import win32service  # type: ignore
    import win32serviceutil  # type: ignore
except ImportError:
    servicemanager = None
    win32event = None
    win32service = None
    win32serviceutil = None


DEFAULT_CONFIG = Path(os.environ.get("PROGRAMDATA", r"C:\ProgramData")) / "Inferra" / "inferra.toml"
SERVICE_CLASS = "windows_service.InferraWindowsService"


def _service_config_path() -> Path:
    return Path(os.environ.get("INFERRA_CONFIG", str(DEFAULT_CONFIG)))


def _python_executable() -> str:
    base_executable = getattr(sys, "_base_executable", "")
    candidates = [
        base_executable,
        sys.executable,
        str(Path(sys.exec_prefix) / "python.exe"),
    ]
    for candidate in candidates:
        if candidate and Path(candidate).exists() and Path(candidate).name.lower() != "pythonservice.exe":
            return candidate
    return sys.executable


if win32serviceutil is not None:
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
            command = [
                _python_executable(),
                "-m",
                "cli",
                "--config",
                str(_service_config_path()),
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


def main() -> int:
    if win32serviceutil is None:
        print("pywin32 is required. Install with: python -m pip install -e .[windows]")
        return 1

    return int(win32serviceutil.HandleCommandLine(InferraWindowsService, serviceClassString=SERVICE_CLASS) or 0)


if __name__ == "__main__":
    raise SystemExit(main())
