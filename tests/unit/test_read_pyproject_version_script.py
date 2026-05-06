from __future__ import annotations

import subprocess
import sys
import tomllib
from pathlib import Path


def test_read_pyproject_version_script_matches_pyproject() -> None:
    root = Path(__file__).resolve().parents[2]
    script = root / "deploy" / "windows" / "read_pyproject_version.py"
    proc = subprocess.run(
        [sys.executable, str(script), str(root)],
        check=True,
        capture_output=True,
        text=True,
    )
    expected = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))["project"]["version"]
    assert proc.stdout.strip() == str(expected)
