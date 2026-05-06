from __future__ import annotations

import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from core.logging import get_logger

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

_LOGGER = get_logger(__name__)

DEFAULT_CONFIG = Path(os.environ.get("PROGRAMDATA", r"C:\ProgramData")) / "Inferra" / "inferra.toml"
SERVICE_CLASS = "windows_service.InferraWindowsService"
_RUNTIME_FILENAME = "service_runtime.json"


def _program_data_root() -> Path:
    return Path(os.environ.get("PROGRAMDATA", r"C:\ProgramData")) / "Inferra"


def service_runtime_path(program_data: Path | None = None) -> Path:
    base = program_data if program_data is not None else _program_data_root()
    return base / _RUNTIME_FILENAME


def serve_log_path(program_data: Path | None = None) -> Path:
    base = program_data if program_data is not None else _program_data_root()
    return base / "logs" / "serve.log"


def _read_server_port(config_path: Path) -> int | None:
    if not config_path.is_file():
        return None
    try:
        import tomllib

        data = tomllib.loads(config_path.read_text(encoding="utf-8"))
    except Exception:
        return None
    sec = data.get("server")
    if not isinstance(sec, dict):
        return None
    raw = sec.get("port")
    if raw is None:
        return None
    try:
        return int(raw)
    except (TypeError, ValueError):
        return None


@dataclass(frozen=True, slots=True)
class ServiceRuntimeOptions:
    config_path: Path
    data_dir: Path | None


@dataclass(frozen=True, slots=True)
class ParsedInstallArgv:
    """Flags consumed from argv before passing the remainder to pywin32."""

    argv_for_pywin32: list[str]
    config_path: Path | None
    data_dir: Path | None


_PYWIN32_VERBS = frozenset(
    {"install", "remove", "update", "start", "stop", "restart", "debug", "pause", "continue"},
)


def _reorder_argv_for_pywin32_getopt(parts: list[str]) -> list[str]:
    """win32serviceutil.HandleCommandLine uses getopt: long options must appear before the verb."""
    if len(parts) < 2:
        return parts
    exe = parts[0]
    rest = parts[1:]
    vi = next((i for i, x in enumerate(rest) if x.lower() in _PYWIN32_VERBS), None)
    if vi is None:
        return parts
    verb = rest[vi]
    before = rest[:vi]
    after = rest[vi + 1 :]
    if vi == 0:
        if not after:
            return parts
        return [exe, *after, verb]
    return [exe, *before, verb, *after]


def parse_install_argv(argv: list[str]) -> ParsedInstallArgv:
    has_install = any(i > 0 and a.lower() == "install" for i, a in enumerate(argv))
    if not has_install:
        return ParsedInstallArgv(argv_for_pywin32=list(argv), config_path=None, data_dir=None)

    config_path: Path | None = None
    data_dir: Path | None = None
    cleaned: list[str] = []
    i = 0
    while i < len(argv):
        item = argv[i]
        if i == 0:
            cleaned.append(item)
            i += 1
            continue
        if item == "--config" and i + 1 < len(argv):
            config_path = Path(argv[i + 1]).expanduser().resolve()
            i += 2
            continue
        if item.startswith("--config="):
            config_path = Path(item.split("=", 1)[1]).expanduser().resolve()
            i += 1
            continue
        if item == "--data-dir" and i + 1 < len(argv):
            data_dir = Path(argv[i + 1]).expanduser().resolve()
            i += 2
            continue
        if item.startswith("--data-dir="):
            data_dir = Path(item.split("=", 1)[1]).expanduser().resolve()
            i += 1
            continue
        cleaned.append(item)
        i += 1
    reordered = _reorder_argv_for_pywin32_getopt(cleaned)
    return ParsedInstallArgv(argv_for_pywin32=reordered, config_path=config_path, data_dir=data_dir)


def write_service_runtime(options: ServiceRuntimeOptions, program_data: Path | None = None) -> Path:
    path = service_runtime_path(program_data)
    path.parent.mkdir(parents=True, exist_ok=True)
    payload: dict[str, Any] = {"config_path": str(options.config_path)}
    if options.data_dir is not None:
        payload["data_dir"] = str(options.data_dir)
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return path


def read_service_runtime(program_data: Path | None = None) -> ServiceRuntimeOptions | None:
    path = service_runtime_path(program_data)
    if not path.is_file():
        return None
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        _LOGGER.warning(
            "service_runtime.json unreadable; falling back to defaults",
            extra={"path": str(path), "error": str(exc)},
        )
        return None
    config_raw = raw.get("config_path") or raw.get("config")
    if not config_raw:
        return None
    data_raw = raw.get("data_dir")
    return ServiceRuntimeOptions(
        config_path=Path(str(config_raw)).expanduser(),
        data_dir=Path(str(data_raw)).expanduser() if data_raw else None,
    )


def _resolved_config_path(runtime: ServiceRuntimeOptions | None) -> Path:
    if runtime is not None:
        return runtime.config_path
    env = os.environ.get("INFERRA_CONFIG", "").strip()
    if env:
        return Path(env).expanduser()
    return DEFAULT_CONFIG


def _resolved_data_dir(runtime: ServiceRuntimeOptions | None) -> Path | None:
    if runtime is None or runtime.data_dir is None:
        return None
    return runtime.data_dir


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


