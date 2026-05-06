"""Async HTTP client helpers used by CLI commands that talk to the local API.

These helpers expect the standard Inferra config dataclass shape but are not
typed against it directly to avoid a heavyweight import in unit tests.
"""

from __future__ import annotations

import asyncio
import json
from typing import Any

import aiohttp

from cli_core.result import CommandError

LOCAL_API_TIMEOUT_SECONDS = 2.0


def server_url(config: Any) -> str:
    return f"http://{config.server.host}:{config.server.port}"


async def local_api_json(
    config: Any,
    method: str,
    path: str,
    payload: dict[str, Any] | None = None,
) -> dict[str, Any]:
    url = f"{server_url(config)}{path}"
    timeout = aiohttp.ClientTimeout(total=LOCAL_API_TIMEOUT_SECONDS)
    try:
        async with aiohttp.ClientSession(timeout=timeout) as session:
            async with session.request(method, url, json=payload) as response:
                body = await response.text()
                if response.status >= 400:
                    raise CommandError(
                        f"Inferra is reachable at {url} but returned HTTP {response.status}: {body}"
                    )
    except (aiohttp.ClientError, asyncio.TimeoutError) as exc:
        raise CommandError(f"No running Inferra supervisor found at {url}.") from exc
    try:
        decoded = json.loads(body or "{}")
    except json.JSONDecodeError as exc:
        raise CommandError(f"Inferra returned invalid JSON from {url}.") from exc
    if not isinstance(decoded, dict):
        raise CommandError(f"Inferra returned an unexpected payload from {url}.")
    return decoded


async def require_local_api(
    config: Any,
    method: str,
    path: str,
    payload: dict[str, Any] | None = None,
) -> dict[str, Any]:
    try:
        return await local_api_json(config, method, path, payload)
    except CommandError as exc:
        raise CommandError(
            f"{exc} Start it with `inferra serve` or `inferra service start`."
        ) from exc
