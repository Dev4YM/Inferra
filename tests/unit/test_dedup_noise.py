from __future__ import annotations

import json
import time
from datetime import UTC, datetime, timedelta

from config.models import (
    DeduplicationConfig,
    NoiseAllowlistConfig,
    NoiseBlocklistConfig,
    NoiseFilterConfig,
)
from core.enums import EventType, Severity
from events.models import DataQuality, NormalizedEvent, SourceRef
from normalization.dedup import DedupTracker
from normalization.noise import NoiseFilter

_BASE_TIME = datetime(2026, 5, 4, 12, 0, 0, tzinfo=UTC)
_QUALITY = DataQuality(
    overall=0.95,
    timestamp_confidence=1.0,
    parse_confidence=1.0,
    identity_confidence=1.0,
    completeness=1.0,
)
_SOURCE_REF = SourceRef(
    source_type="file",
    source_id="file://test.log",
    raw_offset=None,
    collected_at=_BASE_TIME,
)


def _make_event(
    *,
    fingerprint: str = "fp-aaa",
    severity: Severity = Severity.INFO,
    message: str = "test event",
    service_id: str = "test-svc",
    event_id: str = "evt-001",
    timestamp: datetime = _BASE_TIME,
    tags: frozenset[str] = frozenset(),
) -> NormalizedEvent:
    return NormalizedEvent(
        event_id=event_id,
        timestamp=timestamp,
        timestamp_source="parsed",
        service_id=service_id,
        host_id="host-01",
        severity=severity,
        event_type=EventType.LOG,
        message=message,
        structured_data={},
        tags=tags,
        fingerprint=fingerprint,
        quality=_QUALITY,
        source_ref=_SOURCE_REF,
        schema_version=1,
    )


class TestDedupTrackerSlidingWindow:
    def test_first_event_is_stored(self) -> None:
        tracker = DedupTracker(DeduplicationConfig(window_seconds=60))
        event = _make_event()
        result = tracker.check(event, now=_BASE_TIME)
        assert result.decision == "store"
        assert result.event is event

    def test_duplicate_is_suppressed(self) -> None:
        tracker = DedupTracker(DeduplicationConfig(window_seconds=60))
        e1 = _make_event(event_id="evt-001")
        e2 = _make_event(event_id="evt-002", timestamp=_BASE_TIME + timedelta(seconds=1))
        tracker.check(e1, now=_BASE_TIME)
        result = tracker.check(e2, now=_BASE_TIME + timedelta(seconds=1))
        assert result.decision == "suppress"
        assert result.event is None

    def test_1000_identical_info_collapse_to_first_plus_summaries(self) -> None:
        config = DeduplicationConfig(
            window_seconds=120,
            periodic_summary_interval_seconds=30,
        )
        tracker = DedupTracker(config)
        stored_ids: list[str] = []
        summary_events: list[NormalizedEvent] = []

        for i in range(1000):
            event = _make_event(
                event_id=f"evt-{i:04d}",
                timestamp=_BASE_TIME + timedelta(seconds=i * 0.06),
            )
            result = tracker.check(event, now=_BASE_TIME + timedelta(seconds=i * 0.06))
            if result.decision == "store":
                stored_ids.append(event.event_id)
            summary_events.extend(result.summary_events)

        assert stored_ids[0] == "evt-0000"
        assert len(stored_ids) == 1
        assert len(summary_events) >= 1
        for s in summary_events:
            assert "dedup_summary" in s.tags
            assert s.structured_data["fingerprint"] == "fp-aaa"

    def test_different_fingerprints_are_independent(self) -> None:
        tracker = DedupTracker(DeduplicationConfig(window_seconds=60))
        e1 = _make_event(fingerprint="fp-aaa", event_id="evt-001")
        e2 = _make_event(fingerprint="fp-bbb", event_id="evt-002")
        r1 = tracker.check(e1, now=_BASE_TIME)
        r2 = tracker.check(e2, now=_BASE_TIME)
        assert r1.decision == "store"
        assert r2.decision == "store"

    def test_window_expiry_emits_summary(self) -> None:
        config = DeduplicationConfig(window_seconds=10)
        tracker = DedupTracker(config)
        for i in range(5):
            tracker.check(
                _make_event(event_id=f"evt-{i}", timestamp=_BASE_TIME + timedelta(seconds=i)),
                now=_BASE_TIME + timedelta(seconds=i),
            )
        summaries = tracker.flush(now=_BASE_TIME + timedelta(seconds=20))
        assert len(summaries) == 1
        assert summaries[0].structured_data["count"] == 5
        assert summaries[0].structured_data["suppressed"] == 4


