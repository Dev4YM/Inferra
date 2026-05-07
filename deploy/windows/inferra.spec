# -*- mode: python ; coding: utf-8 -*-
# Deprecated single-file inferra.exe path retained for migration fallback only.
#
# Production builds MUST prefer deploy/windows/build-rust-exe.ps1. This legacy
# spec survives only so the archived PyInstaller path remains reproducible.
# The old Python-first wrapper now lives under deprecated/windows-pyinstaller/.
# Historical builds can still use deploy/windows/build-exe.ps1 (compat shim) or
# Import-Module InferraWindows.psm1;
# Invoke-InferraWindowsExeBuild): PyInstaller is invoked with an isolated --distpath staging folder
# and artifacts are promoted with retries. Direct "python -m PyInstaller ... inferra.spec" writes
# to ./dist by default and can hit PermissionError if inferra.exe is loaded by the SCM.
#
# Operator guide: docs/operations/windows_exe_build.md

from pathlib import Path

_spec_dir = Path(SPEC).resolve().parent
_REPO_ROOT = _spec_dir.parent.parent
_SRC = _REPO_ROOT / "src"
_ENTRY = _REPO_ROOT / "deprecated" / "windows-pyinstaller" / "pyi_entry.py"

a = Analysis(
    [str(_ENTRY)],
    pathex=[str(_SRC), str(_REPO_ROOT / "deprecated")],
    binaries=[],
    datas=[
        (str(_REPO_ROOT / "pyproject.toml"), "."),
        (str(_SRC / "config" / "defaults.toml"), "config"),
        # UI is the Vite bundle under ui_dist (see web.frontend_assets); legacy src/web/static was removed.
        (str(_SRC / "web" / "ui_dist"), "web/ui_dist"),
    ],
    hiddenimports=[
        "win32timezone",
        "uvicorn.logging",
        "uvicorn.loops",
        "uvicorn.loops.auto",
        "uvicorn.protocols",
        "uvicorn.protocols.http",
        "uvicorn.protocols.http.auto",
        "uvicorn.protocols.websockets",
        "uvicorn.protocols.websockets.auto",
        "uvicorn.lifespan",
        "uvicorn.lifespan.on",
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.zipfiles,
    a.datas,
    [],
    name="inferra",
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=False,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)
