from ai.redaction import SECRET_REPLACEMENT, redact_text, redact_value, sanitize_plaintext


def test_redact_text_masks_common_secret_shapes():
    text = "Authorization: Bearer abc123 token=mytoken password=hunter2"

    redacted = redact_text(text)

    assert "abc123" not in redacted
    assert "mytoken" not in redacted
    assert "hunter2" not in redacted
    assert SECRET_REPLACEMENT in redacted


def test_redact_value_masks_secret_keys_recursively():
    payload = {"context": {"api_key": "abc", "nested": [{"cookie": "session=1"}]}, "safe": "visible"}

    redacted = redact_value(payload)

    assert redacted["context"]["api_key"] == SECRET_REPLACEMENT
    assert redacted["context"]["nested"][0]["cookie"] == SECRET_REPLACEMENT
    assert redacted["safe"] == "visible"


def test_sanitize_plaintext_returns_report_entries() -> None:
    text = r"host 10.0.0.1 at C:\secret\app.log"
    redacted, report = sanitize_plaintext(text)
    assert "[IP]" in redacted
    assert "[PATH]" in redacted
    assert report.removals
