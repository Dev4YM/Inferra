from __future__ import annotations

from pathlib import Path

from ai.redaction import sanitize_plaintext


def test_seeded_secrets_fixture_redacts_high_fraction() -> None:
    path = Path(__file__).resolve().parent.parent / "fixtures" / "secrets" / "seeded_lines.txt"
    raw = path.read_text(encoding="utf-8")
    forbidden = [
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
        "supersecretvalue",
        "192.168.12.34",
        "fe80::1ff:fe23:4567:890a",
        "ProgramData",
        "/home/deploy/app/config",
        "hunter2",
        "ghp_abcdefghijklmnopqrstuvwxyz12",
        "stolen_session_value",
        "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        "AKIAIOSFODNN7EXAMPLE",
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        "statictoken",
    ]
    sensitive_hits = sum(1 for token in forbidden if token in raw)
    assert sensitive_hits == len(forbidden)
    redacted_parts = [sanitize_plaintext(line)[0] for line in raw.splitlines()]
    blob = "\n".join(redacted_parts)
    remaining = sum(1 for token in forbidden if token in blob)
    assert remaining == 0
    assert "api-gateway" in blob
