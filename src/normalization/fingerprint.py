from __future__ import annotations

import hashlib
import re


def templatize_message(message: str) -> str:
    result = message
    result = re.sub(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}[\.\d]*(?:Z|[+-]\d{2}:\d{2})?", "<TS>", result)
    result = re.sub(r"\b\d{1,3}(?:\.\d{1,3}){3}(?::\d+)?\b", "<IP>", result)
    result = re.sub(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", "<UUID>", result, flags=re.I)
    result = re.sub(r"\b[0-9a-f]{8,}\b", "<HEX>", result, flags=re.I)
    result = re.sub(r'"[^"]*"', "<STR>", result)
    result = re.sub(r"'[^']*'", "<STR>", result)
    result = re.sub(r"(?:[A-Za-z]:)?[\\/][\w\- .\\/]+", "<PATH>", result)
    result = re.sub(r"\b\d+\.?\d*\b", "<NUM>", result)
    return " ".join(result.split()).lower()


def compute_fingerprint(service_id: str, message: str, severity_value: int, length: int = 32) -> str:
    template = templatize_message(message)
    raw = f"{service_id}|{template}|{severity_value}"
    return hashlib.sha256(raw.encode("utf-8")).hexdigest()[:length]