class TestDedupSeverityEscalation:
    def test_severity_escalation_splits_stream(self) -> None:
        tracker = DedupTracker(DeduplicationConfig(window_seconds=60, severity_escalation_splits=True))
        info = _make_event(severity=Severity.INFO, event_id="evt-info")
        warn = _make_event(severity=Severity.WARN, event_id="evt-warn", timestamp=_BASE_TIME + timedelta(seconds=1))
        error = _make_event(severity=Severity.ERROR, event_id="evt-err", timestamp=_BASE_TIME + timedelta(seconds=2))

        r1 = tracker.check(info, now=_BASE_TIME)
        r2 = tracker.check(warn, now=_BASE_TIME + timedelta(seconds=1))
        r3 = tracker.check(error, now=_BASE_TIME + timedelta(seconds=2))

        assert r1.decision == "store"
        assert r2.decision == "store"
        assert r3.decision == "store"

    def test_same_severity_does_not_split(self) -> None:
        tracker = DedupTracker(DeduplicationConfig(window_seconds=60, severity_escalation_splits=True))
        e1 = _make_event(severity=Severity.INFO, event_id="evt-1")
        e2 = _make_event(severity=Severity.INFO, event_id="evt-2", timestamp=_BASE_TIME + timedelta(seconds=1))
        tracker.check(e1, now=_BASE_TIME)
        r = tracker.check(e2, now=_BASE_TIME + timedelta(seconds=1))
        assert r.decision == "suppress"

    def test_escalation_emits_summary_for_prior_window(self) -> None:
        tracker = DedupTracker(DeduplicationConfig(window_seconds=60, severity_escalation_splits=True))
        for i in range(5):
            tracker.check(
                _make_event(severity=Severity.INFO, event_id=f"evt-{i}", timestamp=_BASE_TIME + timedelta(seconds=i)),
                now=_BASE_TIME + timedelta(seconds=i),
            )
        r = tracker.check(
            _make_event(severity=Severity.WARN, event_id="evt-escalation", timestamp=_BASE_TIME + timedelta(seconds=5)),
            now=_BASE_TIME + timedelta(seconds=5),
        )
        assert r.decision == "store"
        assert len(r.summary_events) >= 1
        summary = r.summary_events[0]
        assert summary.structured_data["count"] == 5
        assert summary.structured_data["suppressed"] == 4


