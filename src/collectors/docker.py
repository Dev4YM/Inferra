from __future__ import annotations

import asyncio
import fnmatch
import json
from dataclasses import dataclass
from typing import Any

from collectors.base import Collector
from core.time import to_iso, utc_now
from events.models import RawEvent


@dataclass(frozen=True)
class DockerContainer:
    container_id: str
    name: str
    image: str | None
    labels: dict[str, str]


class DockerCollector(Collector):
    source_type = "docker"

    def __init__(
        self,
        socket: str = "/var/run/docker.sock",
        include_names: tuple[str, ...] = (),
        include_labels: tuple[str, ...] = (),
        exclude_names: tuple[str, ...] = (),
        include_all: bool = True,
        state_store=None,
        emit_timeout_seconds: float = 1.0,
        api_client: Any | None = None,
    ) -> None:
        super().__init__(state_store=state_store, emit_timeout_seconds=emit_timeout_seconds)
        self.socket = socket
        self.include_names = include_names
        self.include_labels = include_labels
        self.exclude_names = exclude_names
        self.include_all = include_all
        self.api_client = api_client
        self._log_tasks: dict[str, asyncio.Task[None]] = {}
        self._background_tasks: list[asyncio.Task[None]] = []

    @property
    def collector_id(self) -> str:
        return f"docker://{self.socket}"

    async def run(self, queue: asyncio.Queue[RawEvent]) -> None:
        await self._mark_running()
        try:
            client = self.api_client or _DockerApiClient(self.socket)
            self._background_tasks = [
                asyncio.create_task(self._stream_events(client, queue), name="inferra:docker-events"),
                asyncio.create_task(self._manage_log_streams(client, queue), name="inferra:docker-logs"),
            ]
            await asyncio.gather(*self._background_tasks)
        except asyncio.CancelledError:
            raise
        except Exception as exc:  # pragma: no cover
            self._record_error(exc)
        finally:
            for task in self._background_tasks:
                task.cancel()
            await asyncio.gather(*self._background_tasks, return_exceptions=True)
            for task in self._log_tasks.values():
                task.cancel()
            await asyncio.gather(*self._log_tasks.values(), return_exceptions=True)
            if self.api_client is None and "client" in locals():
                await client.close()
            await self._mark_stopped()

    async def stop(self) -> None:
        await super().stop()
        for task in self._log_tasks.values():
            task.cancel()
        for task in self._background_tasks:
            task.cancel()

    async def _stream_events(self, client: Any, queue: asyncio.Queue[RawEvent]) -> None:
        since = self.checkpoint_load("events.since")
        async for payload in client.stream_events(since=since):
            if self._should_stop():
                return
            container = _container_from_event(payload)
            if container is not None and not self._matches_container(container):
                continue
            observed_at = utc_now()
            raw = RawEvent(
                source_type=self.source_type,
                source_id=self.collector_id,
                raw_payload=json.dumps(payload, sort_keys=True),
                collected_at=observed_at,
                metadata={
                    "kind": "event",
                    "container_id": payload.get("id"),
                    "status": payload.get("status"),
                    "type": payload.get("Type"),
                },
            )
            await self.emit(queue, raw)
            event_time = _string(payload.get("timeNano") or payload.get("time"))
            if event_time:
                self.checkpoint_save("events.since", event_time)

    async def _manage_log_streams(self, client: Any, queue: asyncio.Queue[RawEvent]) -> None:
        while not self._should_stop():
            containers = await client.list_containers()
            matched = {item.container_id: item for item in containers if self._matches_container(item)}
            for container_id, task in list(self._log_tasks.items()):
                if container_id not in matched:
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)
                    del self._log_tasks[container_id]
            for container in matched.values():
                if container.container_id in self._log_tasks and not self._log_tasks[container.container_id].done():
                    continue
                self._log_tasks[container.container_id] = asyncio.create_task(
                    self._stream_container_logs(client, container, queue),
                    name=f"inferra:docker-log:{container.container_id[:12]}",
                )
            try:
                await asyncio.wait_for(self._stop_event.wait(), timeout=10.0)
            except TimeoutError:
                continue

    async def _stream_container_logs(self, client: Any, container: DockerContainer, queue: asyncio.Queue[RawEvent]) -> None:
        since_key = f"log.{container.container_id}.since"
        since = self.checkpoint_load(since_key)
        try:
            async for line in client.stream_logs(container.container_id, since=since):
                if self._should_stop():
                    return
                observed_at = utc_now()
                raw = RawEvent(
                    source_type=self.source_type,
                    source_id=f"{self.collector_id}/{container.container_id}",
                    raw_payload=line,
                    collected_at=observed_at,
                    metadata={
                        "kind": "log",
                        "container_id": container.container_id,
                        "container_name": container.name,
                        "image": container.image,
                    },
                )
                await self.emit(queue, raw)
                self.checkpoint_save(since_key, to_iso(observed_at))
        except asyncio.CancelledError:
            raise
        except Exception as exc:  # pragma: no cover
            self._record_error(f"{container.name}: {exc}")

    def _matches_container(self, container: DockerContainer) -> bool:
        if any(fnmatch.fnmatch(container.name, pattern) for pattern in self.exclude_names):
            return False
        if self.include_names and not any(fnmatch.fnmatch(container.name, pattern) for pattern in self.include_names):
            return False
        if self.include_labels and not all(_label_matches(container.labels, value) for value in self.include_labels):
            return False
        return self.include_all or bool(self.include_names) or bool(self.include_labels)


