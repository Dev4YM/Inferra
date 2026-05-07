from __future__ import annotations

from typing import Any

from core.models import ExplanationResult
from core.time import to_iso
from events.models import NormalizedEvent

from explanation.finalize import explanation_result_from_dict, finalize_explanation_payload


class TemplateExplanationEngine:
    def generate(
        self,
        incident: dict[str, Any],
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> ExplanationResult:
        incident_id = str(incident.get("incident_id") or "")
        if not hypotheses:
            payload: dict[str, Any] = {
                "incident_id": incident_id,
                "summary": "No hypothesis is available for this incident yet.",
                "primary_hypothesis_text": "Insufficient evidence.",
                "evidence_narrative": "",
                "timeline_narrative": "",
                "alternative_explanations": [],
                "suggested_actions": ["Collect more events for the affected service."],
                "uncertainty_notes": ["No hypotheses were generated."],
                "generation_model": "template_fallback",
                "guardrail_violations": [],
            }
            finalized = finalize_explanation_payload(incident, hypotheses, events, payload, template=True)
            return explanation_result_from_dict(finalized)

        top = hypotheses[0]
        services = ", ".join(top.get("affected_services", [])) or "unknown services"
        timeline = "\n".join(
            f"[{to_iso(event.timestamp)}] {event.service_id}: {event.message}"
            for event in sorted(events, key=lambda item: item.timestamp)[:30]
        )
        alternatives = [str(hyp.get("description") or "") for hyp in hypotheses[1:4]]
        cause = str(top.get("cause_type") or "unknown_cause")
        description = str(top.get("description") or "")
        payload = {
            "incident_id": incident_id,
            "summary": f"Incident affecting {services}. Top hypothesis: {description}.",
            "primary_hypothesis_text": f"{cause}: {description}",
            "evidence_narrative": (
                f"The hypothesis is supported by {len(top.get('supporting_events', []))} stored events "
                f"and scored {float(top.get('total_score') or 0.0):.2f}."
            ),
            "timeline_narrative": timeline,
            "alternative_explanations": alternatives,
            "suggested_actions": list(top.get("suggested_checks") or []),
            "uncertainty_notes": [
                "This is a deterministic structured summary, not an LLM-generated explanation.",
                "Topology-aware attribution depends on configured service graph accuracy.",
            ],
            "generation_model": "template_fallback",
            "guardrail_violations": [],
        }
        finalized = finalize_explanation_payload(incident, hypotheses, events, payload, template=True)
        return explanation_result_from_dict(finalized)
