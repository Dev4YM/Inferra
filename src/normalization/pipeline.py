from __future__ import annotations

import json
import re
import socket
from dataclasses import dataclass
from datetime import UTC, datetime
from typing import Any

from config.model import NormalizationConfig
from core.enums import EventType, Severity
from core.ids import new_id
from core.time import ensure_utc, parse_datetime, utc_now
from events.models import DataQuality, NormalizedEvent, RawEvent, SourceRef
from normalization.fingerprint import compute_fingerprint


@dataclass(frozen=True)
class ParseResult:
    timestamp: datetime | None
    severity: Severity | None
    message: str
    structured_data: dict[str, Any]
    parse_format: str
    parse_confidence: float
    severity_explicit: bool = False


class NormalizationPipeline:
    def __init__(self, config: NormalizationConfig | None = None) -> None:
        self.config = config or NormalizationConfig()

    def normalize(self, raw: RawEvent) -> NormalizedEvent:
        parsed = self._parse(raw.raw_payload)
        timestamp_source = "parsed" if parsed.timestamp else "collected_at"
        timestamp = ensure_utc(parsed.timestamp or raw.collected_at or utc_now())
        severity = parsed.severity or self._infer_severity(parsed.message)
        service_id, identity_confidence = self._resolve_service_id(raw, parsed)
        host_id = self._resolve_host_id(raw, parsed)
        tags = self._derive_tags(parsed.message, parsed.structured_data)
        event_type = self._classify_event_type(raw, parsed.message, tags)
        message = self._sanitize_message(parsed.message)
        source_ref = SourceRef(
            source_type=raw.source_type,
            source_id=raw.source_id,
            raw_offset=raw.metadata.get("raw_offset") or raw.metadata.get("line_number"),
            collected_at=ensure_utc(raw.collected_at),
        )
        quality = self._quality(timestamp_source, parsed, identity_confidence)
        fingerprint = compute_fingerprint(service_id, message, int(severity), self.config.fingerprint_length)
        return NormalizedEvent(
            event_id=new_id("evt"),
            timestamp=timestamp,
            timestamp_source=timestamp_source,
            service_id=service_id,
            host_id=host_id,
            severity=severity,
            event_type=event_type,
            message=message,
            structured_data=parsed.structured_data,
            tags=frozenset(tags),
            fingerprint=fingerprint,
            quality=quality,
            source_ref=source_ref,
            schema_version=1,
        )

    def _parse(self, payload: str) -> ParseResult:
        text = payload.strip()
        if not text:
            return ParseResult(None, Severity.INFO, "<empty>", {"_raw_length": 0, "_parse_method": "empty"}, "empty", 0.3)
        if text[0] in "{[":
            try:
                value = json.loads(text)
                if isinstance(value, dict):
                    return self._parse_json(value)
            except json.JSONDecodeError:
                pass
        kv = self._parse_kv(text)
        if kv is not None:
            return kv
        return self._parse_freetext(text)

    def _parse_json(self, data: dict[str, Any]) -> ParseResult:
        timestamp = None
        for key in ("timestamp", "time", "ts", "@timestamp", "date"):
            if key in data:
                timestamp = parse_datetime(str(data[key]))
                if timestamp:
                    break
        severity = None
        severity_explicit = False
        for key in ("level", "severity", "loglevel", "priority"):
            if key in data:
                severity = self._severity_from_value(data[key])
                severity_explicit = severity is not None
                break
        message = ""
        for key in ("message", "msg", "text", "log"):
            if key in data:
                message = str(data[key])
                break
        if not message:
            message = json.dumps(data, sort_keys=True)
        structured = {k: v for k, v in data.items() if k not in {"message", "msg", "text", "log"}}
        structured["_parse_method"] = "json"
        structured["_raw_length"] = len(json.dumps(data))
        return ParseResult(timestamp, severity, message, structured, "json", 1.0, severity_explicit)

    def _parse_kv(self, text: str) -> ParseResult | None:
        pairs = dict(re.findall(r"(\w+)=(\"[^\"]*\"|'[^']*'|\S+)", text))
        if len(pairs) < 3:
            return None
        clean = {k: v.strip("\"'") for k, v in pairs.items()}
        timestamp = None
        for key in ("timestamp", "time", "ts", "date"):
            if key in clean:
                timestamp = parse_datetime(clean[key])
                if timestamp:
                    break
        severity = None
        severity_explicit = False
        for key in ("level", "severity", "priority"):
            if key in clean:
                severity = self._severity_from_value(clean[key])
                severity_explicit = severity is not None
                break
        message = clean.get("message") or clean.get("msg") or text
        clean["_parse_method"] = "kv"
        clean["_raw_length"] = len(text)
        return ParseResult(timestamp, severity, message, clean, "kv", 0.8, severity_explicit)

    def _parse_freetext(self, text: str) -> ParseResult:
        timestamp = None
        match = re.search(r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?", text)
        if match:
            timestamp = parse_datetime(match.group(0))
        severity = self._infer_severity(text)
        structured = {"_parse_method": "freetext", "_raw_length": len(text)}
        return ParseResult(timestamp, severity, text, structured, "freetext", 0.3, False)

    def _severity_from_value(self, value: Any) -> Severity | None:
        if isinstance(value, int):
            if value >= 40:
                return Severity.CRITICAL
            if value >= 30:
                return Severity.ERROR
            if value >= 20:
                return Severity.WARN
            if value >= 10:
                return Severity.INFO
            return Severity.DEBUG
        text = str(value).lower()
        if text in {"debug", "trace"}:
            return Severity.DEBUG
        if text in {"info", "notice"}:
            return Severity.INFO
        if text in {"warn", "warning"}:
            return Severity.WARN
        if text in {"error", "err"}:
            return Severity.ERROR
        if text in {"fatal", "panic", "critical", "crit", "emerg", "alert"}:
            return Severity.CRITICAL
        return None

    def _infer_severity(self, message: str) -> Severity:
        lower = message.lower()
        if any(token in lower for token in ("fatal", "panic", "oom", "out of memory", "killed", "critical")):
            return Severity.CRITICAL
        if any(token in lower for token in ("error", "fail", "exception", "traceback", "segfault")):
            return Severity.ERROR
        if "warn" in lower:
            return Severity.WARN
        if "debug" in lower:
            return Severity.DEBUG
        return Severity.INFO

    def _resolve_service_id(self, raw: RawEvent, parsed: ParseResult) -> tuple[str, float]:
        for key in ("service", "service_id", "app", "app_name", "logger"):
            value = parsed.structured_data.get(key)
            if value:
                return self._canonical_service(str(value)), 1.0
        if raw.source_type == "docker":
            name = raw.metadata.get("container_name") or raw.metadata.get("name")
            if name:
                return self._canonical_service(str(name)), 0.7
        if raw.source_type == "windows_eventlog":
            provider = raw.metadata.get("provider") or raw.metadata.get("source")
            if provider:
                return self._canonical_service(str(provider)), 0.6
        if raw.source_type == "windows_service":
            service_name = raw.metadata.get("service_name")
            if service_name:
                return self._canonical_service(str(service_name)), 1.0
        if raw.source_type in {"linux_journald", "linux_syslog"}:
            service_name = raw.metadata.get("unit") or raw.metadata.get("identifier") or raw.metadata.get("program")
            if service_name:
                return self._canonical_service(str(service_name)), 0.9
        if raw.source_type == "kubernetes":
            workload = raw.metadata.get("workload") or raw.metadata.get("pod")
            if workload:
                return self._canonical_service(str(workload)), 0.9
        if raw.source_type == "file":
            service_id = raw.metadata.get("service_id")
            if service_id:
                return self._canonical_service(str(service_id)), 1.0
            path = raw.metadata.get("path")
            if path:
                name = str(path).replace("\\", "/").rsplit("/", 1)[-1].split(".", 1)[0]
                return self._canonical_service(name), 0.4
        return f"unknown-{raw.source_type}", 0.2

    def _resolve_host_id(self, raw: RawEvent, parsed: ParseResult) -> str:
        for key in ("host", "hostname", "host_id"):
            value = parsed.structured_data.get(key) or raw.metadata.get(key)
            if value:
                return str(value)[:256]
        if "container_id" in raw.metadata:
            return str(raw.metadata["container_id"])[:12]
        return socket.gethostname()[:256]

    def _canonical_service(self, value: str) -> str:
        lowered = value.strip().lower()
        lowered = re.sub(r"(_\d+|-replica-\d+|-\d+)$", "", lowered)
        lowered = re.sub(r"[^a-z0-9\-_.]+", "-", lowered).strip("-")
        return lowered[:128] or "unknown"

    def _derive_tags(self, message: str, structured_data: dict[str, Any]) -> set[str]:
        haystack = f"{message} {json.dumps(structured_data, default=str)}".lower()
        rules = {
            "oom": ("out of memory", "oom", "cannot allocate", "memory exhausted"),
            "resource_pressure": ("resource pressure", "high memory", "high cpu", "disk_percent"),
            "restart": ("restarting", "restart", "started container", "process started"),
            "connection_refused": ("connection refused", "econnrefused", "connect: connection refused"),
            "timeout": ("timed out", "timeout", "deadline exceeded", "context deadline"),
            "crash": ("segfault", "panic", "fatal", "core dumped", "unhandled exception"),
            "disk_full": ("no space left", "disk full", "enospc"),
            "permission_denied": ("permission denied", "access denied", "eacces", "forbidden"),
            "dns_failure": ("nxdomain", "name resolution", "getaddrinfo", "dns failure"),
            "certificate_error": ("certificate", "ssl", "tls", "x509"),
            "rate_limited": ("rate limit", "too many requests", "429"),
            "config_change": ("configuration", "config reload", "settings changed"),
            "deployment": ("deploy", "release", "rolling update", "image pulled"),
            "kubernetes": ("kubernetes", "pod", "crashloopbackoff", "imagepullbackoff", "evicted"),
        }
        return {tag for tag, patterns in rules.items() if any(pattern in haystack for pattern in patterns)}

    def _classify_event_type(self, raw: RawEvent, message: str, tags: set[str]) -> EventType:
        if raw.source_type in {"procfs", "host_metrics", "process_snapshot"}:
            return EventType.METRIC
        if raw.source_type == "windows_service":
            return EventType.STATE_CHANGE
        lower = message.lower()
        if tags & {"restart", "deployment", "config_change"}:
            return EventType.STATE_CHANGE
        if "health" in lower and any(token in lower for token in ("pass", "fail", "healthy", "unhealthy")):
            return EventType.HEALTH_CHECK
        return EventType.LOG

    def _sanitize_message(self, message: str) -> str:
        text = " ".join(message.strip().split())
        if not text:
            text = "<empty>"
        if len(text) > self.config.max_message_length:
            return text[: self.config.max_message_length - 3] + "..."
        return text

    def _quality(self, timestamp_source: str, parsed: ParseResult, identity_confidence: float) -> DataQuality:
        ts_conf = 1.0 if timestamp_source == "parsed" else 0.5
        populated = sum(
            [
                bool(parsed.message),
                parsed.severity is not None,
                bool(parsed.structured_data),
                parsed.parse_format != "empty",
            ]
        ) / 4.0
        overall = 0.3 * ts_conf + 0.3 * parsed.parse_confidence + 0.2 * identity_confidence + 0.2 * populated
        return DataQuality(
            overall=round(max(0.0, min(1.0, overall)), 4),
            timestamp_confidence=round(ts_conf, 4),
            parse_confidence=round(parsed.parse_confidence, 4),
            identity_confidence=round(identity_confidence, 4),
            completeness=round(populated, 4),
        )