class TestDedupLRUEviction:
    def test_lru_eviction_at_capacity(self) -> None:
        config = DeduplicationConfig(max_tracked_fingerprints=3, window_seconds=600)
        tracker = DedupTracker(config)
        for i in range(4):
            tracker.check(
                _make_event(fingerprint=f"fp-{i}", event_id=f"evt-{i}", timestamp=_BASE_TIME + timedelta(seconds=i)),
                now=_BASE_TIME + timedelta(seconds=i),
            )

        stats = tracker.stats()
        assert stats.tracked_fingerprints == 3
        assert stats.evictions == 1

    def test_recently_used_fingerprint_survives_eviction(self) -> None:
        config = DeduplicationConfig(max_tracked_fingerprints=3, window_seconds=600)
        tracker = DedupTracker(config)
        t = _BASE_TIME
        tracker.check(_make_event(fingerprint="fp-0", event_id="evt-0", timestamp=t), now=t)
        tracker.check(_make_event(fingerprint="fp-1", event_id="evt-1", timestamp=t + timedelta(seconds=1)), now=t + timedelta(seconds=1))
        tracker.check(_make_event(fingerprint="fp-2", event_id="evt-2", timestamp=t + timedelta(seconds=2)), now=t + timedelta(seconds=2))
        tracker.check(_make_event(fingerprint="fp-0", event_id="evt-0b", timestamp=t + timedelta(seconds=3)), now=t + timedelta(seconds=3))
        tracker.check(_make_event(fingerprint="fp-3", event_id="evt-3", timestamp=t + timedelta(seconds=4)), now=t + timedelta(seconds=4))

        r = tracker.check(_make_event(fingerprint="fp-0", event_id="evt-0c", timestamp=t + timedelta(seconds=5)), now=t + timedelta(seconds=5))
        assert r.decision == "suppress"


class TestDedupPeriodicSummary:
    def test_periodic_summary_emitted_at_interval(self) -> None:
        config = DeduplicationConfig(window_seconds=300, periodic_summary_interval_seconds=10)
        tracker = DedupTracker(config)
        for i in range(20):
            tracker.check(
                _make_event(event_id=f"evt-{i}", timestamp=_BASE_TIME + timedelta(seconds=i)),
                now=_BASE_TIME + timedelta(seconds=i),
            )

        summaries = tracker.flush(now=_BASE_TIME + timedelta(seconds=30))
        assert len(summaries) >= 1
        for s in summaries:
            assert s.structured_data["_dedup_summary"] is True


class TestNoiseFilterBlocklist:
    def test_blocklist_filters_matching_info_event(self) -> None:
        config = NoiseFilterConfig(
            blocklist=[NoiseBlocklistConfig(pattern="health check passed", severity_max="INFO", reason="routine")],
        )
        nf = NoiseFilter(config)
        event = _make_event(severity=Severity.INFO, message="health check passed for api-gateway")
        assert nf.should_store(event) is False

    def test_blocklist_does_not_filter_above_severity_max(self) -> None:
        config = NoiseFilterConfig(
            blocklist=[NoiseBlocklistConfig(pattern="health check passed", severity_max="INFO", reason="routine")],
        )
        nf = NoiseFilter(config)
        event = _make_event(severity=Severity.WARN, message="health check passed for api-gateway")
        assert nf.should_store(event) is True

    def test_blocklist_service_id_scoping(self) -> None:
        config = NoiseFilterConfig(
            blocklist=[NoiseBlocklistConfig(pattern="heartbeat", severity_max="INFO", service_id="monitor")],
        )
        nf = NoiseFilter(config)
        e_match = _make_event(severity=Severity.INFO, message="heartbeat ok", service_id="monitor")
        e_other = _make_event(severity=Severity.INFO, message="heartbeat ok", service_id="api")
        assert nf.should_store(e_match) is False
        assert nf.should_store(e_other) is True


class TestNoiseFilterAllowlist:
    def test_allowlist_overrides_blocklist(self) -> None:
        config = NoiseFilterConfig(
            blocklist=[NoiseBlocklistConfig(pattern="out of memory", severity_max="ERROR", reason="test")],
            allowlist=[NoiseAllowlistConfig(pattern="out of memory", tags=["oom"], reason="resource failure")],
        )
        nf = NoiseFilter(config)
        event = _make_event(severity=Severity.ERROR, message="out of memory", tags=frozenset({"oom"}))
        assert nf.should_store(event) is True

    def test_allowlist_always_wins(self) -> None:
        config = NoiseFilterConfig(
            blocklist=[NoiseBlocklistConfig(pattern="crash", severity_max="CRITICAL", reason="test")],
            allowlist=[NoiseAllowlistConfig(pattern="crash detected", reason="critical event")],
        )
        nf = NoiseFilter(config)
        event = _make_event(severity=Severity.INFO, message="crash detected in worker-3")
        assert nf.should_store(event) is True


