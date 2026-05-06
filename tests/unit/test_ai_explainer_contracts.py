from __future__ import annotations

from datetime import datetime
from unittest.mock import AsyncMock, MagicMock

import pytest

from ai.explainer import AIExplanationEngine
from ai.ollama import OllamaStreamChunk
from ai.prompts import ExplainIncidentOutput
from config.models import AIConfig
from core.enums import EventType, Severity
from events.models import DataQuality, NormalizedEvent, SourceRef


def _minimal_event(eid: str) -> NormalizedEvent:
    ts = datetime(2026, 1, 2, 3, 4, 5)
    return NormalizedEvent(
        event_id=eid,
        timestamp=ts,
        timestamp_source="parsed",
        service_id="api",
        host_id="host-a",
        severity=Severity.ERROR,
        event_type=EventType.LOG,
        message="timeout",
        structured_data={},
        tags=frozenset({"t"}),
        fingerprint=f"fp-{eid}",
        quality=DataQuality(1.0, 1.0, 1.0, 1.0, 1.0),
        source_ref=SourceRef("app", "src", None, ts),
    )


@pytest.mark.asyncio
async def test_explain_schema_mismatch_falls_back_to_template() -> None:
    ai_cfg = AIConfig(enabled=True, stream=False)
    provider = MagicMock()
    provider.config = ai_cfg
    provider.chat = AsyncMock(return_value="{}")

    engine = AIExplanationEngine(provider)
    incident = {
        "incident_id": "inc-1",
        "affected_services": ["api"],
        "time_range_start": "",
        "time_range_end": "",
        "primary_service": "api",
        "severity": 3,
        "state": "investigating",
    }
    hypotheses = [
        {
            "hypothesis_id": "h1",
            "rank": 1,
            "cause_type": "dependency_failure",
            "description": "postgres refused",
            "total_score": 0.9,
            "score_breakdown": {},
            "supporting_events": [],
            "contradicting_events": [],
            "suggested_checks": ["check dns"],
            "confidence_label": "high",
        }
    ]
    events = [_minimal_event("evt-1")]
    payload, trace = await engine.generate(incident, hypotheses, events)

    assert trace.trace_kind == "explain"
    assert payload["generation_model"] == "template_fallback"
    assert "schema_validation_fallback" in payload["guardrail_violations"]


@pytest.mark.asyncio
async def test_user_prompt_contains_summarized_payload_before_provider_call() -> None:
    ai_cfg = AIConfig(enabled=True, stream=False)
    provider = MagicMock()
    provider.config = ai_cfg

    async def capture(messages: list[dict[str, str]], model: str | None = None, *, retries: int = 0) -> str:
        user = messages[-1]["content"]
        assert "event_summaries" in user
        valid = ExplainIncidentOutput(
            summary="s",
            primary_hypothesis_text="p",
            evidence_narrative="e",
            timeline_narrative="t",
            alternative_explanations=[],
            suggested_actions=[],
            uncertainty_notes=[],
        )
        import json

        return json.dumps(valid.model_dump())

    provider.chat = capture

    engine = AIExplanationEngine(provider)
    incident = {
        "incident_id": "inc-1",
        "affected_services": ["api"],
        "time_range_start": "",
        "time_range_end": "",
        "primary_service": "api",
        "severity": 3,
        "state": "investigating",
    }
    await engine.generate(incident, [], [_minimal_event("evt-1")])


@pytest.mark.asyncio
async def test_stream_aggregate_equals_direct_chat_for_same_messages() -> None:
    full_json = '{"answer": "consistent"}'

    async def chunk_stream():
        yield OllamaStreamChunk(content='{"answer": "', done=False, raw={})
        yield OllamaStreamChunk(content="consistent", done=False, raw={})
        yield OllamaStreamChunk(content='"}', done=True, raw={})

    cfg_stream = AIConfig(enabled=True, stream=True)
    prov_stream = MagicMock()
    prov_stream.config = cfg_stream
    prov_stream.chat_stream = MagicMock(return_value=chunk_stream())

    cfg_direct = AIConfig(enabled=True, stream=False)
    prov_direct = MagicMock()
    prov_direct.config = cfg_direct
    prov_direct.chat = AsyncMock(return_value=full_json)

    messages = [{"role": "system", "content": "sys"}, {"role": "user", "content": "payload"}]

    aggregated = await AIExplanationEngine(prov_stream)._complete_messages(messages)
    direct = await AIExplanationEngine(prov_direct)._complete_messages(messages)

    assert aggregated == direct == full_json
