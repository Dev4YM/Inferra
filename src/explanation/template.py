from __future__ import annotations

from typing import Any

from core.ids import new_id
from core.time import to_iso
from events.models import NormalizedEvent


class TemplateExplanationEngine:
    def generate(
        self,
        incident_id: str,
        hypotheses: list[dict[str, Any]],
        events: list[NormalizedEvent],
    ) -> dict[str, Any]:
        if not hypotheses:
            return {
                "explanation_id": new_id("exp"),
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

        top = hypotheses[0]
        services = ", ".join(top.get("affected_services", [])) or "unknown services"
        timeline = "\n".join(
            f"[{to_iso(event.timestamp)}] {event.service_id}: {event.message}"
            for event in sorted(events, key=lambda item: item.timestamp)[:30]
        )
        alternatives = [hyp["description"] for hyp in hypotheses[1:4]]
        return {
            "explanation_id": new_id("exp"),
            "incident_id": incident_id,
            "summary": f"Incident affecting {services}. Top hypothesis: {top['description']}.",
            "primary_hypothesis_text": f"{top['cause_type']}: {top['description']}",
            "evidence_narrative": (
                f"The hypothesis is supported by {len(top.get('supporting_events', []))} stored events "
                f"and scored {top.get('total_score', 0):.2f}."
            ),
            "timeline_narrative": timeline,
            "alternative_explanations": alternatives,
            "suggested_actions": top.get("suggested_checks", []),
            "uncertainty_notes": [
                "This is a deterministic structured summary, not an LLM-generated explanation.",
                "Topology-aware attribution depends on configured service graph accuracy.",
            ],
            "generation_model": "template_fallback",
            "guardrail_violations": [],
        }
