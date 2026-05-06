from __future__ import annotations

import itertools
from dataclasses import dataclass
from types import SimpleNamespace
from typing import Any

from config.models import CustomHypothesisRuleConfig, HypothesisEngineConfig
from core.enums import CauseType, InferenceEdgeType
from core.models import CompositionRule, InferenceGraph
from events.models import NormalizedEvent

from reasoning.signals.types import Signal


def _cause_from_str(value: str) -> CauseType:
    try:
        return CauseType(value)
    except ValueError:
        return CauseType.UNKNOWN


def builtin_composition_rules() -> tuple[CompositionRule, ...]:
    return (
        CompositionRule(
            name="resource_pressure_only",
            requires=["resource_pressure"],
            cause_type=CauseType.RESOURCE_EXHAUSTION,
            cause_subtype="resource_pressure",
            title_template="Resource exhaustion signals detected",
            confidence=0.78,
        ),
        CompositionRule(
            name="restart_pattern",
            requires=["restart_pattern"],
            cause_type=CauseType.APPLICATION_BUG,
            cause_subtype="crash_loop",
            title_template="Restart or crash pattern on {restart_pattern.service_id}",
            confidence=0.72,
        ),
        CompositionRule(
            name="dns_failure",
            requires=["dns_failure"],
            cause_type=CauseType.INFRASTRUCTURE_FAILURE,
            cause_subtype="dns_failure",
            title_template="DNS resolution failure",
            confidence=0.76,
        ),
        CompositionRule(
            name="certificate_error",
            requires=["certificate_error"],
            cause_type=CauseType.INFRASTRUCTURE_FAILURE,
            cause_subtype="certificate_error",
            title_template="TLS or certificate validation failure",
            confidence=0.74,
        ),
        CompositionRule(
            name="database_contention",
            requires=["database_contention"],
            cause_type=CauseType.DATABASE_FAILURE,
            cause_subtype="contention",
            title_template="Database contention on {database_contention.service_id}",
            confidence=0.78,
        ),
        CompositionRule(
            name="network_partition",
            requires=["network_partition"],
            cause_type=CauseType.INFRASTRUCTURE_FAILURE,
            cause_subtype="network_partition",
            title_template="Possible network partition",
            confidence=0.71,
        ),
        CompositionRule(
            name="health_check_failing",
            requires=["health_check_failing"],
            cause_type=CauseType.DEPENDENCY_FAILURE,
            cause_subtype="health_check_down",
            title_template="Health check failing on {health_check_failing.service_id}",
            confidence=0.7,
        ),
        CompositionRule(
            name="quota_exhaustion",
            requires=["quota_exhaustion"],
            cause_type=CauseType.RESOURCE_EXHAUSTION,
            cause_subtype="quota_or_rate_limit",
            title_template="Quota or rate limit pressure on {quota_exhaustion.service_id}",
            confidence=0.73,
        ),
        CompositionRule(
            name="unexpected_silence",
            requires=["unexpected_silence"],
            cause_type=CauseType.UNKNOWN,
            cause_subtype="missing_expected_traffic",
            title_template="Unexpected silence or missing heartbeats",
            confidence=0.55,
        ),
        CompositionRule(
            name="deployment_errors",
            requires=["deployment_near", "error_spike"],
            cause_type=CauseType.CONFIGURATION_ERROR,
            cause_subtype="deployment",
            title_template="Errors after deployment on {deployment_near.service_id}",
            confidence=0.64,
            requires_temporal_order=True,
        ),
        CompositionRule(
            name="config_errors",
            requires=["config_change_near", "error_spike"],
            cause_type=CauseType.CONFIGURATION_ERROR,
            cause_subtype="config_change",
            title_template="Errors after configuration change on {config_change_near.service_id}",
            confidence=0.63,
            requires_temporal_order=True,
        ),
        CompositionRule(
            name="outbound_dependency",
            requires=["connection_failures_outbound", "error_spike"],
            cause_type=CauseType.DEPENDENCY_FAILURE,
            cause_subtype="outbound_connectivity",
            title_template="Outbound connectivity failures on {connection_failures_outbound.service_id}",
            confidence=0.74,
            requires_same_service=True,
        ),
        CompositionRule(
            name="resource_and_errors",
            requires=["resource_pressure", "error_spike"],
            cause_type=CauseType.RESOURCE_EXHAUSTION,
            cause_subtype="resource_and_errors",
            title_template="Resource pressure correlated with errors",
            confidence=0.81,
        ),
    )


