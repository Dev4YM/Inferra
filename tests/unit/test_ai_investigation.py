"""Unit tests for the AI investigation contract and deterministic fallback."""

from __future__ import annotations

import asyncio

from ai.investigation import (
    EvidenceBundle,
    InvestigationOutput,
    InvestigationStep,
    investigation_result_to_dict,
    redact_bundle,
    run_investigation,
)
from config import AIConfig, InferraConfig, StorageConfig


class _FakeAiService:
    async def status(self) -> dict[str, object]:
        return {
            "enabled": True,
            "available": True,
            "model": "fake-model",
            "base_url": "http://127.0.0.1:11434",
            "allow_remote": False,
        }


def test_investigation_step_defaults_to_read_only():
    step = InvestigationStep(title="Check service")
    assert step.safety == "read_only"
    assert step.requires_user_action is True


def test_investigation_output_validates_risk_level():
    out = InvestigationOutput(headline="x", risk_level="high", confidence=0.5)
    assert out.risk_level == "high"


def test_redact_bundle_summarizes_event_messages():
    bundle = EvidenceBundle(
        mode="operator",
        events=[
            {
                "event_id": "evt-1",
                "timestamp": "2024-01-01T00:00:00Z",
                "service_id": "api",
                "severity": 3,
                "message": "x" * 500,
                "tags": ["a"],
                "source_ref": {"source_type": "app"},
            }
        ],
    )
    redacted = redact_bundle(bundle, redact_raw_logs=True)
    events = redacted["events"]
    assert events
    assert "summary" in events[0]
    assert len(events[0]["summary"]) <= 240


def test_run_investigation_uses_deterministic_fallback_when_ai_disabled(tmp_path):
    config = InferraConfig(storage=StorageConfig(data_dir=tmp_path))
    bundle = EvidenceBundle(
        mode="operator",
        incident={"incident_id": "inc-test", "primary_service": "api", "severity": 3},
        services=[{"service_id": "api", "status": "degraded"}],
        events=[],
    )
    result = asyncio.run(run_investigation(config, bundle))
    assert result.used_ai is False
    assert result.fallback_reason
    assert result.output.risk_level in {"medium", "high", "critical"}
    assert result.output.next_steps, "fallback should suggest next steps"
    for step in result.output.next_steps:
        assert step.safety == "read_only"
        assert step.requires_user_action is True


def test_investigation_result_to_dict_redacts_fields(tmp_path):
    config = InferraConfig(storage=StorageConfig(data_dir=tmp_path))
    bundle = EvidenceBundle(
        mode="developer",
        events=[{"event_id": "evt-1", "service_id": "api", "severity": 1, "message": "ok"}],
    )
    result = asyncio.run(run_investigation(config, bundle))
    payload = investigation_result_to_dict(result)
    assert payload["used_ai"] is False
    assert payload["provider"]["enabled"] is False
    assert "output" in payload
    assert "bundle" in payload


def test_run_investigation_retries_empty_output_then_succeeds(tmp_path, monkeypatch):
    config = InferraConfig(storage=StorageConfig(data_dir=tmp_path), ai=AIConfig(enabled=True))
    bundle = EvidenceBundle(mode="operator", services=[{"service_id": "api", "status": "degraded"}], events=[])

    class _FakeProvider:
        responses = iter(
            [
                "   ",
                '{"headline":"API degraded","risk_level":"high","confidence":0.7,"what_happened":["api latency spiked"],"why_it_matters":["requests are failing"],"likely_causes":["database contention"],"evidence":[{"type":"service","id":"api","summary":"service degraded"}],"missing_evidence":[],"next_steps":[{"title":"Inspect api logs","reason":"validate recent failures","command":"inferra services events api --limit 25","safety":"read_only","requires_user_action":true}],"uncertainty":[],"citations":["api"]}',
            ]
        )

        def __init__(self, _config):
            pass

        async def chat(self, _messages):
            return next(self.responses)

    monkeypatch.setattr("ai.investigation.AsyncOllamaProvider", _FakeProvider)
    result = asyncio.run(run_investigation(config, bundle, ai_service=_FakeAiService()))
    assert result.used_ai is True
    assert result.attempts == 2
    assert result.warnings
    assert "empty response body" in result.warnings[0]
    assert result.output.headline == "API degraded"


def test_run_investigation_falls_back_when_payload_has_no_signal(tmp_path, monkeypatch):
    config = InferraConfig(storage=StorageConfig(data_dir=tmp_path), ai=AIConfig(enabled=True))
    bundle = EvidenceBundle(mode="operator", services=[{"service_id": "api", "status": "degraded"}], events=[])

    class _FakeProvider:
        def __init__(self, _config):
            pass

        async def chat(self, _messages):
            return (
                '{"headline":"","risk_level":"low","confidence":0.0,"what_happened":[],"why_it_matters":[],'
                '"likely_causes":[],"evidence":[],"missing_evidence":[],"next_steps":[],"uncertainty":[],"citations":[]}'
            )

    monkeypatch.setattr("ai.investigation.AsyncOllamaProvider", _FakeProvider)
    result = asyncio.run(run_investigation(config, bundle, ai_service=_FakeAiService()))
    payload = investigation_result_to_dict(result)
    assert result.used_ai is False
    assert result.attempts == 3
    assert result.warnings
    assert "meaningful content" in result.warnings[-1]
    assert "unusable after" in result.fallback_reason
    assert payload["warnings"]
