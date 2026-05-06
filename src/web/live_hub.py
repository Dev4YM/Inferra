from __future__ import annotations

import asyncio
from typing import Any

from fastapi import WebSocket
from starlette.websockets import WebSocketState


class LiveHub:
    def __init__(self) -> None:
        self._lock = asyncio.Lock()
        self._connections: list[WebSocket] = []
        self._incident_subs: dict[int, set[str]] = {}

    async def register(self, websocket: WebSocket) -> None:
        async with self._lock:
            self._connections.append(websocket)
            self._incident_subs[id(websocket)] = set()

    async def unregister(self, websocket: WebSocket) -> None:
        async with self._lock:
            try:
                self._connections.remove(websocket)
            except ValueError:
                pass
            self._incident_subs.pop(id(websocket), None)

    def subscribe_incident(self, websocket: WebSocket, incident_id: str) -> None:
        self._incident_subs.setdefault(id(websocket), set()).add(incident_id)

    def unsubscribe_incident(self, websocket: WebSocket, incident_id: str) -> None:
        self._incident_subs.get(id(websocket), set()).discard(incident_id)

    def _wants_incident_stream(self, websocket: WebSocket, incident_id: str | None) -> bool:
        if not incident_id:
            return True
        subs = self._incident_subs.get(id(websocket))
        if subs is None or not subs:
            return True
        return incident_id in subs

    async def broadcast(self, kind: str, payload: dict[str, Any]) -> None:
        stream_id = payload.get("incident_id") if kind == "ai_stream_token" else None
        message = {"type": kind, **payload}
        async with self._lock:
            pairs = [(ws, self._wants_incident_stream(ws, stream_id)) for ws in list(self._connections)]
        dead: list[WebSocket] = []
        for websocket, allow in pairs:
            if not allow:
                continue
            if websocket.client_state != WebSocketState.CONNECTED:
                dead.append(websocket)
                continue
            try:
                await websocket.send_json(message)
            except Exception:
                dead.append(websocket)
        for websocket in dead:
            await self.unregister(websocket)