def merge_rules(engine_cfg: HypothesisEngineConfig) -> list[CompositionRule]:
    merged: list[CompositionRule] = list(builtin_composition_rules())
    for raw in engine_cfg.custom_rules:
        merged.append(_custom_to_rule(raw))
    return merged


def _custom_to_rule(cfg: CustomHypothesisRuleConfig) -> CompositionRule:
    return CompositionRule(
        name=cfg.name or "custom",
        requires=list(cfg.requires),
        cause_type=_cause_from_str(cfg.cause_type),
        cause_subtype=cfg.cause_subtype,
        title_template=cfg.title_template or cfg.name,
        confidence=float(cfg.confidence),
        requires_same_service=bool(cfg.requires_same_service),
        requires_temporal_order=bool(cfg.requires_temporal_order),
    )


def _signal_ctx(signals: list[Signal]) -> dict[str, Any]:
    ctx: dict[str, Any] = {}
    for s in signals:
        prefix = s.name.replace("-", "_")
        ctx[prefix] = SimpleNamespace(
            service_id=s.service_id or "",
            confidence=s.confidence,
            evidence_event_ids=s.evidence_event_ids,
        )
        ctx[f"{prefix}_service_id"] = s.service_id or ""
        ctx[f"{prefix}_confidence"] = s.confidence
    return ctx


def _earliest_ts(signal: Signal, by_id: dict[str, NormalizedEvent]) -> float:
    times = [by_id[eid].timestamp.timestamp() for eid in signal.evidence_event_ids if eid in by_id]
    return min(times) if times else 0.0


def _signals_temporally_ordered(requires: list[str], combo: tuple[Signal, ...], by_id: dict[str, NormalizedEvent]) -> bool:
    times = [_earliest_ts(s, by_id) for s in combo]
    return all(times[i] <= times[i + 1] + 1e-6 for i in range(len(times) - 1))


def _same_service(combo: tuple[Signal, ...]) -> bool:
    services = {s.service_id for s in combo if s.service_id}
    return len(services) <= 1


@dataclass(frozen=True)
class RawHypothesis:
    hypothesis_id: str
    cause_type: CauseType
    cause_subtype: str
    title: str
    description: str
    root_cause_event_id: str | None
    affected_services: tuple[str, ...]
    supporting_events: tuple[str, ...]
    suggested_checks: tuple[str, ...]
    generation_rule: str
    generation_confidence: float


def compose_from_signals(
    signals: tuple[Signal, ...],
    rules: list[CompositionRule],
    by_id: dict[str, NormalizedEvent],
    incident_id: str,
) -> list[RawHypothesis]:
    sig_list = list(signals)
    by_name: dict[str, list[Signal]] = {}
    for s in sig_list:
        by_name.setdefault(s.name, []).append(s)
    for key in by_name:
        by_name[key].sort(key=lambda x: (x.service_id or "", x.evidence_event_ids))
    out: list[RawHypothesis] = []
    seen_ids: set[str] = set()
    for rule in rules:
        lists = [by_name.get(req) for req in rule.requires]
        if any(x is None for x in lists):
            continue
        assert lists  # for mypy
        for combo in itertools.product(*lists):
            if rule.requires_same_service and not _same_service(combo):
                continue
            if rule.requires_temporal_order and not _signals_temporally_ordered(rule.requires, combo, by_id):
                continue
            ctx_map = _signal_ctx(list(combo))
            try:
                title = rule.title_template.format(**ctx_map)
            except (KeyError, ValueError, IndexError):
                title = rule.title_template
            subtype = rule.cause_subtype
            try:
                subtype = rule.cause_subtype.format(**ctx_map)
            except (KeyError, ValueError, IndexError):
                pass
            ev_ids: list[str] = []
            for s in combo:
                ev_ids.extend(list(s.evidence_event_ids))
            supporting = tuple(sorted(set(ev_ids), key=str))
            services = tuple(sorted({by_id[e].service_id for e in supporting if e in by_id}))
            root = supporting[0] if supporting else None
            if supporting:
                root = min(supporting, key=lambda eid: (by_id[eid].timestamp, eid))
            hid = _stable_id(incident_id, rule.name, supporting)
            if hid in seen_ids:
                continue
            seen_ids.add(hid)
            out.append(
                RawHypothesis(
                    hypothesis_id=hid,
                    cause_type=rule.cause_type,
                    cause_subtype=subtype,
                    title=title,
                    description=title,
                    root_cause_event_id=root,
                    affected_services=services,
                    supporting_events=supporting,
                    suggested_checks=_checks_for_cause(rule.cause_type),
                    generation_rule=rule.name,
                    generation_confidence=rule.confidence,
                )
            )
    return out