class TestNoiseFilterErrorNeverDemoted:
    def test_error_severity_always_stored(self) -> None:
        config = NoiseFilterConfig(
            blocklist=[NoiseBlocklistConfig(pattern=".*", severity_max="CRITICAL", reason="block everything")],
            adaptive_enabled=True,
            always_keep_severity="ERROR",
        )
        nf = NoiseFilter(config)
        for sev in (Severity.ERROR, Severity.CRITICAL):
            event = _make_event(severity=sev, message="anything at all")
            assert nf.should_store(event) is True

    def test_warn_not_demoted_by_adaptive(self) -> None:
        config = NoiseFilterConfig(
            adaptive_enabled=True,
            always_keep_severity="WARN",
            high_rate_threshold_per_minute=1,
            stability_threshold_cv=10.0,
        )
        nf = NoiseFilter(config)
        event = _make_event(severity=Severity.WARN, message="warn level event")
        for _ in range(200):
            nf.record_event(event)
        assert nf.should_store(event) is True


class TestNoiseFilterAdaptive:
    def test_high_rate_stable_fingerprint_is_demoted(self) -> None:
        config = NoiseFilterConfig(
            adaptive_enabled=True,
            high_rate_threshold_per_minute=10,
            stability_threshold_cv=0.5,
            routine_sample_target_per_minute=1,
            frequency_window_minutes=5,
            always_keep_severity="ERROR",
        )
        nf = NoiseFilter(config)
        event = _make_event(severity=Severity.INFO, message="periodic heartbeat", fingerprint="fp-heartbeat")

        from normalization.noise import _FingerprintBucket

        bucket = _FingerprintBucket()
        base = time.monotonic()
        for i in range(300):
            bucket.timestamps.append(base + i * 0.01)
        nf._buckets[event.fingerprint] = bucket

        stored_count = sum(1 for _ in range(100) if nf.should_store(event))
        assert stored_count < 100

    def test_info_event_below_rate_threshold_is_not_demoted(self) -> None:
        config = NoiseFilterConfig(
            adaptive_enabled=True,
            high_rate_threshold_per_minute=1000,
            stability_threshold_cv=0.2,
        )
        nf = NoiseFilter(config)
        event = _make_event(severity=Severity.INFO, message="occasional log line")
        for _ in range(5):
            nf.record_event(event)
        assert nf.should_store(event) is True


class TestNoiseFilterAnnotation:
    def test_annotate_adds_noise_score(self) -> None:
        nf = NoiseFilter(NoiseFilterConfig())
        event = _make_event(severity=Severity.ERROR, tags=frozenset({"crash"}))
        annotated = nf.annotate(event)
        assert "_noise_score" in annotated.structured_data
        assert annotated.structured_data["_noise_score"] == 1.0

    def test_annotate_marks_routine(self) -> None:
        config = NoiseFilterConfig(
            adaptive_enabled=True,
            high_rate_threshold_per_minute=5,
            stability_threshold_cv=10.0,
            always_keep_severity="ERROR",
        )
        nf = NoiseFilter(config)
        event = _make_event(severity=Severity.INFO, message="heartbeat", fingerprint="fp-hb")

        from normalization.noise import _FingerprintBucket

        bucket = _FingerprintBucket()
        base = time.monotonic()
        for i in range(200):
            bucket.timestamps.append(base + i * 0.01)
        nf._buckets[event.fingerprint] = bucket

        nf.should_store(event)
        annotated = nf.annotate(event)
        assert annotated.structured_data.get("_noise_routine") is True


