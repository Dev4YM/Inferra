from __future__ import annotations

import importlib.util
import os
import sys
from pathlib import Path

import pytest


def _load_pyi_entry():
    root = Path(__file__).resolve().parents[2]
    path = root / "deploy" / "windows" / "pyi_entry.py"
    spec = importlib.util.spec_from_file_location("inferra_pyi_entry_test", path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def test_dispatch_windows_service_when_verb_is_first() -> None:
    pe = _load_pyi_entry()
    assert pe._argv_dispatches_to_windows_service(["inferra.exe", "install", "--startup", "auto"]) is True


def test_dispatch_windows_service_when_options_precede_install() -> None:
    pe = _load_pyi_entry()
    assert (
        pe._argv_dispatches_to_windows_service(
            [
                "inferra.exe",
                "--startup",
                "auto",
                "install",
                "--config",
                "C:\\ProgramData\\Inferra\\inferra.toml",
            ]
        )
        is True
    )


def test_dispatch_cli_for_serve() -> None:
    pe = _load_pyi_entry()
    assert pe._argv_dispatches_to_windows_service(["inferra.exe", "serve"]) is False


def test_dispatch_cli_for_config_then_serve() -> None:
    pe = _load_pyi_entry()
    assert pe._argv_dispatches_to_windows_service(["inferra.exe", "--config", "x.toml", "serve"]) is False


@pytest.mark.skipif(os.name != "nt", reason="frozen SCM dispatch is Windows-only")
def test_main_frozen_single_argv_tries_service_host(monkeypatch: pytest.MonkeyPatch) -> None:
    import windows_service as ws

    pe = _load_pyi_entry()
    called: list[bool] = []

    def fake_try() -> bool:
        called.append(True)
        return True

    monkeypatch.setattr(ws, "try_run_frozen_windows_service", fake_try)
    monkeypatch.setattr(sys, "argv", [r"C:\dist\inferra.exe"])
    monkeypatch.setattr(sys, "frozen", True, raising=False)

    assert pe.main() == 0
    assert called == [True]
