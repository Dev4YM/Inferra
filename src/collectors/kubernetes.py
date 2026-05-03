from __future__ import annotations

import asyncio
import json
from typing import Any

from collectors.base import CollectorHealth
from core.time import to_iso, utc_now
from events.models import RawEvent


class KubernetesCollector:
    source_type = "kubernetes"

    def __init__(
        self,
        namespaces: tuple[str, ...] = (),
        all_namespaces: bool = True,
        limit: int = 200,
        include_pods: bool = True,
        include_events: bool = True,
        poll_interval_seconds: float = 15.0,
        core_api: Any | None = None,
    ) -> None:
        self.namespaces = namespaces
        self.all_namespaces = all_namespaces
        self.limit = limit
        self.include_pods = include_pods
        self.include_events = include_events
        self.poll_interval_seconds = poll_interval_seconds
        self.core_api = core_api
        self._running = False
        self._events = 0
        self._errors = 0
        self._last_error: str | None = None
        self._last_event_at = None

    @property
    def collector_id(self) -> str:
        scope = "all" if self.all_namespaces else ",".join(self.namespaces or ("default",))
        return f"kubernetes://{scope}"

    async def start(self, sink: asyncio.Queue[RawEvent]) -> None:
        self._running = True
        try:
            while self._running:
                await self.collect_once(sink)
                await asyncio.sleep(self.poll_interval_seconds)
        finally:
            self._running = False

    async def collect_once(self, sink: asyncio.Queue[RawEvent]) -> int:
        try:
            api = self.core_api or self._load_core_api()
            emitted = 0
            if self.include_events:
                for item in self._read_events(api):
                    await sink.put(self._raw_from_event(item))
                    emitted += 1
            if self.include_pods:
                for pod in self._read_pods(api):
                    if self._pod_is_noteworthy(pod):
                        await sink.put(self._raw_from_pod(pod))
                        emitted += 1
            self._events += emitted
            if emitted:
                self._last_event_at = utc_now()
            return emitted
        except Exception as exc:  # pragma: no cover
            self._errors += 1
            self._last_error = str(exc)
            return 0

    async def stop(self) -> None:
        self._running = False

    def health_check(self) -> CollectorHealth:
        return CollectorHealth(
            collector_id=self.collector_id,
            source_type=self.source_type,
            is_running=self._running,
            events_emitted=self._events,
            last_event_at=self._last_event_at,
            error_count=self._errors,
            last_error=self._last_error,
        )

    def _load_core_api(self) -> Any:
        try:
            from kubernetes import client, config
        except ImportError as exc:
            raise RuntimeError("kubernetes optional dependency is not installed") from exc
        try:
            config.load_incluster_config()
        except Exception:
            config.load_kube_config()
        return client.CoreV1Api()

    def _read_events(self, api: Any) -> list[Any]:
        if self.all_namespaces:
            return list(api.list_event_for_all_namespaces(limit=self.limit).items)
        items: list[Any] = []
        for namespace in self.namespaces or ("default",):
            items.extend(api.list_namespaced_event(namespace=namespace, limit=self.limit).items)
        return items

    def _read_pods(self, api: Any) -> list[Any]:
        if self.all_namespaces:
            return list(api.list_pod_for_all_namespaces(limit=self.limit).items)
        items: list[Any] = []
        for namespace in self.namespaces or ("default",):
            items.extend(api.list_namespaced_pod(namespace=namespace, limit=self.limit).items)
        return items

    def _raw_from_event(self, item: Any) -> RawEvent:
        involved = getattr(item, "involved_object", None)
        metadata = getattr(item, "metadata", None)
        namespace = getattr(metadata, "namespace", None) or getattr(involved, "namespace", None)
        object_kind = getattr(involved, "kind", None)
        object_name = getattr(involved, "name", None)
        service = object_name or getattr(item, "source", None) or "kubernetes"
        event_type = str(getattr(item, "type", None) or "Normal")
        level = "warn" if event_type.lower() == "warning" else "info"
        reason = str(getattr(item, "reason", None) or "Event")
        message = str(getattr(item, "message", None) or reason)
        timestamp = _event_timestamp(item)
        payload = {
            "timestamp": to_iso(timestamp) if timestamp else None,
            "level": level,
            "service": service,
            "host": getattr(getattr(item, "source", None), "host", None),
            "message": f"kubernetes {event_type.lower()} {reason}: {message}",
            "kubernetes": {
                "kind": "Event",
                "namespace": namespace,
                "reason": reason,
                "type": event_type,
                "count": getattr(item, "count", None),
                "involved_object": {"kind": object_kind, "name": object_name},
            },
        }
        return RawEvent(
            source_type=self.source_type,
            source_id=self.collector_id,
            raw_payload=json.dumps(payload, sort_keys=True),
            collected_at=utc_now(),
            metadata={"namespace": namespace, "workload": service, "kind": object_kind, "reason": reason},
        )

    def _raw_from_pod(self, pod: Any) -> RawEvent:
        metadata = getattr(pod, "metadata", None)
        status = getattr(pod, "status", None)
        spec = getattr(pod, "spec", None)
        labels = dict(getattr(metadata, "labels", None) or {})
        name = str(getattr(metadata, "name", None) or "pod")
        namespace = getattr(metadata, "namespace", None)
        service = labels.get("app.kubernetes.io/name") or labels.get("app") or name
        phase = str(getattr(status, "phase", None) or "Unknown")
        restarts = _restart_count(status)
        ready = _pod_ready(status)
        node = getattr(spec, "node_name", None)
        level = "warn" if phase != "Running" or not ready or restarts > 0 else "info"
        message = f"kubernetes pod {name} phase={phase} ready={ready} restarts={restarts}"
        payload = {
            "timestamp": to_iso(utc_now()),
            "level": level,
            "service": service,
            "host": node,
            "message": message,
            "kubernetes": {
                "kind": "Pod",
                "namespace": namespace,
                "name": name,
                "phase": phase,
                "ready": ready,
                "restart_count": restarts,
                "node": node,
                "labels": labels,
            },
        }
        return RawEvent(
            source_type=self.source_type,
            source_id=self.collector_id,
            raw_payload=json.dumps(payload, sort_keys=True),
            collected_at=utc_now(),
            metadata={"namespace": namespace, "workload": service, "kind": "Pod", "pod": name, "node": node},
        )

    def _pod_is_noteworthy(self, pod: Any) -> bool:
        status = getattr(pod, "status", None)
        phase = str(getattr(status, "phase", None) or "Unknown")
        return phase != "Running" or not _pod_ready(status) or _restart_count(status) > 0


def _event_timestamp(item: Any) -> Any:
    for attr in ("event_time", "last_timestamp", "first_timestamp"):
        value = getattr(item, attr, None)
        if value is not None:
            return value
    return None


def _restart_count(status: Any) -> int:
    total = 0
    for container in getattr(status, "container_statuses", None) or []:
        total += int(getattr(container, "restart_count", 0) or 0)
    return total


def _pod_ready(status: Any) -> bool:
    conditions = getattr(status, "conditions", None) or []
    for condition in conditions:
        if getattr(condition, "type", None) == "Ready":
            return str(getattr(condition, "status", "")).lower() == "true"
    return False