class TestNoiseRegistryPersistence:
    def test_registry_round_trip(self, tmp_path) -> None:
        config = NoiseFilterConfig(
            registry_enabled=True,
            registry_expiry_days=14,
            adaptive_enabled=True,
            high_rate_threshold_per_minute=5,
            stability_threshold_cv=10.0,
            always_keep_severity="ERROR",
        )
        nf = NoiseFilter(config, data_dir=tmp_path)
        event = _make_event(severity=Severity.INFO, message="routine ping", fingerprint="fp-ping")

        from normalization.noise import _FingerprintBucket

        bucket = _FingerprintBucket()
        base = time.monotonic()
        for i in range(200):
            bucket.timestamps.append(base + i * 0.01)
        nf._buckets[event.fingerprint] = bucket

        nf.should_store(event)
        nf.persist_registry()

        registry_path = tmp_path / "noise_registry.json"
        assert registry_path.exists()
        entries = json.loads(registry_path.read_text())
        assert len(entries) >= 1
        assert entries[0]["fingerprint"] == "fp-ping"

        nf2 = NoiseFilter(config, data_dir=tmp_path)
        assert nf2.stats().routine_fingerprints >= 1


class TestNoiseStats:
    def test_stats_accumulate(self) -> None:
        config = NoiseFilterConfig(
            blocklist=[NoiseBlocklistConfig(pattern="heartbeat", severity_max="INFO", reason="noise")],
            allowlist=[NoiseAllowlistConfig(pattern="crash", reason="keep")],
        )
        nf = NoiseFilter(config)
        nf.should_store(_make_event(severity=Severity.INFO, message="heartbeat ok"))
        nf.should_store(_make_event(severity=Severity.ERROR, message="crash detected"))
        stats = nf.stats()
        assert stats.blocklist_hits == 1
        assert stats.allowlist_hits == 1
        assert stats.total_filtered == 1


class TestDeterminism:
    def test_same_fixture_same_stored_ids_across_100_runs(self) -> None:
        reference_ids: list[str] | None = None
        for run_index in range(100):
            config = DeduplicationConfig(
                window_seconds=120,
                periodic_summary_interval_seconds=30,
                severity_escalation_splits=True,
            )
            tracker = DedupTracker(config)
            stored: list[str] = []
            events = [
                _make_event(
                    event_id=f"evt-{i:04d}",
                    fingerprint="fp-stable",
                    severity=Severity.INFO,
                    timestamp=_BASE_TIME + timedelta(seconds=i),
                )
                for i in range(50)
            ]
            events.insert(
                25,
                _make_event(
                    event_id="evt-escalate",
                    fingerprint="fp-stable",
                    severity=Severity.WARN,
                    timestamp=_BASE_TIME + timedelta(seconds=25),
                ),
            )
            for event in events:
                result = tracker.check(event, now=event.timestamp)
                if result.decision == "store":
                    stored.append(event.event_id)
                for s in result.summary_events:
                    stored.append(s.event_id)

            non_summary_stored = [eid for eid in stored if not eid.startswith("evt-") or eid in ("evt-0000", "evt-escalate")]
            if reference_ids is None:
                reference_ids = non_summary_stored
            else:
                assert non_summary_stored == reference_ids, f"Run {run_index} diverged"

    def test_noise_filter_deterministic_across_runs(self) -> None:
        config = NoiseFilterConfig(
            blocklist=[NoiseBlocklistConfig(pattern="health check passed", severity_max="INFO")],
            allowlist=[NoiseAllowlistConfig(pattern="out of memory", tags=["oom"])],
            always_keep_severity="ERROR",
        )
        events = [
            _make_event(severity=Severity.INFO, message="health check passed", event_id=f"e-{i}")
            for i in range(10)
        ] + [
            _make_event(severity=Severity.ERROR, message="out of memory", event_id=f"e-err-{i}", tags=frozenset({"oom"}))
            for i in range(5)
        ] + [
            _make_event(severity=Severity.INFO, message="normal log line", event_id=f"e-norm-{i}")
            for i in range(10)
        ]

        reference: list[bool] | None = None
        for _ in range(100):
            nf = NoiseFilter(config)
            results = [nf.should_store(e) for e in events]
            if reference is None:
                reference = results
            else:
                assert results == reference