def compose_from_paths(
    graph: InferenceGraph,
    by_id: dict[str, NormalizedEvent],
    incident_id: str,
) -> list[RawHypothesis]:
    adj: dict[str, list[str]] = {}
    for edge in sorted(graph.edges, key=lambda e: (e.source_event_id, e.target_event_id)):
        adj.setdefault(edge.source_event_id, []).append(edge.target_event_id)
    for key in adj:
        adj[key] = sorted(set(adj[key]))
    paths: list[list[str]] = []
    for origin in sorted(graph.root_candidates):
        stack: list[list[str]] = [[origin]]
        while stack:
            path = stack.pop()
            node = path[-1]
            children = adj.get(node, [])
            if not children:
                if len(path) >= 2:
                    paths.append(path)
                continue
            for child in sorted(children):
                if child in path:
                    continue
                stack.append(path + [child])
    out: list[RawHypothesis] = []
    seen: set[str] = set()
    for idx, path in enumerate(sorted(paths, key=lambda p: (len(p), p))):
        raw = _path_to_raw(incident_id, path, graph, by_id, idx)
        if raw.hypothesis_id in seen:
            continue
        seen.add(raw.hypothesis_id)
        out.append(raw)
    return out


def _path_to_raw(
    incident_id: str,
    path: list[str],
    graph: InferenceGraph,
    by_id: dict[str, NormalizedEvent],
    idx: int,
) -> RawHypothesis:
    edge_types: list[InferenceEdgeType] = []
    plausibilities: list[float] = []
    for i in range(len(path) - 1):
        edge = graph.get_edge(path[i], path[i + 1])
        if edge:
            edge_types.append(edge.edge_type)
            plausibilities.append(edge.plausibility)
    min_pl = min(plausibilities) if plausibilities else 0.35
    cause, subtype = _classify_path(edge_types, path, by_id)
    services = sorted({by_id[e].service_id for e in path if e in by_id})
    root = path[0]
    root_ev = by_id.get(root)
    last_ev = by_id.get(path[-1])
    if cause == CauseType.DEPENDENCY_FAILURE and root_ev:
        joined = ", ".join(sorted(services))
        description = f"Connection failures or timeouts affecting {joined}; likely upstream/root service: {root_ev.service_id}"
    elif root_ev and last_ev:
        description = f"Inferred sequence ({cause.value}) from {root_ev.service_id} to {last_ev.service_id}"
    else:
        description = f"Inferred sequence ({cause.value}) across {len(path)} events"
    title = f"{cause.value.replace('_', ' ')}: {' -> '.join(by_id[e].service_id for e in path if e in by_id)}"
    hid = _stable_id(incident_id, "path", tuple(path))
    checks = _checks_for_cause(cause)
    return RawHypothesis(
        hypothesis_id=hid,
        cause_type=cause,
        cause_subtype=subtype,
        title=title,
        description=description,
        root_cause_event_id=root,
        affected_services=tuple(services),
        supporting_events=tuple(path),
        suggested_checks=checks,
        generation_rule="inference_path",
        generation_confidence=round(min(0.95, 0.55 + min_pl * 0.45), 4),
    )


def _classify_path(
    edge_types: list[InferenceEdgeType],
    path: list[str],
    by_id: dict[str, NormalizedEvent],
) -> tuple[CauseType, str]:
    tags: set[str] = set()
    msg = ""
    for eid in path:
        ev = by_id.get(eid)
        if ev:
            tags |= set(ev.tags)
            msg += ev.message.lower() + " "
    if any(p in msg for p in ("deadlock", "connection pool", "lock timeout", "too many connections")):
        return CauseType.DATABASE_FAILURE, "query_or_pool"
    if InferenceEdgeType.CONFIG_PRECEDED_ERROR in edge_types:
        return CauseType.CONFIGURATION_ERROR, "change_preceded_errors"
    if InferenceEdgeType.RESOURCE_PRECEDED_ERROR in edge_types or "resource_pressure" in tags:
        return CauseType.RESOURCE_EXHAUSTION, "resource_preceded_symptoms"
    if "dns_failure" in tags or "nxdomain" in msg:
        return CauseType.INFRASTRUCTURE_FAILURE, "dns"
    if "network_partition" in tags or "split brain" in msg:
        return CauseType.INFRASTRUCTURE_FAILURE, "partition"
    if any(t in msg for t in ("certificate", "x509", "ssl", "tls alert")):
        return CauseType.INFRASTRUCTURE_FAILURE, "tls"
    if InferenceEdgeType.RESTART_PRECEDED_DISCONNECTION in edge_types or "restart" in tags:
        return CauseType.APPLICATION_BUG, "restart_instability"
    if InferenceEdgeType.DEPENDENCY_PROPAGATION in edge_types or InferenceEdgeType.TIMEOUT_CHAIN in edge_types:
        return CauseType.DEPENDENCY_FAILURE, "dependency_propagation"
    if InferenceEdgeType.SHARED_FATE in edge_types:
        return CauseType.RESOURCE_EXHAUSTION, "shared_host_fate"
    return CauseType.UNKNOWN, "inferred_sequence"


