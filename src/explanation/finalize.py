from __future__ import annotations

from typing import Any

from core.models import ExplanationResult
from events.models import NormalizedEvent

from .cache_key import explanation_cache_key_hashes, stable_template_explanation_id
from .guardrails import run_explanation_guardrails


def _sanitize_plaintext(value: str):
    from ai.redaction import sanitize_plaintext as _impl

    return _impl(value)


def finalize_explanation_payload(
    incident: dict[str, Any],
    hypotheses: list[dict[str, Any]],
    events: list[NormalizedEvent],
    payload: dict[str, Any],
    *,
    template: bool,
) -> dict[str, Any]:
    hyp_h, evt_h = explanation_cache_key_hashes(hypotheses, events)
    out = dict(payload)
    out["hypotheses_hash"] = hyp_h
    out["events_hash_head"] = evt_h
    out.setdefault("schema_version", 1)
    if template:
        out["explanation_id"] = stable_template_explanation_id(str(out.get("incident_id") or ""), hypotheses, events)
    else:
        out.setdefault("explanation_id", out.get("explanation_id") or "")
    for key in ("summary", "primary_hypothesis_text", "evidence_narrative", "timeline_narrative"):
        raw = out.get(key)
        if isinstance(raw, str):
            text, _report = _sanitize_plaintext(raw)
            out[key] = text
    for key in ("alternative_explanations", "suggested_actions", "uncertainty_notes"):
        items = out.get(key)
        if not isinstance(items, list):
            continue
        cleaned: list[str] = []
        for item in items:
            if not isinstance(item, str):
                continue
            text, _report = _sanitize_plaintext(item)
            cleaned.append(text)
        out[key] = cleaned
    violations = run_explanation_guardrails(
        incident,
        hypotheses,
        events,
        summary=str(out.get("summary") or ""),
        primary_hypothesis_text=str(out.get("primary_hypothesis_text") or ""),
        evidence_narrative=str(out.get("evidence_narrative") or ""),
        timeline_narrative=str(out.get("timeline_narrative") or ""),
        alternative_explanations=list(out.get("alternative_explanations") or []),
        suggested_actions=list(out.get("suggested_actions") or []),
        uncertainty_notes=list(out.get("uncertainty_notes") or []),
    )
    merged = sorted({*list(out.get("guardrail_violations") or []), *violations})
    out["guardrail_violations"] = merged
    out["quality"] = "degraded" if merged else "ok"
    return out


def explanation_result_from_dict(data: dict[str, Any]) -> ExplanationResult:
    return ExplanationResult(
        incident_id=str(data["incident_id"]),
        summary=str(data.get("summary") or ""),
        primary_hypothesis_text=str(data.get("primary_hypothesis_text") or ""),
        evidence_narrative=str(data.get("evidence_narrative") or ""),
        timeline_narrative=str(data.get("timeline_narrative") or ""),
        alternative_explanations=list(data.get("alternative_explanations") or []),
        suggested_actions=list(data.get("suggested_actions") or []),
        uncertainty_notes=list(data.get("uncertainty_notes") or []),
        generation_model=str(data.get("generation_model") or "template_fallback"),
        guardrail_violations=list(data.get("guardrail_violations") or []),
        explanation_id=str(data.get("explanation_id") or ""),
        hypotheses_hash=str(data.get("hypotheses_hash") or ""),
        events_hash_head=str(data.get("events_hash_head") or ""),
        schema_version=int(data.get("schema_version") or 1),
        quality=str(data.get("quality") or "ok"),
    )
