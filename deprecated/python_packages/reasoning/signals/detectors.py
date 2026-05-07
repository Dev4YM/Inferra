from __future__ import annotations

from collections import defaultdict
from collections.abc import Callable, Iterable
from datetime import timedelta

from core.enums import EventType, Severity

from .types import Signal, SignalContext

_DB_PATTERNS = (
    "connection pool",
    "too many connections",
    "deadlock",
    "lock timeout",
    "query timeout",
    "could not connect",
    "database is locked",
    "serialization failure",
)

_CERT_PATTERNS = ("certificate", "x509", "ssl handshake", "tls alert", "cert expired", "unknown ca")


def _msg_lower(event) -> str:
    return event.message.lower()


def _metric_pressure(event) -> bool:
    if event.event_type != EventType.METRIC:
        return False
    metrics = event.structured_data.get("metrics")
    if not isinstance(metrics, dict):
        return False
    try:
        cpu = float(metrics.get("cpu_percent", 0) or 0)
        mem = float(metrics.get("memory_percent", 0) or 0)
        disk = float(metrics.get("disk_percent", 0) or 0)
    except (TypeError, ValueError):
        return False
    return cpu >= 90.0 or mem >= 90.0 or disk >= 92.0


def detect_connection_failures_outbound(ctx: SignalContext) -> list[Signal]:
    out: list[Signal] = []
    for sid in sorted({e.service_id for e in ctx.events}):
        hits = [
            e
            for e in ctx.events
            if e.service_id == sid
            and (
                {"connection_refused", "timeout"} & e.tags
                or "connection refused" in _msg_lower(e)
                or " timed out" in _msg_lower(e)
                or "timeout" in _msg_lower(e)
            )
        ]
        if not hits:
            continue
        ids = tuple(sorted({e.event_id for e in hits}, key=str))
        conf = min(0.95, 0.55 + 0.08 * len(ids))
        out.append(
            Signal(
                name="connection_failures_outbound",
                confidence=round(conf, 4),
                evidence_event_ids=ids,
                service_id=sid,
            )
        )
    return out


def detect_error_spike(ctx: SignalContext) -> list[Signal]:
    out: list[Signal] = []
    for sid in sorted({e.service_id for e in ctx.events}):
        spike_events = [
            e
            for e in ctx.events
            if e.service_id == sid
            and (
                e.severity >= Severity.ERROR
                or (
                    e.severity >= Severity.WARN
                    and (
                        "timeout" in e.message.lower()
                        or "pressure" in e.message.lower()
                        or "fail" in e.message.lower()
                    )
                )
            )
        ]
        spike_events = sorted(spike_events, key=lambda e: (e.timestamp, e.event_id))
        if not spike_events:
            continue
        errs = [e for e in spike_events if e.severity >= Severity.ERROR]
        score = float(ctx.service_scores.get(sid, 0.0))
        if len(errs) >= 3 or (len(errs) >= 1 and score > 0.5) or (len(errs) >= 2 and score > 0.3) or (
            len(spike_events) >= 1 and score > 0.42
        ):
            ids = tuple(e.event_id for e in spike_events)
            conf = min(0.95, 0.5 + 0.1 * len(spike_events) + 0.15 * score)
            out.append(
                Signal(name="error_spike", confidence=round(conf, 4), evidence_event_ids=ids, service_id=sid)
            )
    return out


def detect_resource_pressure(ctx: SignalContext) -> list[Signal]:
    hits: list = []
    for e in ctx.events:
        if "resource_pressure" in e.tags or "oom" in e.tags or "disk_full" in e.tags:
            hits.append((e, 0.88 if "oom" in e.tags else 0.82))
        elif _metric_pressure(e):
            hits.append((e, 0.84))
        elif "resource pressure" in _msg_lower(e) or "memory pressure" in _msg_lower(e):
            hits.append((e, 0.72))
    if not hits:
        return []
    hits.sort(key=lambda pair: (pair[0].timestamp, pair[0].event_id))
    ids = tuple(sorted({pair[0].event_id for pair in hits}, key=str))
    conf = max(c for _, c in hits)
    return [Signal(name="resource_pressure", confidence=round(conf, 4), evidence_event_ids=ids, service_id=None)]


