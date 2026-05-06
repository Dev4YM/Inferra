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
from config import InferraConfig, StorageConfig


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
