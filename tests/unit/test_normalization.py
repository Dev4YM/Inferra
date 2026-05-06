from __future__ import annotations

import json
from datetime import UTC, datetime
from pathlib import Path

import pytest

from config.models import NormalizationConfig, ServiceMappingConfig, TagRuleConfig
from core.enums import EventType, Severity
from events.models import RawEvent
from normalization.fingerprint import compute_fingerprint, extract_template
from normalization.pipeline import NormalizationPipeline

FIXTURE_DIR = Path(__file__).resolve().parents[1] / "fixtures" / "logs"
COLLECTED_AT = datetime(2026, 5, 4, 12, 0, tzinfo=UTC)


def _fixture_text(name: str) -> str:
    return (FIXTURE_DIR / name).read_text(encoding="utf-8").strip()


def _raw_event(
    *,
    source_type: str,
    fixture_name: str,
    source_id: str,
    metadata: dict[str, object],
    collected_at: datetime = COLLECTED_AT,
) -> RawEvent:
    return RawEvent(
        source_type=source_type,
        source_id=source_id,
        raw_payload=_fixture_text(fixture_name),
        collected_at=collected_at,
        metadata=metadata,
    )


@pytest.mark.parametrize(
    ("raw", "expected_service", "expected_severity", "expected_tag", "expected_event_type", "expected_host"),
    [
        (
            _raw_event(
                source_type="file",
                fixture_name="json_line.log",
                source_id="file://api.log",
                metadata={"path": "/var/log/api.log", "host": "collector-a"},
            ),
            "api",
            Severity.ERROR,
            "connection_refused",
            EventType.LOG,
            "collector-a",
        ),
        (
            _raw_event(
                source_type="file",
                fixture_name="syslog_rfc3164.log",
                source_id="file:///var/log/syslog",
                metadata={"path": "/var/log/syslog"},
            ),
            "nginx",
            Severity.ERROR,
            "timeout",
            EventType.LOG,
            "web-01",
        ),
        (
            _raw_event(
                source_type="file",
                fixture_name="syslog_rfc5424.log",
                source_id="file:///var/log/app.log",
                metadata={"path": "/var/log/app.log"},
            ),
            "billing-api",
            Severity.ERROR,
            "timeout",
            EventType.LOG,
            "host-01",
        ),
        (
            _raw_event(
                source_type="windows_eventlog",
                fixture_name="windows_eventlog.jsonl",
                source_id="windows_eventlog://System",
                metadata={"provider": "Service Control Manager", "computer_name": "WIN-01"},
            ),
            "service-control-manager",
            Severity.ERROR,
            "windows_eventlog",
            EventType.LOG,
            "win-01",
        ),
        (
            _raw_event(
                source_type="kubernetes",
                fixture_name="k8s_event.jsonl",
                source_id="kubernetes://prod",
                metadata={"namespace": "prod", "workload": "payments-api", "node": "node-a"},
            ),
            "payments-api",
            Severity.WARN,
            "kubernetes",
            EventType.STATE_CHANGE,
            "node-a",
        ),
        (
            _raw_event(
                source_type="docker",
                fixture_name="docker_json.log",
                source_id="docker://gateway",
                metadata={"container_name": "gateway-1", "container_id": "8a18803fbf30beef"},
            ),
            "gateway",
            Severity.ERROR,
            "docker",
            EventType.HEALTH_CHECK,
            "8a18803fbf30",
        ),
        (
            _raw_event(
                source_type="file",
                fixture_name="generic_text.log",
                source_id="file://listener.log",
                metadata={"path": "/var/log/listener.log", "host": "collector-b"},
            ),
            "listener",
            Severity.ERROR,
            "unrecognized_format",
            EventType.LOG,
            "collector-b",
        ),
    ],
)
def test_pipeline_normalizes_fixture_parsers(
    raw: RawEvent,
    expected_service: str,
    expected_severity: Severity,
    expected_tag: str,
    expected_event_type: EventType,
    expected_host: str,
):
    event = NormalizationPipeline().normalize(raw)

    assert event.service_id == expected_service
    assert event.severity == expected_severity
    assert expected_tag in event.tags or expected_tag in event.quality.flags
    assert event.event_type == expected_event_type
    assert event.host_id == expected_host
    assert event.timestamp_source == "parsed" or "timestamp_missing" in event.quality.flags


def test_pipeline_applies_service_mapping_host_override_and_process_context():
    config = NormalizationConfig(
        host_id="edge-node-01",
        service_mappings=[ServiceMappingConfig(pattern=r"listener\.log$", service_id="edge-listener")],
        tag_rules=[TagRuleConfig(pattern=r"listener failed", tags=["listener_failure"])],
    )
    raw = _raw_event(
        source_type="file",
        fixture_name="generic_text.log",
        source_id="file://listener.log",
        metadata={"path": "/var/log/listener.log", "pid": 4242, "comm": "listenerd"},
    )

    event = NormalizationPipeline(config).normalize(raw)

    assert event.service_id == "edge-listener"
    assert event.host_id == "edge-node-01"
    assert event.structured_data["process_context"] == {"pid": 4242, "comm": "listenerd"}
    assert "listener_failure" in event.tags