def detect_restart_pattern(ctx: SignalContext) -> list[Signal]:
    out: list[Signal] = []
    by_svc: dict[str, list] = defaultdict(list)
    for e in ctx.events:
        if {"restart", "crash"} & e.tags or "restart" in _msg_lower(e):
            by_svc[e.service_id].append(e)
    for sid in sorted(by_svc):
        evs = sorted(by_svc[sid], key=lambda e: (e.timestamp, e.event_id))
        if len(evs) >= 2:
            ids = tuple(x.event_id for x in evs)
            out.append(
                Signal(name="restart_pattern", confidence=0.8, evidence_event_ids=ids, service_id=sid),
            )
    return out


def detect_config_change_near(ctx: SignalContext) -> list[Signal]:
    cfg_events = [e for e in ctx.events if "config_change" in e.tags]
    if not cfg_events:
        return []
    cfg_events = sorted(cfg_events, key=lambda e: (e.timestamp, e.event_id))
    errors = [e for e in ctx.events if e.severity >= Severity.ERROR]
    out: list[Signal] = []
    for ce in cfg_events:
        later = [e for e in errors if e.timestamp > ce.timestamp and (e.timestamp - ce.timestamp) <= timedelta(seconds=300)]
        if later:
            ids = tuple(sorted({ce.event_id, *(e.event_id for e in later)}, key=str))
            out.append(
                Signal(
                    name="config_change_near",
                    confidence=0.62,
                    evidence_event_ids=ids,
                    service_id=ce.service_id,
                )
            )
        else:
            out.append(
                Signal(
                    name="config_change_near",
                    confidence=0.45,
                    evidence_event_ids=(ce.event_id,),
                    service_id=ce.service_id,
                )
            )
    return _dedupe_signals(out)


def detect_deployment_near(ctx: SignalContext) -> list[Signal]:
    dep_events = [e for e in ctx.events if "deployment" in e.tags]
    if not dep_events:
        return []
    dep_events = sorted(dep_events, key=lambda e: (e.timestamp, e.event_id))
    errors = [e for e in ctx.events if e.severity >= Severity.ERROR]
    out: list[Signal] = []
    for de in dep_events:
        later = [e for e in errors if e.timestamp > de.timestamp and (e.timestamp - de.timestamp) <= timedelta(seconds=300)]
        if later:
            ids = tuple(sorted({de.event_id, *(e.event_id for e in later)}, key=str))
            out.append(
                Signal(
                    name="deployment_near",
                    confidence=0.64,
                    evidence_event_ids=ids,
                    service_id=de.service_id,
                )
            )
        else:
            out.append(
                Signal(
                    name="deployment_near",
                    confidence=0.46,
                    evidence_event_ids=(de.event_id,),
                    service_id=de.service_id,
                )
            )
    return _dedupe_signals(out)


def detect_dns_failure(ctx: SignalContext) -> list[Signal]:
    hits = [e for e in ctx.events if "dns_failure" in e.tags or "nxdomain" in _msg_lower(e) or "name resolution" in _msg_lower(e)]
    if not hits:
        return []
    hits = sorted(hits, key=lambda e: (e.timestamp, e.event_id))
    ids = tuple(e.event_id for e in hits)
    return [Signal(name="dns_failure", confidence=0.78, evidence_event_ids=ids, service_id=None)]


def detect_certificate_error(ctx: SignalContext) -> list[Signal]:
    hits = [
        e
        for e in ctx.events
        if "certificate_error" in e.tags or "tls" in e.tags or any(p in _msg_lower(e) for p in _CERT_PATTERNS)
    ]
    if not hits:
        return []
    hits = sorted(hits, key=lambda e: (e.timestamp, e.event_id))
    ids = tuple(e.event_id for e in hits)
    return [Signal(name="certificate_error", confidence=0.76, evidence_event_ids=ids, service_id=hits[0].service_id)]


