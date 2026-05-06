from __future__ import annotations

import json
import math
import re
import socket
from datetime import timedelta
from pathlib import Path
from typing import Any

from config.model import NormalizationConfig
from core.enums import EventType
from core.ids import new_id
from core.logging import get_logger
from core.time import ensure_utc
from events.models import DataQuality, NormalizedEvent, RawEvent, SourceRef, thaw_value
from normalization.fingerprint import compute_fingerprint
from normalization.parsers import (
    ParserResult,
    docker_json,
    generic_text,
    json_line,
    k8s_event,
    syslog_rfc3164,
    syslog_rfc5424,
    windows_eventlog,
)
from normalization.parsers.base import derive_tags, infer_severity

_log = get_logger(__name__)

_RFC5424_PREFIX_RE = re.compile(r"^<\d{1,3}>1\s")
_RFC3164_PREFIX_RE = re.compile(r"^(?:<\d{1,3}>)?[A-Z][a-z]{2}\s+\d{1,2}\s+\d{2}:\d{2}:\d{2}\s+")
_QUALITY_WEIGHTS = {
    "parse": 0.35,
    "timestamp": 0.25,
    "identity": 0.20,
    "completeness": 0.20,
}


class NormalizationPipeline:
    def __init__(self, config: NormalizationConfig | None = None) -> None:
        self.config = config or NormalizationConfig()

    def normalize(self, raw: RawEvent) -> NormalizedEvent:
        parsed = self._parse(raw)
        timestamp, timestamp_source, timestamp_confidence, timestamp_flags = self._resolve_timestamp(raw, parsed)
        message, message_flags = self._sanitize_message(parsed.message)
        service_id, identity_confidence = self._resolve_service_id(raw, parsed)
        host_id = self._resolve_host_id(raw, parsed)
        structured_data = self._enrich_structured_data(raw, parsed, service_id=service_id, host_id=host_id)
        structured_data, structured_flags = self._enforce_structured_data_budget(structured_data)
        tags = self._derive_tags(message, structured_data, parsed.tags)
        severity = parsed.severity or infer_severity(message)
        source_ref = SourceRef(
            source_type=raw.source_type,
            source_id=raw.source_id,
            raw_offset=raw.metadata.get("raw_offset") or raw.metadata.get("line_number"),
            collected_at=ensure_utc(raw.collected_at),
        )
        quality_flags = parsed.quality.flags | timestamp_flags | message_flags | structured_flags
        quality = self._quality(
            parse_confidence=parsed.quality.parse_confidence,
            timestamp_confidence=timestamp_confidence,
            identity_confidence=identity_confidence,
            completeness=parsed.quality.completeness,
            flags=quality_flags,
        )
        fingerprint = compute_fingerprint(
            service_id=service_id,
            message=message,
            severity=severity,
            length=self.config.fingerprint_length,
        )
        return NormalizedEvent(
            event_id=new_id("evt"),
            timestamp=timestamp,
            timestamp_source=timestamp_source,
            service_id=service_id,
            host_id=host_id,
            severity=severity,
            event_type=self._classify_event_type(raw, message, tags),
            message=message,
            structured_data=structured_data,
            tags=tags,
            fingerprint=fingerprint,
            quality=quality,
            source_ref=source_ref,
            schema_version=1,
        )

    def _parse(self, raw: RawEvent) -> ParserResult:
        text = raw.raw_payload if isinstance(raw.raw_payload, str) else str(raw.raw_payload)
        try:
            return self._parse_known_shapes(raw, text)
        except (TypeError, ValueError, json.JSONDecodeError) as exc:
            _log.warning(
                "normalization parser fallback",
                extra={"source_type": raw.source_type, "source_id": raw.source_id, "error": str(exc)},
            )
            return generic_text.parse(text)

    def _parse_known_shapes(self, raw: RawEvent, text: str) -> ParserResult:
        stripped = text.strip()
        if not stripped:
            return generic_text.parse(stripped)

        if stripped[0] in "{[":
            value = json.loads(stripped)
            if isinstance(value, dict):
                if raw.source_type == "windows_eventlog" or "windows_eventlog" in value:
                    return windows_eventlog.parse(value)
                if raw.source_type == "kubernetes" or "kubernetes" in value:
                    return k8s_event.parse(value)
                if raw.source_type == "linux_syslog":
                    syslog_raw = str((value.get("syslog") or {}).get("raw") or "")
                    if syslog_raw:
                        return self._parse_syslog_text(syslog_raw, raw)
                if raw.source_type == "docker" or self._looks_like_docker_json(value):
                    return docker_json.parse(value)
                return json_line.parse(stripped, value)

        if raw.source_type == "linux_syslog":
            return self._parse_syslog_text(stripped, raw)
        if self._looks_like_rfc5424(stripped):
            return syslog_rfc5424.parse(stripped)
        if self._looks_like_rfc3164(stripped):
            return syslog_rfc3164.parse(stripped, ensure_utc(raw.collected_at))
        return generic_text.parse(stripped)

    def _parse_syslog_text(self, text: str, raw: RawEvent) -> ParserResult:
        if self._looks_like_rfc5424(text):
            return syslog_rfc5424.parse(text)
        return syslog_rfc3164.parse(text, ensure_utc(raw.collected_at))

    def _resolve_timestamp(self, raw: RawEvent, parsed: ParserResult) -> tuple[Any, str, float, frozenset[str]]:
        now = ensure_utc(raw.collected_at)
        if parsed.timestamp is None:
            return now, "collected_at", 0.55, frozenset({"timestamp_missing"})

        candidate = ensure_utc(parsed.timestamp)
        skew_upper = now + timedelta(hours=1)
        tolerance_upper = now + timedelta(seconds=self.config.timestamp_future_tolerance_seconds)
        lower_bound = now - timedelta(days=30)
        if candidate > skew_upper:
            return now, "collected_at", 0.12, frozenset({"clock_skew_future", "timestamp_in_future"})
        if candidate > tolerance_upper:
            return now, "collected_at", 0.15, frozenset({"timestamp_in_future"})
        if candidate < lower_bound:
            return now, "collected_at", 0.15, frozenset({"timestamp_too_old"})
        return candidate, "parsed", 1.0, frozenset()

    def _resolve_service_id(self, raw: RawEvent, parsed: ParserResult) -> tuple[str, float]:
        config_match = self._service_mapping_match(raw, parsed)
        if config_match is not None:
            return config_match, 1.0

        direct_candidates = [
            parsed.structured.get("service"),
            parsed.structured.get("service_id"),
            parsed.structured.get("app"),
            parsed.structured.get("app_name"),
            parsed.structured.get("logger"),
            parsed.structured.get("program"),
        ]
        for candidate in direct_candidates:
            if candidate:
                return self._canonical_service(str(candidate)), 0.95

        if raw.source_type == "docker":
            attrs = parsed.structured.get("attrs")
            if isinstance(attrs, dict):
                service = attrs.get("com.docker.compose.service") or attrs.get("service")
                if service:
                    return self._canonical_service(str(service)), 0.9
            name = raw.metadata.get("container_name") or raw.metadata.get("name")
            if name:
                return self._canonical_service(str(name)), 0.8

        if raw.source_type == "windows_service":
            service_name = raw.metadata.get("service_name")
            if service_name:
                return self._canonical_service(str(service_name)), 1.0

        if raw.source_type == "windows_eventlog":
            provider = raw.metadata.get("provider") or raw.metadata.get("source") or parsed.structured.get("service")
            if provider:
                return self._canonical_service(str(provider)), 0.85

        if raw.source_type in {"linux_journald", "linux_syslog"}:
            service_name = (
                raw.metadata.get("unit")
                or raw.metadata.get("identifier")
                or raw.metadata.get("program")
                or parsed.structured.get("program")
                or parsed.structured.get("app")
            )
            if service_name:
                return self._canonical_service(str(service_name)), 0.88

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
                return self._canonical_service(Path(str(path)).stem), 0.65

        return self._canonical_service(f"unknown-{raw.source_type}"), 0.2

    def _resolve_host_id(self, raw: RawEvent, parsed: ParserResult) -> str:
        configured = getattr(self.config, "host_id", None)
        if configured:
            return self._canonical_host(str(configured))

        for key in ("host", "hostname", "host_id"):
            value = parsed.structured.get(key) or raw.metadata.get(key)
            if value:
                return self._canonical_host(str(value))

        for key in ("computer_name", "node"):
            value = raw.metadata.get(key)
            if value:
                return self._canonical_host(str(value))

        if raw.metadata.get("container_id"):
            return self._canonical_host(str(raw.metadata["container_id"])[:12])
        return self._canonical_host(socket.gethostname())

    def _enrich_structured_data(
        self,
        raw: RawEvent,
        parsed: ParserResult,
        *,
        service_id: str,
        host_id: str,
    ) -> dict[str, Any]:
        enriched = dict(thaw_value(parsed.structured))
        enriched.setdefault("service_id", service_id)
        enriched.setdefault("host_id", host_id)

        process_context: dict[str, Any] = {}
        pid = raw.metadata.get("pid") or self._nested_get(enriched, "process", "pid") or enriched.get("pid")
        comm = (
            raw.metadata.get("comm")
            or raw.metadata.get("process_name")
            or self._nested_get(enriched, "process", "name")
            or enriched.get("program")
            or enriched.get("app")
        )
        if pid is not None:
            process_context["pid"] = pid
        if comm:
            process_context["comm"] = str(comm)
        if process_context:
            enriched["process_context"] = process_context

        return enriched

    def _enforce_structured_data_budget(self, structured_data: dict[str, Any]) -> tuple[dict[str, Any], frozenset[str]]:
        budget = self.config.max_structured_data_bytes
        payload = self._structured_data_bytes(structured_data)
        if payload <= budget:
            return structured_data, frozenset()
        return {"keys": sorted(structured_data.keys())}, frozenset({"structured_payload_dropped"})

    def _derive_tags(self, message: str, structured_data: dict[str, Any], parser_tags: frozenset[str]) -> frozenset[str]:
        tags = set(parser_tags) | set(derive_tags(message, structured_data))
        haystack = f"{message} {json.dumps(structured_data, default=str, sort_keys=True)}".lower()
        for rule in self.config.tag_rules:
            if rule.pattern and rule.tags and re.search(rule.pattern, haystack, re.IGNORECASE):
                tags.update(rule.tags)
        return frozenset(sorted(tags))

    def _classify_event_type(self, raw: RawEvent, message: str, tags: frozenset[str]) -> EventType:
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

    def _sanitize_message(self, message: str) -> tuple[str, frozenset[str]]:
        text = " ".join(message.strip().split()) or "<empty>"
        if len(text) <= self.config.max_message_length:
            return text, frozenset()
        max_length = self.config.max_message_length
        if max_length <= 3:
            return text[:max_length], frozenset({"truncated"})
        return text[: max_length - 3] + "...", frozenset({"truncated"})

    def _quality(
        self,
        *,
        parse_confidence: float,
        timestamp_confidence: float,
        identity_confidence: float,
        completeness: float,
        flags: frozenset[str],
    ) -> DataQuality:
        overall = math.prod(
            (
                max(parse_confidence, 0.0) ** _QUALITY_WEIGHTS["parse"],
                max(timestamp_confidence, 0.0) ** _QUALITY_WEIGHTS["timestamp"],
                max(identity_confidence, 0.0) ** _QUALITY_WEIGHTS["identity"],
                max(completeness, 0.0) ** _QUALITY_WEIGHTS["completeness"],
            )
        )
        return DataQuality(
            overall=round(max(0.0, min(1.0, overall)), 4),
            timestamp_confidence=round(timestamp_confidence, 4),
            parse_confidence=round(parse_confidence, 4),
            identity_confidence=round(identity_confidence, 4),
            completeness=round(completeness, 4),
            flags=flags,
        )

    def _service_mapping_match(self, raw: RawEvent, parsed: ParserResult) -> str | None:
        candidates = {
            str(raw.source_id),
            str(raw.metadata.get("path") or ""),
            str(raw.metadata.get("unit") or ""),
            str(raw.metadata.get("container_name") or ""),
            str(parsed.structured.get("service") or ""),
            str(parsed.structured.get("app") or ""),
        }
        for mapping in self.config.service_mappings:
            if not mapping.pattern or not mapping.service_id:
                continue
            if any(self._pattern_matches(mapping.pattern, candidate) for candidate in candidates if candidate):
                return self._canonical_service(mapping.service_id)
        return None

    def _pattern_matches(self, pattern: str, candidate: str) -> bool:
        try:
            return re.search(pattern, candidate, re.IGNORECASE) is not None
        except re.error:
            return pattern.lower() in candidate.lower()

    def _canonical_service(self, value: str) -> str:
        lowered = value.strip().lower()
        lowered = lowered.replace("\\", "/").rsplit("/", 1)[-1]
        lowered = lowered.removesuffix(".service")
        lowered = re.sub(r"(_\d+|-replica-\d+|-\d+)$", "", lowered)
        lowered = re.sub(r"[^a-z0-9\-_.]+", "-", lowered).strip("-")
        return lowered[:128] or "unknown"

    def _canonical_host(self, value: str) -> str:
        lowered = value.strip().lower()
        lowered = re.sub(r"[^a-z0-9\-_.]+", "-", lowered).strip("-")
        return lowered[:256] or "unknown"

    def _looks_like_docker_json(self, value: dict[str, Any]) -> bool:
        return "log" in value and ("time" in value or "stream" in value or "attrs" in value)

    def _looks_like_rfc3164(self, text: str) -> bool:
        return _RFC3164_PREFIX_RE.match(text) is not None

    def _looks_like_rfc5424(self, text: str) -> bool:
        return _RFC5424_PREFIX_RE.match(text) is not None

    def _structured_data_bytes(self, structured_data: dict[str, Any]) -> int:
        return len(json.dumps(structured_data, default=str, sort_keys=True).encode("utf-8"))

    def _nested_get(self, payload: dict[str, Any], *path: str) -> Any:
        current: Any = payload
        for part in path:
            if not isinstance(current, dict) or part not in current:
                return None
            current = current[part]
        return current