class _DockerApiClient:
    def __init__(self, socket: str) -> None:
        self.socket = socket
        self._session = None

    async def close(self) -> None:
        if self._session is not None:
            await self._session.close()

    async def list_containers(self) -> list[DockerContainer]:
        payload = await self._get_json("/containers/json")
        containers: list[DockerContainer] = []
        for item in payload:
            names = item.get("Names") or []
            name = str(names[0]).lstrip("/") if names else str(item.get("Id", ""))[:12]
            containers.append(
                DockerContainer(
                    container_id=str(item.get("Id") or ""),
                    name=name,
                    image=_string(item.get("Image")),
                    labels={str(key): str(value) for key, value in (item.get("Labels") or {}).items()},
                )
            )
        return containers

    async def stream_events(self, since: str | None = None):
        params = {"since": since} if since else None
        async with await self._request("GET", "/events", params=params) as response:
            async for line in response.content:
                text = line.decode("utf-8", errors="replace").strip()
                if text:
                    yield json.loads(text)

    async def stream_logs(self, container_id: str, since: str | None = None):
        params = {
            "follow": "1",
            "stdout": "1",
            "stderr": "1",
            "timestamps": "1",
            "tail": "0",
        }
        if since:
            params["since"] = since
        async with await self._request("GET", f"/containers/{container_id}/logs", params=params) as response:
            async for line in response.content:
                text = line.decode("utf-8", errors="replace").rstrip("\r\n")
                if text:
                    yield text

    async def _get_json(self, path: str) -> list[dict[str, Any]]:
        async with await self._request("GET", path) as response:
            return await response.json()

    async def _request(self, method: str, path: str, params: dict[str, str] | None = None):
        session = await self._session_for_socket()
        response = await session.request(method, path, params=params)
        response.raise_for_status()
        return response

    async def _session_for_socket(self):
        if self._session is not None:
            return self._session
        try:
            import aiohttp
        except ImportError as exc:  # pragma: no cover
            raise RuntimeError("aiohttp is required for the Docker collector") from exc
        if self.socket.startswith("tcp://"):
            base_url = "http://" + self.socket.removeprefix("tcp://")
            self._session = aiohttp.ClientSession(base_url=base_url)
            return self._session
        if self.socket.startswith("http://") or self.socket.startswith("https://"):
            self._session = aiohttp.ClientSession(base_url=self.socket.rstrip("/"))
            return self._session
        socket_path = self.socket.removeprefix("unix://")
        connector = aiohttp.UnixConnector(path=socket_path)
        self._session = aiohttp.ClientSession(connector=connector, base_url="http://docker")
        return self._session


def _container_from_event(payload: dict[str, Any]) -> DockerContainer | None:
    actor = payload.get("Actor") or {}
    attributes = actor.get("Attributes") or {}
    container_id = _string(payload.get("id")) or _string(actor.get("ID"))
    if not container_id:
        return None
    name = _string(attributes.get("name")) or container_id[:12]
    image = _string(attributes.get("image"))
    labels = {str(key): str(value) for key, value in attributes.items() if key not in {"name", "image"}}
    return DockerContainer(container_id=container_id, name=name, image=image, labels=labels)


def _label_matches(labels: dict[str, str], expression: str) -> bool:
    if "=" in expression:
        key, expected = expression.split("=", 1)
        return labels.get(key) == expected
    return expression in labels


def _string(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None
