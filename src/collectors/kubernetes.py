from __future__ import annotations

import asyncio
import json
from typing import Any

from collectors.base import PollingCollector
from core.time import to_iso, utc_now
from events.models import RawEvent


class KubernetesCollector(PollingCollector):
    source_type = "kubernetes"

    def __init__(
        self,
        namespaces: tuple[str, ...] = (),
        all_namespaces: bool = True,
        limit: int = 200,
        include_pods: bool = True,
        include_events: bool = True,
        label_selector: str | None = None,
        poll_interval_seconds: float = 15.0,
        core_api: Any | None = None,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(poll_interval_seconds=poll_interval_seconds, emit_timeout_seconds=emit_timeout_seconds)
        self.namespaces = namespaces
        self.all_namespaces = all_namespaces
        self.limit = limit
        self.include_pods = include_pods
        self.include_events = include_events
        self.label_selector = label_selector
        self.core_api = core_api
        self._seen_events: set[str] = set()
        self._pod_state: dict[str, tuple[str, bool, int, bool]] = {}

    @property
    def collector_id(self) -> str:
        scope = "all" if self.all_namespaces else ",".join(self.namespaces or ("default",))
        return f"kubernetes://{scope}"

    async def collect_once(self, sink) -> int:
        try:
            api = self.core_api or await asyncio.to_thread(self._load_core_api)
            emitted = 0
            if self.include_events:
                for item in await asyncio.to_thread(self._read_events, api):
                    event_key = _event_key(item)
                    if event_key in self._seen_events:
                        continue
                    self._seen_events.add(event_key)
                    emitted += 1 if await self.emit(sink, self._raw_from_event(item)) else 0
            if self.include_pods:
                for pod in await asyncio.to_thread(self._read_pods, api):
                    if not self._pod_is_noteworthy(pod):
                        continue
                    pod_key = _pod_key(pod)
                    current_state = _pod_state_tuple(pod)
                    previous_state = self._pod_state.get(pod_key)
                    if previous_state == current_state:
                        continue
                    self._pod_state[pod_key] = current_state
                    emitted += 1 if await self.emit(sink, self._raw_from_pod(pod, previous_state)) else 0
            return emitted
        except Exception as exc:  # pragma: no cover
            self._record_error(exc)
            return 0

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
            return list(api.list_event_for_all_namespaces(limit=self.limit, label_selector=self.label_selector).items)
        items: list[Any] = []
        for namespace in self.namespaces or ("default",):
            items.extend(
                api.list_namespaced_event(namespace=namespace, limit=self.limit, label_selector=self.label_selector).items
            )
        return items

    def _read_pods(self, api: Any) -> list[Any]:
        if self.all_namespaces:
            return list(api.list_pod_for_all_namespaces(limit=self.limit, label_selector=self.label_selector).items)
        items: list[Any] = []
        for namespace in self.namespaces or ("default",):
            items.extend(api.list_namespaced_pod(namespace=namespace, limit=self.limit, label_selector=self.label_selector).items)
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

    def _raw_from_pod(self, pod: Any, previous_state: tuple[str, bool, int, bool] | None) -> RawEvent:
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
        oom_killed = _pod_oom_killed(status)
        node = getattr(spec, "node_name", None)
        level = "warn" if phase != "Running" or not ready or restarts > 0 or oom_killed else "info"
        previous_desc = ""
        if previous_state is not None:
            previous_desc = (
                f" previous_phase={previous_state[0]} previous_ready={previous_state[1]} "
                f"previous_restarts={previous_state[2]} previous_oom_killed={previous_state[3]}"
            )
        message = (
            f"kubernetes pod {name} phase={phase} ready={ready} restarts={restarts} oom_killed={oom_killed}"
            f"{previous_desc}"
        )
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
                "oom_killed": oom_killed,
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
        return phase != "Running" or not _pod_ready(status) or _restart_count(status) > 0 or _pod_oom_killed(status)


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


def _pod_oom_killed(status: Any) -> bool:
    for container in getattr(status, "container_statuses", None) or []:
        state = getattr(container, "state", None)
        terminated = getattr(state, "terminated", None)
        reason = str(getattr(terminated, "reason", "") or "")
        if reason == "OOMKilled":
            return True
        last_state = getattr(container, "last_state", None)
        terminated = getattr(last_state, "terminated", None)
        if str(getattr(terminated, "reason", "") or "") == "OOMKilled":
            return True
    return False


def _pod_ready(status: Any) -> bool:
    conditions = getattr(status, "conditions", None) or []
    for condition in conditions:
        if getattr(condition, "type", None) == "Ready":
            return str(getattr(condition, "status", "")).lower() == "true"
    return False


def _event_key(item: Any) -> str:
    metadata = getattr(item, "metadata", None)
    uid = getattr(metadata, "uid", None)
    if uid:
        return str(uid)
    namespace = getattr(metadata, "namespace", None)
    name = getattr(metadata, "name", None)
    reason = getattr(item, "reason", None)
    count = getattr(item, "count", None)
    return f"{namespace}:{name}:{reason}:{count}"


def _pod_key(pod: Any) -> str:
    metadata = getattr(pod, "metadata", None)
    namespace = getattr(metadata, "namespace", None)
    name = getattr(metadata, "name", None)
    return f"{namespace}:{name}"


def _pod_state_tuple(pod: Any) -> tuple[str, bool, int, bool]:
    status = getattr(pod, "status", None)
    phase = str(getattr(status, "phase", None) or "Unknown")
    return (phase, _pod_ready(status), _restart_count(status), _pod_oom_killed(status))
