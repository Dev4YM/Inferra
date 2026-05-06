from __future__ import annotations

import pytest


@pytest.fixture(autouse=True)
def _clear_argcomplete_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("COMP_LINE", raising=False)
    monkeypatch.delenv("COMP_POINT", raising=False)