def detect_health_check_failing(ctx: SignalContext) -> list[Signal]:
    out: list[Signal] = []
    for e in ctx.events:
        if e.event_type != EventType.HEALTH_CHECK:
            continue
        low = _msg_lower(e)
        if any(t in low for t in ("fail", "unhealthy", "error", "down")):
            out.append(
                Signal(
                    name="health_check_failing",
                    confidence=0.85,
                    evidence_event_ids=(e.event_id,),
                    service_id=e.service_id,
                )
            )
    return out


def detect_unexpected_silence(ctx: SignalContext) -> list[Signal]:
    tag_hits = [e for e in ctx.events if "unexpected_silence" in e.tags or "heartbeat missing" in _msg_lower(e)]
    if tag_hits:
        hits = sorted(tag_hits, key=lambda e: (e.timestamp, e.event_id))
        ids = tuple(e.event_id for e in hits)
        return [Signal(name="unexpected_silence", confidence=0.7, evidence_event_ids=ids, service_id=None)]
    out: list[Signal] = []
    for sid, fps in sorted(ctx.expected_heartbeats.items()):
        for fp in sorted(fps):
            if not any(e.service_id == sid and e.fingerprint == fp for e in ctx.events):
                out.append(
                    Signal(
                        name="unexpected_silence",
                        confidence=0.68,
                        evidence_event_ids=tuple(),
                        service_id=sid,
                    )
                )
    return _dedupe_signals(out)


def detect_quota_exhaustion(ctx: SignalContext) -> list[Signal]:
    keys = ("quota", "rate limit", "429", "too many requests", "throttl")
    hits = [e for e in ctx.events if "quota" in e.tags or any(k in _msg_lower(e) for k in keys)]
    if not hits:
        return []
    hits = sorted(hits, key=lambda e: (e.timestamp, e.event_id))
    by_svc: dict[str, list] = defaultdict(list)
    for e in hits:
        by_svc[e.service_id].append(e)
    out: list[Signal] = []
    for sid in sorted(by_svc):
        evs = by_svc[sid]
        ids = tuple(e.event_id for e in sorted(evs, key=lambda x: (x.timestamp, x.event_id)))
        out.append(Signal(name="quota_exhaustion", confidence=0.74, evidence_event_ids=ids, service_id=sid))
    return out


def detect_database_contention(ctx: SignalContext) -> list[Signal]:
    out: list[Signal] = []
    for e in ctx.events:
        low = _msg_lower(e)
        if any(p in low for p in _DB_PATTERNS):
            out.append(
                Signal(
                    name="database_contention",
                    confidence=0.8,
                    evidence_event_ids=(e.event_id,),
                    service_id=e.service_id,
                )
            )
    return _dedupe_signals(out)


def detect_network_partition(ctx: SignalContext) -> list[Signal]:
    hits = [
        e
        for e in ctx.events
        if "network_partition" in e.tags or "split brain" in _msg_lower(e) or "network unreachable" in _msg_lower(e)
    ]
    if not hits:
        return []
    hits = sorted(hits, key=lambda e: (e.timestamp, e.event_id))
    ids = tuple(e.event_id for e in hits)
    return [Signal(name="network_partition", confidence=0.73, evidence_event_ids=ids, service_id=None)]


def _dedupe_signals(items: Iterable[Signal]) -> list[Signal]:
    seen: set[tuple[object, ...]] = set()
    out: list[Signal] = []
    for s in sorted(items, key=lambda x: (x.name, x.service_id or "", x.evidence_event_ids)):
        key = (s.name, s.service_id, s.evidence_event_ids)
        if key in seen:
            continue
        seen.add(key)
        out.append(s)
    return out


DETECTOR_FUNCS: tuple[Callable[[SignalContext], list[Signal]], ...] = (
    detect_certificate_error,
    detect_config_change_near,
    detect_connection_failures_outbound,
    detect_database_contention,
    detect_deployment_near,
    detect_dns_failure,
    detect_error_spike,
    detect_health_check_failing,
    detect_network_partition,
    detect_quota_exhaustion,
    detect_resource_pressure,
    detect_restart_pattern,
    detect_unexpected_silence,
)
