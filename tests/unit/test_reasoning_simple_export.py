from __future__ import annotations

import reasoning.simple as rs


def test_simple_reexports_hypothesis_engine() -> None:
    assert hasattr(rs, "HypothesisEngine")
    assert hasattr(rs, "SimpleHypothesisEngine")
