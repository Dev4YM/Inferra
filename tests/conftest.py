from __future__ import annotations

import sys
from pathlib import Path

import pytest

_REPO_ROOT = Path(__file__).resolve().parents[1]
_LEGACY_PY_PACKAGES = _REPO_ROOT / "deprecated" / "python_packages"
if str(_LEGACY_PY_PACKAGES) not in sys.path:
    sys.path.insert(0, str(_LEGACY_PY_PACKAGES))


@pytest.fixture(autouse=True)
def _clear_argcomplete_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("COMP_LINE", raising=False)
    monkeypatch.delenv("COMP_POINT", raising=False)
