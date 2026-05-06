from __future__ import annotations

from unittest.mock import AsyncMock, MagicMock

import pytest
from fastapi import WebSocket
from starlette.websockets import WebSocketState

from core.ids import new_id
from web.live_hub import LiveHub


def test_new_id_prefix_and_shape() -> None:
    first = new_id("evt")
    second = new_id("evt")
    assert first.startswith("evt-")
    assert second.startswith("evt-")
    assert first != second
    assert len(first.split("-", 1)[1]) == 32


@pytest.mark.asyncio
async def test_live_hub_broadcast_to_registered_socket() -> None:
    hub = LiveHub()
    ws = MagicMock(spec=WebSocket)
    ws.client_state = WebSocketState.CONNECTED
    ws.send_json = AsyncMock()
    await hub.register(ws)
    await hub.broadcast("event_count", {"total": 3})
    ws.send_json.assert_called_once()


@pytest.mark.asyncio
async def test_live_hub_incident_subscription_filters_ai_stream() -> None:
    hub = LiveHub()
    ws_a = MagicMock(spec=WebSocket)
    ws_a.client_state = WebSocketState.CONNECTED
    ws_a.send_json = AsyncMock()
    ws_b = MagicMock(spec=WebSocket)
    ws_b.client_state = WebSocketState.CONNECTED
    ws_b.send_json = AsyncMock()
    await hub.register(ws_a)
    await hub.register(ws_b)
    hub.subscribe_incident(ws_a, "inc-1")
    hub.subscribe_incident(ws_b, "inc-2")
    await hub.broadcast("ai_stream_token", {"incident_id": "inc-1", "token": "x", "done": False})
    assert ws_a.send_json.called
    assert not ws_b.send_json.called
