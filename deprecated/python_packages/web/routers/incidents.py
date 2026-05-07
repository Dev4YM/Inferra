"""Incident routes: list, detail, events, hypotheses, feedback, clusters,
explanation, chat, state log, resolve.

The chat-streaming websocket lives in `web.api` because it shares state with
the global `/ws` channel; the HTTP chat endpoint is here.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from fastapi import APIRouter, Body, HTTPException, Request

from ai.service import AIService
from inferra_legacy.app import InferraRuntime
from core.enums import IncidentState
from core.models import IncidentFeedback, ResolutionInfo
from core.time import to_iso, utc_now
from events.serialization import event_to_dict
from explanation import TemplateExplanationEngine
from explanation.cache_key import explanation_cache_key_hashes
from explanation.finalize import explanation_result_from_dict
from storage.calibration_store import update_calibration
from storage.weight_store import update_weights
from web._shared import (
    active_incidents,
    explanation_to_dict,
    hypothesis_to_dict,
    incident_to_dict,
    persist_ai_prompt_trace,
)
from web.live_hub import LiveHub


@dataclass(frozen=True)
class IncidentsDeps:
    runtime: InferraRuntime
    ai_holder: list[AIService]
    explanations: TemplateExplanationEngine
    live_hub: LiveHub


def build_incidents_router(deps: IncidentsDeps) -> APIRouter:
    router = APIRouter(prefix="/api/incidents")
    runtime = deps.runtime
    ai_holder = deps.ai_holder
    live_hub = deps.live_hub
    explanations = deps.explanations

    @router.get("")
    async def incidents() -> dict[str, Any]:
        return {"incidents": [incident_to_dict(item) for item in active_incidents(runtime)]}

    @router.get("/{incident_id}")
    async def incident(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
        events_for_incident = [event for event in events_for_incident if event is not None]
        return {
            "incident": incident_to_dict(item),
            "events": [event_to_dict(event) for event in events_for_incident],
            "clusters": runtime.incident_store.get_clusters(incident_id),
            "hypotheses": [
                hypothesis_to_dict(hypothesis)
                for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
            ],
        }

    @router.get("/{incident_id}/events")
    async def incident_events(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
        return {"events": [event_to_dict(event) for event in events_for_incident if event is not None]}

    @router.get("/{incident_id}/hypotheses")
    async def incident_hypotheses(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        return {
            "hypotheses": [
                hypothesis_to_dict(hypo)
                for hypo in runtime.incident_store.get_hypotheses(incident_id)
            ]
        }

    @router.post("/{incident_id}/feedback")
    async def incident_feedback(incident_id: str, payload: dict[str, Any] = Body(...)) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        hypotheses = runtime.incident_store.get_hypotheses(incident_id)
        if not hypotheses:
            raise HTTPException(status_code=400, detail="No hypotheses recorded for this incident")
        was_correct = bool(payload.get("was_correct", True))
        correct_id = payload.get("correct_hypothesis_id")
        if was_correct and not correct_id:
            correct_id = hypotheses[0].hypothesis_id
        if not was_correct and not correct_id:
            feedback_type = "none_correct"
        else:
            feedback_type = "confirmed"
        feedback = IncidentFeedback(
            incident_id=incident_id,
            resolved_at=utc_now(),
            correct_hypothesis_id=str(correct_id) if correct_id else None,
            feedback_type=feedback_type,
            operator_notes=str(payload.get("notes") or ""),
        )
        if runtime.config.scoring.tuning.enabled:
            weight_state = runtime.weight_store.load()
            update_weights(
                weight_state,
                feedback,
                hypotheses,
                tuning=runtime.config.scoring.tuning,
            )
            runtime.weight_store.save(weight_state)
        if feedback_type != "none_correct":
            cal_model = runtime.calibration_store.load()
            update_calibration(
                cal_model,
                feedback,
                hypotheses,
                min_samples=int(runtime.config.calibration.min_samples_per_bucket),
            )
            runtime.calibration_store.save(cal_model)
        item_after = runtime.incident_store.get_incident(incident_id)
        if item_after is not None:
            await live_hub.broadcast("incident_updated", incident_to_dict(item_after))
        return {"stored": True, "feedback_type": feedback_type, "correct_hypothesis_id": correct_id}

    @router.get("/{incident_id}/clusters")
    async def incident_clusters(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        return {"clusters": runtime.incident_store.get_clusters(incident_id)}

    @router.get("/{incident_id}/explanation")
    async def incident_explanation(incident_id: str, request: Request) -> dict[str, Any]:
        if not request.app.state.rate_explain.consume(_client_ip(request)):
            raise HTTPException(status_code=429, detail="explain rate limit exceeded")
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
        events_for_incident = [event for event in events_for_incident if event is not None]
        hypotheses = [
            hypothesis_to_dict(hypothesis)
            for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
        ]
        incident_payload = incident_to_dict(item)
        hyp_h, evt_h = explanation_cache_key_hashes(hypotheses, events_for_incident)
        cached = runtime.incident_store.get_cached_explanation(incident_id, hyp_h, evt_h)
        if cached is not None:
            if item.state == IncidentState.INVESTIGATING:
                runtime.incident_store.transition_state(
                    incident_id,
                    IncidentState.EXPLAINED,
                    "explanation persisted",
                )
            exp_dict = explanation_to_dict(cached)
            await live_hub.broadcast(
                "explanation_ready",
                {"incident_id": incident_id, "explanation": exp_dict},
            )
            return {"explanation": exp_dict}
        explanation_job = await ai_holder[0].explain(incident_payload, hypotheses, events_for_incident)
        if explanation_job is None:
            explanation_result = explanations.generate(incident_payload, hypotheses, events_for_incident)
        else:
            explanation_payload, prompt_trace = explanation_job
            explanation_result = explanation_result_from_dict(explanation_payload)
            if prompt_trace is not None:
                persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
        runtime.incident_store.add_explanation(explanation_result)
        item_after = runtime.incident_store.get_incident(incident_id)
        if item_after is not None and item_after.state == IncidentState.INVESTIGATING:
            runtime.incident_store.transition_state(
                incident_id,
                IncidentState.EXPLAINED,
                "explanation persisted",
            )
        exp_dict = explanation_to_dict(explanation_result)
        await live_hub.broadcast("explanation_ready", {"incident_id": incident_id, "explanation": exp_dict})
        return {"explanation": exp_dict}

    @router.post("/{incident_id}/chat")
    async def incident_chat(
        incident_id: str,
        request: Request,
        payload: dict[str, Any] = Body(...),
    ) -> dict[str, Any]:
        if not request.app.state.rate_chat.consume(_client_ip(request)):
            raise HTTPException(status_code=429, detail="chat rate limit exceeded")
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        question = payload.get("question")
        if not question:
            raise HTTPException(status_code=400, detail="'question' is required")
        events_for_incident = [runtime.event_store.get_event(event_id) for event_id in item.events]
        events_for_incident = [event for event in events_for_incident if event is not None]
        hypotheses = [
            hypothesis_to_dict(hypothesis)
            for hypothesis in runtime.incident_store.get_hypotheses(incident_id)
        ]
        prior_messages = runtime.incident_store.list_chat_messages(incident_id)
        history_rows = [{"role": message.role, "content": message.content} for message in prior_messages]
        runtime.incident_store.append_chat_message(incident_id, "user", str(question))
        chat_payload, prompt_trace = await ai_holder[0].chat(
            str(question),
            incident_to_dict(item),
            hypotheses,
            events_for_incident,
            history=history_rows,
        )
        runtime.incident_store.append_chat_message(incident_id, "assistant", str(chat_payload.get("answer") or ""))
        if prompt_trace is not None:
            persist_ai_prompt_trace(runtime, incident_id, prompt_trace)
        return {key: value for key, value in chat_payload.items() if key != "_trace"}

    @router.get("/{incident_id}/chat/messages")
    async def incident_chat_messages(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        messages = runtime.incident_store.list_chat_messages(incident_id)
        return {
            "messages": [
                {
                    "message_id": message.message_id,
                    "role": message.role,
                    "content": message.content,
                    "created_at": message.created_at,
                    "schema_version": message.schema_version,
                }
                for message in messages
            ]
        }

    @router.get("/{incident_id}/state-log")
    async def incident_state_log(incident_id: str) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        entries = runtime.incident_store.list_state_log(incident_id)
        return {
            "incident_id": incident_id,
            "entries": [
                {
                    "log_id": entry.log_id,
                    "old_state": entry.old_state,
                    "new_state": entry.new_state,
                    "changed_at": to_iso(entry.changed_at),
                    "reason": entry.reason,
                }
                for entry in entries
            ],
        }

    @router.post("/{incident_id}/resolve")
    async def resolve_incident(
        incident_id: str,
        payload: dict[str, Any] | None = Body(default=None),
    ) -> dict[str, Any]:
        item = runtime.incident_store.get_incident(incident_id)
        if item is None:
            raise HTTPException(status_code=404, detail="Incident not found")
        feedback = payload or {}
        runtime.incident_store.resolve_incident(
            incident_id,
            ResolutionInfo(
                resolved_by=str(feedback.get("resolved_by", "operator")),
                correct_hypothesis_id=feedback.get("correct_hypothesis_id"),
                feedback_type=str(feedback.get("feedback_type", "confirmed")),
                notes=feedback.get("notes"),
                resolved_at=utc_now(),
            ),
        )
        await live_hub.broadcast(
            "incident_resolved",
            {"incident_id": incident_id, "reason": "operator_resolve"},
        )
        return {"resolved": True, "feedback": feedback}

    return router


def _client_ip(request: Request) -> str:
    return request.client.host if request.client else "unknown"
