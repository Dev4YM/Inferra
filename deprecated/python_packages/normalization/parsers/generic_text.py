from __future__ import annotations

import re

from core.time import parse_datetime
from normalization.parsers.base import ParserResult, infer_severity, make_result


_ISO_RE = re.compile(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?")


def parse(text: str) -> ParserResult:
    stripped = text.strip()
    if not stripped:
        return make_result(
            parser_name="generic_text",
            timestamp=None,
            severity=None,
            message="<empty>",
            structured={},
            parse_confidence=0.15,
            flags={"empty_payload", "unrecognized_format"},
        )

    timestamp_match = _ISO_RE.search(stripped)
    timestamp = parse_datetime(timestamp_match.group(0)) if timestamp_match else None
    severity = infer_severity(stripped)
    return make_result(
        parser_name="generic_text",
        timestamp=timestamp,
        severity=severity,
        message=stripped,
        structured={"text_length": len(stripped)},
        parse_confidence=0.15,
        flags={"unrecognized_format"},
    )