def _checks_for_cause(cause: CauseType) -> tuple[str, ...]:
    if cause == CauseType.DEPENDENCY_FAILURE:
        return (
            "Check upstream service health and dependency timeouts",
            "Review recent connection errors for the affected hop",
        )
    if cause == CauseType.RESOURCE_EXHAUSTION:
        return (
            "Check host CPU, memory, and disk utilization",
            "Inspect high-usage processes and service resource limits",
        )
    if cause == CauseType.APPLICATION_BUG:
        return ("Inspect service logs before restart", "Check process exit codes")
    if cause == CauseType.CONFIGURATION_ERROR:
        return ("Review recent deployments", "Diff recent configuration changes")
    if cause == CauseType.DATABASE_FAILURE:
        return ("Inspect database locks and pool sizing", "Review slow queries and connection counts")
    if cause == CauseType.INFRASTRUCTURE_FAILURE:
        return ("Verify DNS and TLS trust chain", "Check network path and load balancer health")
    return ("Inspect grouped event timeline", "Add service topology for stronger correlation")


def _stable_key(parts: tuple[object, ...]) -> str:
    flat: list[str] = []
    for part in parts:
        if isinstance(part, tuple | list):
            flat.extend(str(item) for item in part)
        else:
            flat.append(str(part))
    return "|".join(flat)


def _stable_id(incident_id: str, kind: str, key: tuple[object, ...]) -> str:
    import hashlib

    digest = hashlib.sha256(f"{kind}|{_stable_key(key)}".encode("utf-8")).hexdigest()[:12]
    return f"{incident_id}-{kind}-{digest}"


def dedup_raw_hypotheses(items: list[RawHypothesis], overlap_threshold: float) -> list[RawHypothesis]:
    if not items:
        return []
    sorted_items = sorted(items, key=lambda h: (-h.generation_confidence, h.hypothesis_id))
    kept: list[RawHypothesis] = []
    for cand in sorted_items:
        drop = False
        sup_a = set(cand.supporting_events)
        for existing in kept:
            sup_b = set(existing.supporting_events)
            inter = len(sup_a & sup_b)
            union = len(sup_a | sup_b) or 1
            if inter / union >= overlap_threshold:
                drop = True
                break
        if not drop:
            kept.append(cand)
    return kept


def standalone_signal_hypothesis(
    incident_id: str,
    signal: Signal,
    by_id: dict[str, NormalizedEvent],
) -> RawHypothesis:
    cause = CauseType.UNKNOWN
    if signal.name in ("connection_failures_outbound",):
        cause = CauseType.DEPENDENCY_FAILURE
    elif signal.name in ("resource_pressure", "quota_exhaustion"):
        cause = CauseType.RESOURCE_EXHAUSTION
    elif signal.name in ("restart_pattern",):
        cause = CauseType.APPLICATION_BUG
    elif signal.name in ("deployment_near", "config_change_near"):
        cause = CauseType.CONFIGURATION_ERROR
    elif signal.name in ("dns_failure", "certificate_error", "network_partition"):
        cause = CauseType.INFRASTRUCTURE_FAILURE
    elif signal.name in ("database_contention",):
        cause = CauseType.DATABASE_FAILURE
    elif signal.name in ("health_check_failing",):
        cause = CauseType.DEPENDENCY_FAILURE
    supporting = tuple(sorted(signal.evidence_event_ids, key=str))
    services = tuple(sorted({by_id[e].service_id for e in supporting if e in by_id}))
    root = min(supporting, key=lambda eid: (by_id[eid].timestamp, eid)) if supporting else None
    title = f"{signal.name.replace('_', ' ')} ({signal.service_id or 'cluster'})"
    hid = _stable_id(incident_id, "sig", (signal.name, signal.service_id or "", supporting))
    return RawHypothesis(
        hypothesis_id=hid,
        cause_type=cause,
        cause_subtype=signal.name,
        title=title,
        description=title,
        root_cause_event_id=root,
        affected_services=services,
        supporting_events=supporting,
        suggested_checks=_checks_for_cause(cause),
        generation_rule=f"signal:{signal.name}",
        generation_confidence=signal.confidence,
    )
