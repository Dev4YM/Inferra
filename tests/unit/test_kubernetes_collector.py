import asyncio
from datetime import UTC, datetime
from types import SimpleNamespace

from collectors.kubernetes import KubernetesCollector
from core.enums import EventType, Severity
from normalization.pipeline import NormalizationPipeline


class FakeCoreApi:
    def __init__(self, events=None, pods=None):
        self.events = events or []
        self.pods = pods or []

    def list_event_for_all_namespaces(self, limit, label_selector=None):
        return SimpleNamespace(items=self.events[:limit])

    def list_namespaced_event(self, namespace, limit, label_selector=None):
        return SimpleNamespace(items=[item for item in self.events if item.metadata.namespace == namespace][:limit])

    def list_pod_for_all_namespaces(self, limit, label_selector=None):
        return SimpleNamespace(items=self.pods[:limit])

    def list_namespaced_pod(self, namespace, limit, label_selector=None):
        return SimpleNamespace(items=[item for item in self.pods if item.metadata.namespace == namespace][:limit])


def test_kubernetes_collector_normalizes_warning_events():
    event = SimpleNamespace(
        metadata=SimpleNamespace(namespace="prod", name="api.1"),
        involved_object=SimpleNamespace(kind="Pod", name="api-123", namespace="prod"),
        type="Warning",
        reason="BackOff",
        message="Back-off restarting failed container",
        count=3,
        last_timestamp=datetime(2026, 5, 3, tzinfo=UTC),
        source=SimpleNamespace(host="node-a"),
    )
    collector = KubernetesCollector(core_api=FakeCoreApi(events=[event], pods=[]), include_pods=False)
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_once(queue))
    normalized = NormalizationPipeline().normalize(queue.get_nowait())

    assert emitted == 1
    assert normalized.service_id == "api"
    assert normalized.severity == Severity.WARN
    assert normalized.event_type == EventType.STATE_CHANGE
    assert "kubernetes" in normalized.tags


def test_kubernetes_collector_emits_unhealthy_pod_snapshots():
    pod = SimpleNamespace(
        metadata=SimpleNamespace(namespace="prod", name="api-123", labels={"app": "api"}),
        spec=SimpleNamespace(node_name="node-a"),
        status=SimpleNamespace(
            phase="Running",
            conditions=[SimpleNamespace(type="Ready", status="False")],
            container_statuses=[SimpleNamespace(restart_count=2)],
        ),
    )
    collector = KubernetesCollector(core_api=FakeCoreApi(events=[], pods=[pod]), include_events=False)
    queue = asyncio.Queue()

    emitted = asyncio.run(collector.collect_once(queue))
    normalized = NormalizationPipeline().normalize(queue.get_nowait())

    assert emitted == 1
    assert normalized.service_id == "api"
    assert normalized.host_id == "node-a"
    assert normalized.severity == Severity.WARN
    assert normalized.structured_data["kubernetes"]["restart_count"] == 2


def test_kubernetes_collector_skips_healthy_pods():
    pod = SimpleNamespace(
        metadata=SimpleNamespace(namespace="prod", name="api-123", labels={"app": "api"}),
        spec=SimpleNamespace(node_name="node-a"),
        status=SimpleNamespace(
            phase="Running",
            conditions=[SimpleNamespace(type="Ready", status="True")],
            container_statuses=[SimpleNamespace(restart_count=0)],
        ),
    )
    collector = KubernetesCollector(core_api=FakeCoreApi(events=[], pods=[pod]), include_events=False)

    emitted = asyncio.run(collector.collect_once(asyncio.Queue()))

    assert emitted == 0
