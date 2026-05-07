from __future__ import annotations

import asyncio
import json
from typing import Any

from fastapi import APIRouter, Body, Header, HTTPException

from collectors.base import Collector
from core.time import utc_now
from events.models import RawEvent


class AppHttpCollector(Collector):
    source_type = "app"

    def __init__(
        self,
        listen: str = "127.0.0.1:9876",
        max_payload_bytes: int = 65536,
        shared_token: str | None = None,
        mount_path: str = "/api/ingest",
        enable_main_api: bool = True,
        enable_standalone: bool = False,
        emit_timeout_seconds: float = 1.0,
    ) -> None:
        super().__init__(emit_timeout_seconds=emit_timeout_seconds)
        self.listen = listen
        self.max_payload_bytes = max_payload_bytes
        self.shared_token = shared_token or None
        self.mount_path = mount_path
        self.enable_main_api = enable_main_api
        self.enable_standalone = enable_standalone
        self._queue: asyncio.Queue[RawEvent] | None = None
        self._server = None

    @property
    def collector_id(self) -> str:
        return f"app://{self.listen}"

    def attach_queue(self, queue: asyncio.Queue[RawEvent]) -> None:
        self._queue = queue

    def router(self) -> APIRouter:
        router = APIRouter()

        @router.post(self.mount_path)
        async def ingest_route(
            payload: dict[str, Any] = Body(...),
            authorization: str | None = Header(default=None),
        ) -> dict[str, Any]:
            stored = await self.ingest_http_payload(payload, authorization=authorization, source_id="app://mounted")
            return {"stored": stored}

        return router

    async def run(self, queue: asyncio.Queue[RawEvent]) -> None:
        self.attach_queue(queue)
        await self._mark_running()
        try:
            if not self.enable_standalone:
                await self._stop_event.wait()
                return
            host, port = self._parse_listen()
            try:
                import uvicorn
            except ImportError as exc:  # pragma: no cover
                raise RuntimeError("uvicorn is required for the standalone app HTTP collector") from exc
            app = self._standalone_app()
            self._server = uvicorn.Server(uvicorn.Config(app, host=host, port=port, log_config=None))
            await self._server.serve()
        finally:
            await self._mark_stopped()

    async def stop(self) -> None:
        await super().stop()
        if self._server is not None:
            self._server.should_exit = True

    async def ingest_http_payload(
        self,
        payload: dict[str, Any],
        *,
        authorization: str | None,
        source_id: str,
    ) -> bool:
        self._authorize(authorization)
        queue = self._queue
        if queue is None:
            raise HTTPException(status_code=503, detail="App HTTP collector queue is not attached")
        message = str(payload.get("message") or "").strip()
        if not message:
            raise HTTPException(status_code=400, detail="'message' is required")
        raw_payload = json.dumps(
            {
                "timestamp": payload.get("timestamp"),
                "service": payload.get("service", "app"),
                "level": payload.get("level", "info"),
                "message": message,
                "context": payload.get("context", {}),
            },
            sort_keys=True,
        )
        if len(raw_payload.encode("utf-8")) > self.max_payload_bytes:
            raise HTTPException(status_code=413, detail="payload exceeds max_payload_bytes")
        raw = RawEvent(
            source_type=self.source_type,
            source_id=source_id,
            raw_payload=raw_payload,
            collected_at=utc_now(),
            metadata={"service_id": payload.get("service", "app")},
        )
        return await self.emit(queue, raw)

    def _authorize(self, authorization: str | None) -> None:
        if self.shared_token is None:
            return
        if not authorization:
            raise HTTPException(status_code=401, detail="missing Authorization header")
        scheme, _, token = authorization.partition(" ")
        candidate = token if token else authorization
        if scheme.lower() == "bearer" and token == self.shared_token:
            return
        if candidate == self.shared_token:
            return
        raise HTTPException(status_code=401, detail="invalid shared token")

    def _parse_listen(self) -> tuple[str, int]:
        host, _, raw_port = self.listen.rpartition(":")
        if not host or not raw_port:
            raise RuntimeError(f"Invalid collectors.app.listen value: {self.listen}")
        return host, int(raw_port)

    def _standalone_app(self):
        from fastapi import FastAPI

        app = FastAPI(title="Inferra Collector Ingest", version="0.1.0")
        app.include_router(self.router())
        return app