def test_pipeline_truncates_message_and_compacts_large_structured_payload():
    raw = RawEvent(
        source_type="file",
        source_id="file://oversized.log",
        raw_payload='{"timestamp":"2026-05-04T10:00:00Z","service":"api","level":"error","message":"abcdefghij","payload":{"secret":"x","nested":{"token":"y"}}}',
        collected_at=COLLECTED_AT,
        metadata={"path": "/var/log/oversized.log"},
    )

    event = NormalizationPipeline(
        NormalizationConfig(
            max_message_length=5,
            max_structured_data_bytes=32,
        )
    ).normalize(raw)

    assert event.message == "ab..."
    assert "truncated" in event.quality.flags
    assert "structured_payload_dropped" in event.quality.flags
    assert dict(event.structured_data) == {
        "keys": ("host_id", "level", "message", "payload", "service", "service_id", "timestamp")
    }


def test_pipeline_rejects_out_of_range_timestamps_by_falling_back_to_collected_at():
    future = RawEvent(
        source_type="file",
        source_id="file://future.log",
        raw_payload='{"timestamp":"2026-06-10T10:00:00Z","service":"api","level":"error","message":"future error"}',
        collected_at=COLLECTED_AT,
        metadata={"path": "/var/log/future.log"},
    )
    past = RawEvent(
        source_type="file",
        source_id="file://past.log",
        raw_payload='{"timestamp":"2026-01-01T10:00:00Z","service":"api","level":"error","message":"old error"}',
        collected_at=COLLECTED_AT,
        metadata={"path": "/var/log/past.log"},
    )

    future_event = NormalizationPipeline().normalize(future)
    past_event = NormalizationPipeline().normalize(past)

    assert future_event.timestamp == COLLECTED_AT
    assert future_event.timestamp_source == "collected_at"
    assert "timestamp_in_future" in future_event.quality.flags
    assert "clock_skew_future" in future_event.quality.flags
    assert past_event.timestamp == COLLECTED_AT
    assert past_event.timestamp_source == "collected_at"
    assert "timestamp_too_old" in past_event.quality.flags


def test_pipeline_mild_future_timestamp_does_not_set_clock_skew_flag():
    mild_ts = COLLECTED_AT.replace(minute=2)
    raw = RawEvent(
        source_type="file",
        source_id="file://mild.log",
        raw_payload=json.dumps(
            {
                "timestamp": mild_ts.strftime("%Y-%m-%dT%H:%M:%SZ"),
                "service": "api",
                "level": "error",
                "message": "mild future",
            }
        ),
        collected_at=COLLECTED_AT,
        metadata={"path": "/var/log/mild.log"},
    )
    event = NormalizationPipeline().normalize(raw)
    assert "timestamp_in_future" in event.quality.flags
    assert "clock_skew_future" not in event.quality.flags


def test_fingerprint_is_stable_for_500_template_variations():
    fingerprints = {
        compute_fingerprint(
            "api",
            (
                f"2026-05-04T10:{index % 60:02d}:30Z request {index} for user-{index}@example.com "
                f"to 10.0.{index % 10}.{index % 255}:5432 at /srv/app/{index}/config.yaml "
                f"uuid 123e4567-e89b-42d3-a456-{index:012d} hex 0x{index:08x} failed"
            ),
            Severity.ERROR,
        )
        for index in range(500)
    }

    assert len(fingerprints) == 1
    assert extract_template(
        "2026-05-04T10:00:30Z request 99 for user-99@example.com to 10.0.1.2:5432 at /srv/app/99/config.yaml hex 0x0000ffff failed"
    ).count("{N}") >= 1


def test_unrecognized_input_emits_best_effort_event_below_half_quality():
    raw = _raw_event(
        source_type="file",
        fixture_name="generic_text.log",
        source_id="file://listener.log",
        metadata={"path": "/var/log/listener.log"},
    )

    event = NormalizationPipeline().normalize(raw)

    assert "unrecognized_format" in event.quality.flags
    assert event.quality.overall < 0.5


def test_quality_score_regression_fixture():
    raw = _raw_event(
        source_type="file",
        fixture_name="json_line.log",
        source_id="file://api.log",
        metadata={"path": "/var/log/api.log"},
    )

    quality = NormalizationPipeline().normalize(raw).quality

    assert {
        "overall": quality.overall,
        "timestamp_confidence": quality.timestamp_confidence,
        "parse_confidence": quality.parse_confidence,
        "identity_confidence": quality.identity_confidence,
        "completeness": quality.completeness,
        "flags": quality.flags,
    } == {
        "overall": 0.965,
        "timestamp_confidence": 1.0,
        "parse_confidence": 0.93,
        "identity_confidence": 0.95,
        "completeness": 1.0,
        "flags": frozenset(),
    }
