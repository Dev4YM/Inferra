from __future__ import annotations

import hashlib
import re

from core.enums import Severity


_TIMESTAMP_RE = re.compile(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?")
_UUID_RE = re.compile(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b", re.I)
_IPV4_RE = re.compile(r"\b\d{1,3}(?:\.\d{1,3}){3}(?::\d+)?\b")
_EMAIL_RE = re.compile(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
_WINDOWS_PATH_RE = re.compile(r"\b[A-Za-z]:\\(?:[^<>:\"|?*\r\n\\]+\\)*[^<>:\"|?*\r\n\\]*")
_UNIX_PATH_RE = re.compile(r"(?<!\w)/(?:[^\s/]+/)*[^\s/]+")
_HEX_RE = re.compile(r"\b0x[0-9a-f]+\b|\b[0-9a-f]{10,}\b", re.I)
_NUMBER_RE = re.compile(r"\b\d+(?:\.\d+)?\b")


def extract_template(message: str) -> str:
    result = " ".join(message.strip().split()).lower()
    result = _TIMESTAMP_RE.sub("{TS}", result)
    result = _UUID_RE.sub("{UUID}", result)
    result = _IPV4_RE.sub("{IP}", result)
    result = _EMAIL_RE.sub("{EMAIL}", result)
    result = _WINDOWS_PATH_RE.sub("{PATH}", result)
    result = _UNIX_PATH_RE.sub("{PATH}", result)
    result = _HEX_RE.sub("{HEX}", result)
    result = _NUMBER_RE.sub("{N}", result)
    return result


def compute_fingerprint(service_id: str, message: str, severity: Severity | int, length: int = 32) -> str:
    severity_value = int(severity)
    template = extract_template(message)
    payload = f"{template}|{severity_value}|{service_id.strip().lower()}".encode("utf-8")
    digest = hashlib.sha256(payload).digest()
    return digest[:length].hex()

def templatize_message(message: str) -> str:
    return extract_template(message)