def _build_serve_argv(config_path: Path, data_dir: Path | None) -> list[str]:
    """Argv for `inferra serve` child process.

    Global flags (`--config`) belong before the subcommand. Flags defined only on the
    `serve` subparser (`--data-dir`, `--host`, `--port`) must appear *after* ``serve`` —
    otherwise argparse treats the data-dir path as the subcommand name.
    """
    port = _read_server_port(config_path)
    if getattr(sys, "frozen", False):
        argv = [sys.executable, "--config", str(config_path), "serve"]
        if data_dir is not None:
            argv.extend(["--data-dir", str(data_dir)])
        argv.extend(["--host", "0.0.0.0"])
        if port is not None:
            argv.extend(["--port", str(port)])
        return argv
    cmd = [_python_executable(), "-m", "cli", "--config", str(config_path), "serve"]
    if data_dir is not None:
        cmd.extend(["--data-dir", str(data_dir)])
    cmd.extend(["--host", "0.0.0.0"])
    if port is not None:
        cmd.extend(["--port", str(port)])
    return cmd


def _popen_kwargs() -> dict[str, Any]:
    kwargs: dict[str, Any] = {}
    if os.name == "nt":
        kwargs["creationflags"] = getattr(subprocess, "CREATE_NO_WINDOW", 0)
    return kwargs


if win32serviceutil is not None:

    class InferraWindowsService(win32serviceutil.ServiceFramework):  # type: ignore[misc]
        _svc_name_ = "Inferra"
        _svc_display_name_ = "Inferra"
        _svc_description_ = "Local-first runtime failure explanation service"

        def __init__(self, args: list[str]) -> None:
            win32serviceutil.ServiceFramework.__init__(self, args)
            self.stop_event = win32event.CreateEvent(None, 0, 0, None)
            self.process: subprocess.Popen | None = None
            self._stopping = False
            self._serve_log_fp: Any = None

        def SvcStop(self) -> None:
            self.ReportServiceStatus(win32service.SERVICE_STOP_PENDING)
            self._stopping = True
            if self.process and self.process.poll() is None:
                self.process.terminate()
            win32event.SetEvent(self.stop_event)

        def SvcDoRun(self) -> None:
            servicemanager.LogInfoMsg("Inferra service starting")
            runtime = read_service_runtime()
            log_path = serve_log_path()
            log_path.parent.mkdir(parents=True, exist_ok=True)

            config_path = _resolved_config_path(runtime)
            data_dir = _resolved_data_dir(runtime)
            command = _build_serve_argv(config_path, data_dir)
            _LOGGER.info("starting child process", extra={"command": command})
            popen_out: Any
            try:
                self._serve_log_fp = open(log_path, "a", encoding="utf-8", buffering=1)
                self._serve_log_fp.write(f"\n--- inferra serve ({config_path}) ---\n")
                self._serve_log_fp.flush()
                popen_out = self._serve_log_fp
                popen_err = subprocess.STDOUT
            except OSError as exc:
                self._serve_log_fp = None
                servicemanager.LogErrorMsg(
                    f"Inferra cannot write serve log at {log_path} ({exc}); child output discarded."
                )
                popen_out = subprocess.DEVNULL
                popen_err = subprocess.DEVNULL

            self.process = subprocess.Popen(
                command,
                stdin=subprocess.DEVNULL,
                stdout=popen_out,
                stderr=popen_err,
                **_popen_kwargs(),
            )
            poll_ms = 5000
            while True:
                rc = win32event.WaitForSingleObject(self.stop_event, poll_ms)
                if rc == win32event.WAIT_OBJECT_0:
                    break
                if self.process.poll() is not None:
                    code = self.process.returncode if self.process.returncode is not None else 0
                    if code != 0 and not self._stopping:
                        servicemanager.LogErrorMsg(
                            f"Inferra serve process exited early (code {code}). Log: {log_path}"
                        )
                    break

            if self.process and self.process.poll() is None:
                self.process.terminate()
                try:
                    self.process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    self.process.kill()

            try:
                if self._serve_log_fp is not None:
                    self._serve_log_fp.close()
            finally:
                self._serve_log_fp = None

            servicemanager.LogInfoMsg("Inferra service stopped")

    def try_run_frozen_windows_service() -> bool:
        """Host the SCM-launched frozen exe (argv is only the executable path).

        Returns True if the service dispatcher ran (process is exiting). Returns False
        if this process was not started by the SCM so the caller should run the CLI.
        """
        if not getattr(sys, "frozen", False) or os.name != "nt" or len(sys.argv) != 1:
            return False
        import pywintypes  # type: ignore[import-untyped]
        import winerror

        servicemanager.Initialize()
        servicemanager.PrepareToHostSingle(InferraWindowsService)
        scm_failed_connect = winerror.ERROR_FAILED_SERVICE_CONTROLLER_CONNECT
        try:
            servicemanager.StartServiceCtrlDispatcher()
        except pywintypes.error as exc:
            # Some pywin32 builds only populate args=(winerror, funcname, msg), not .winerror.
            code = getattr(exc, "winerror", None)
            if code is None and exc.args:
                code = exc.args[0]
            if int(code) != int(scm_failed_connect):
                raise
            return False
        return True

else:

    def try_run_frozen_windows_service() -> bool:
        return False


def main() -> int:
    if win32serviceutil is None:
        _LOGGER.error("pywin32 is required for the Windows service helper")
        return 1
    argv = list(sys.argv)
    parsed = parse_install_argv(argv)
    if parsed.config_path is not None or parsed.data_dir is not None:
        prior = read_service_runtime()
        cfg = parsed.config_path or (prior.config_path if prior is not None else DEFAULT_CONFIG)
        data_dir_merged = parsed.data_dir if parsed.data_dir is not None else (prior.data_dir if prior is not None else None)
        write_service_runtime(ServiceRuntimeOptions(config_path=cfg, data_dir=data_dir_merged))
    sys.argv = parsed.argv_for_pywin32
    return int(win32serviceutil.HandleCommandLine(InferraWindowsService, serviceClassString=SERVICE_CLASS) or 0)


if __name__ == "__main__":
    raise SystemExit(main())
