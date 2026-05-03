from __future__ import annotations

from dataclasses import replace

from core.enums import Severity
from events.models import NormalizedEvent


FAILURE_TAGS = {
    "oom",
    "crash",
    "restart",
    "connection_refused",
    "timeout",
    "disk_full",
    "permission_denied",
    "dns_failure",
    "certificate_error",
}


class NoiseFilter:
    def should_store(self, event: NormalizedEvent) -> bool:
        if event.severity >= Severity.ERROR:
            return True
        if event.tags & FAILURE_TAGS:
            return True
        message = event.message.lower()
        if event.severity <= Severity.INFO and ("health" in message and ("ok" in message or "200" in message or "pass" in message)):
            return False
        return True

    def annotate(self, event: NormalizedEvent) -> NormalizedEvent:
        score = self.score(event)
        structured = dict(event.structured_data)
        structured["_noise_score"] = score
        return replace(event, structured_data=structured)

    def score(self, event: NormalizedEvent) -> float:
        severity_score = {
            Severity.DEBUG: 0.1,
            Severity.INFO: 0.2,
            Severity.WARN: 0.6,
            Severity.ERROR: 1.0,
            Severity.CRITICAL: 1.0,
        }[event.severity]
        tag_score = 1.0 if event.tags & FAILURE_TAGS else 0.2
        return round(0.7 * severity_score + 0.3 * tag_score, 4)
