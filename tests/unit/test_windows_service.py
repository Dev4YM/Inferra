from __future__ import annotations

import json
from pathlib import Path

import pytest

from inferra_legacy.windows_service import (
    ServiceRuntimeOptions,
    _read_server_port,
    parse_install_argv,
    read_service_runtime,
    write_service_runtime,
)


def test_build_serve_argv_puts_serve_before_subparser_flags(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    import inferra_legacy.windows_service as ws

    cfg = tmp_path / "inferra.toml"
    cfg.write_text("[server]\nport = 7433\n", encoding="utf-8")
    data = tmp_path / "data"
    monkeypatch.setattr(ws.sys, "frozen", True, raising=False)
    monkeypatch.setattr(ws.sys, "executable", r"C:\fake\inferra.exe")

    argv = ws._build_serve_argv(cfg, data)

    assert argv[0] == r"C:\fake\inferra.exe"
    assert argv[1:8] == ["--config", str(cfg), "serve", "--data-dir", str(data), "--host", "0.0.0.0"]
    assert argv[8:] == ["--port", "7433"]


def test_build_serve_argv_non_frozen_puts_serve_before_data_dir(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    import inferra_legacy.windows_service as ws

    cfg = tmp_path / "inferra.toml"
    cfg.write_text("[server]\nport = 80\n", encoding="utf-8")
    monkeypatch.setattr(ws.sys, "frozen", False, raising=False)
    monkeypatch.setattr(ws, "_python_executable", lambda: r"C:\Py\python.exe")

    argv = ws._build_serve_argv(cfg, tmp_path / "state")

    assert argv[:8] == [
        r"C:\Py\python.exe",
        "-m",
        "inferra_legacy.cli",
        "--config",
        str(cfg),
        "serve",
        "--data-dir",
        str(tmp_path / "state"),
    ]
    assert argv[8:] == ["--host", "0.0.0.0", "--port", "80"]


def test_parse_install_argv_pass_through_non_install() -> None:
    argv = ["py", "start"]
    parsed = parse_install_argv(argv)
    assert parsed.argv_for_pywin32 == argv
    assert parsed.config_path is None
    assert parsed.data_dir is None


def test_parse_install_argv_strips_flags(tmp_path: Path) -> None:
    cfg = tmp_path / "inferra.toml"
    data = tmp_path / "data"
    argv = ["py", "install", "--startup", "auto", "--config", str(cfg), "--data-dir", str(data)]
    parsed = parse_install_argv(argv)
    assert parsed.argv_for_pywin32 == ["py", "--startup", "auto", "install"]
    assert parsed.config_path == cfg.resolve()
    assert parsed.data_dir == data.resolve()


def test_parse_install_argv_equals_forms(tmp_path: Path) -> None:
    cfg = tmp_path / "a.toml"
    data = tmp_path / "b"
    argv = ["py", "install", f"--config={cfg}", f"--data-dir={data}"]
    parsed = parse_install_argv(argv)
    assert parsed.argv_for_pywin32 == ["py", "install"]
    assert parsed.config_path == cfg.resolve()
    assert parsed.data_dir == data.resolve()


def test_parse_install_argv_options_before_install_unchanged(tmp_path: Path) -> None:
    cfg = tmp_path / "c.toml"
    argv = ["py", "--startup", "manual", "install", "--config", str(cfg)]
    parsed = parse_install_argv(argv)
    assert parsed.argv_for_pywin32 == ["py", "--startup", "manual", "install"]
    assert parsed.config_path == cfg.resolve()
    opts = ServiceRuntimeOptions(config_path=tmp_path / "c.toml", data_dir=tmp_path / "d")
    path = write_service_runtime(opts, program_data=tmp_path)
    assert path == tmp_path / "service_runtime.json"
    loaded = read_service_runtime(program_data=tmp_path)
    assert loaded is not None
    assert loaded.config_path == opts.config_path
    assert loaded.data_dir == opts.data_dir


def test_read_service_runtime_legacy_config_key(tmp_path: Path) -> None:
    legacy = {"config": str(tmp_path / "x.toml"), "data_dir": str(tmp_path / "y")}
    (tmp_path / "service_runtime.json").write_text(json.dumps(legacy), encoding="utf-8")
    loaded = read_service_runtime(program_data=tmp_path)
    assert loaded is not None
    assert loaded.config_path == tmp_path / "x.toml"
    assert loaded.data_dir == tmp_path / "y"


def test_read_server_port_from_config(tmp_path: Path) -> None:
    cfg = tmp_path / "inferra.toml"
    cfg.write_text("[server]\nport = 9911\n", encoding="utf-8")
    assert _read_server_port(cfg) == 9911


def test_read_server_port_invalid_returns_none(tmp_path: Path) -> None:
    cfg = tmp_path / "inferra.toml"
    cfg.write_text("[server]\nport = not-int\n", encoding="utf-8")
    assert _read_server_port(cfg) is None


def test_read_server_port_missing_file(tmp_path: Path) -> None:
    assert _read_server_port(tmp_path / "missing.toml") is None


def test_main_without_pywin32_returns_one() -> None:
    import inferra_legacy.windows_service as ws

    if ws.win32serviceutil is not None:
        pytest.skip("pywin32 is installed")
    assert ws.main() == 1
