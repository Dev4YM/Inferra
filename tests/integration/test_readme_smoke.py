from __future__ import annotations

import platform
import shlex
from pathlib import Path

import pytest

from cli import main


def test_readme_and_operations_cli_smoke(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    lines: list[tuple[str, str]] = []
    readme = Path("README.md").read_text(encoding="utf-8")
    for line in _powershell_command_lines(readme):
        lines.append(("README.md", line))
    for path in _operations_markdown_paths():
        text = path.read_text(encoding="utf-8")
        for line in _inferra_command_lines_from_fenced_blocks(text):
            lines.append((str(path), line))
    assert lines, "Expected inferra command lines in README and docs/operations"
    monkeypatch.chdir(tmp_path)
    for source, command in lines:
        argv = _argv_from_command_line(command)
        if argv is None:
            continue
        if _skip_cli_smoke(argv):
            continue
        try:
            exit_code = main(argv)
        except SystemExit as exc:
            assert exc.code == 0, f"Command failed ({source}): {command}"
            continue
        assert exit_code == 0, f"Command failed ({source}): {command}"


def _operations_markdown_paths() -> list[Path]:
    base = Path("docs/operations")
    files = sorted(base.glob("*.md"))
    install = base / "install.md"
    if install in files:
        files.remove(install)
        files.insert(0, install)
    return files


def _powershell_command_lines(markdown: str) -> list[str]:
    return _command_lines_from_fence(markdown, "```powershell")


def _inferra_command_lines_from_fenced_blocks(markdown: str) -> list[str]:
    lines: list[str] = []
    lines.extend(_command_lines_from_fence(markdown, "```powershell"))
    lines.extend(_command_lines_from_fence(markdown, "```bash"))
    return lines


def _command_lines_from_fence(markdown: str, fence: str) -> list[str]:
    lines: list[str] = []
    in_block = False
    for raw in markdown.splitlines():
        stripped = raw.strip()
        if stripped == fence:
            in_block = True
            continue
        if stripped == "```" and in_block:
            in_block = False
            continue
        if not in_block or not stripped or stripped.startswith("#"):
            continue
        lines.append(stripped)
    return lines


def _argv_from_command_line(command: str) -> list[str] | None:
    if command.startswith("inferra "):
        return shlex.split(command, posix=False)[1:]
    if command.startswith("python -m cli "):
        return shlex.split(command, posix=False)[3:]
    return None


def _skip_cli_smoke(argv: list[str]) -> bool:
    if "--help" in argv or "-h" in argv:
        return False
    if _argv_contains_long_running(argv):
        return True
    if _argv_requires_live_api(argv):
        return True
    if _argv_requires_ollama_network(argv):
        return True
    if _argv_requires_service_admin(argv):
        return True
    if _argv_platform_mismatch(argv):
        return True
    return False


def _strip_global_cli_flags(argv: list[str]) -> list[str]:
    out: list[str] = []
    i = 0
    while i < len(argv):
        if argv[i] == "--config" and i + 1 < len(argv):
            i += 2
            continue
        if argv[i] == "--data-dir" and i + 1 < len(argv):
            i += 2
            continue
        if argv[i] == "--json":
            i += 1
            continue
        out.append(argv[i])
        i += 1
    return out


def _argv_contains_long_running(argv: list[str]) -> bool:
    parts = _strip_global_cli_flags(argv)
    if not parts:
        return False
    cmd = parts[0]
    rest = parts[1:]
    if cmd == "serve":
        return "--help" not in rest and "-h" not in rest
    if cmd == "run-collectors":
        return "--help" not in rest and "-h" not in rest
    if cmd == "run":
        return "--help" not in rest and "-h" not in rest
    return False


def _argv_requires_live_api(argv: list[str]) -> bool:
    parts = _strip_global_cli_flags(argv)
    if not parts:
        return False
    if parts[0] in {"overview", "investigate", "incidents", "events", "services"}:
        return True
    if "collectors" in parts and ("start" in parts or "stop" in parts):
        return True
    return False


def _argv_requires_ollama_network(argv: list[str]) -> bool:
    if "--help" in argv or "-h" in argv:
        return False
    if "ai" not in argv:
        return False
    if "pull" in argv or "test" in argv:
        return True
    return False


def _argv_platform_mismatch(argv: list[str]) -> bool:
    sysname = platform.system().lower()
    joined = " ".join(argv)
    if sysname != "linux":
        if "collect-syslog" in joined or "collect-journald" in joined:
            return True
    if sysname != "windows":
        if "collect-services" in joined or "collect-eventlog" in joined:
            return True
    return False


def _argv_requires_service_admin(argv: list[str]) -> bool:
    parts = _strip_global_cli_flags(argv)
    if not parts or parts[0] != "service":
        return False
    return any(verb in parts for verb in ("install", "start", "stop", "restart", "remove"))
