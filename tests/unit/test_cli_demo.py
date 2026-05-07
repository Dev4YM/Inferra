"""Smoke tests for `inferra demo` CLI commands."""

from __future__ import annotations

import json

from inferra_legacy.cli import main


def _config_path(tmp_path):
    config_path = tmp_path / "inferra.toml"
    config_path.write_text(
        f"""
[storage]
data_dir = "{(tmp_path / 'state').as_posix()}"
""".strip()
        + "\n",
        encoding="utf-8",
    )
    return config_path


def _extract_last_json(out: str) -> dict:
    """Parse the trailing JSON object from CLI stdout (logs may precede it)."""
    decoder = json.JSONDecoder()
    text = out.rstrip()
    last: dict | None = None
    idx = 0
    while idx < len(text):
        try:
            obj, end = decoder.raw_decode(text, idx)
        except json.JSONDecodeError:
            idx = text.find("{", idx + 1)
            if idx == -1:
                break
            continue
        if isinstance(obj, dict):
            last = obj
        idx = end
        while idx < len(text) and text[idx] in " \r\n\t":
            idx += 1
    if last is None:
        raise AssertionError(f"no JSON object found in: {out!r}")
    return last


def test_demo_seed_writes_events(tmp_path, capsys):
    config_path = _config_path(tmp_path)
    rc = main(["--json", "--config", str(config_path), "demo", "seed", "--count", "3"])
    assert rc == 0
    payload = _extract_last_json(capsys.readouterr().out)
    assert payload["events_written"] == 3
    assert payload["service_id"] == "api"
    assert payload["events_db"].endswith("events.db")


def test_demo_clear_runs_after_seed(tmp_path, capsys):
    config_path = _config_path(tmp_path)
    rc = main(["--json", "--config", str(config_path), "demo", "seed", "--count", "2"])
    assert rc == 0
    capsys.readouterr()
    rc = main(["--json", "--config", str(config_path), "demo", "clear"])
    assert rc == 0
    payload = _extract_last_json(capsys.readouterr().out)
    assert payload["command"] == "demo clear"
