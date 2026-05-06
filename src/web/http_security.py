from __future__ import annotations

import os
from typing import Callable

from starlette.middleware.base import BaseHTTPMiddleware
from starlette.requests import Request
from starlette.responses import JSONResponse, Response


class ContentSecurityPolicyMiddleware(BaseHTTPMiddleware):
    def __init__(self, app: Callable, *, policy: str) -> None:
        super().__init__(app)
        self.policy = policy

    async def dispatch(self, request: Request, call_next: Callable) -> Response:
        response = await call_next(request)
        response.headers.setdefault("Content-Security-Policy", self.policy)
        return response


class LocalSecurityMiddleware(BaseHTTPMiddleware):
    def __init__(
        self,
        app: Callable,
        *,
        require_loopback: bool,
        auth_token_env: str,
        allow_paths: frozenset[str],
    ) -> None:
        super().__init__(app)
        self.require_loopback = require_loopback
        self.auth_token_env = auth_token_env
        self.allow_paths = allow_paths

    async def dispatch(self, request: Request, call_next: Callable) -> Response:
        if request.method == "OPTIONS":
            return await call_next(request)
        path = request.url.path
        if path in self.allow_paths or path.startswith("/static/"):
            return await call_next(request)
        client_host = request.client.host if request.client else None
        if self.require_loopback and client_host not in (None, "127.0.0.1", "::1", "testclient"):
            return JSONResponse({"detail": "local clients only"}, status_code=403)
        if self.auth_token_env:
            expected = os.environ.get(self.auth_token_env, "").strip()
            if not expected:
                return JSONResponse(
                    {"detail": f"server auth_token_env {self.auth_token_env!r} is not set in the environment"},
                    status_code=503,
                )
            header = request.headers.get("authorization") or request.headers.get("Authorization") or ""
            scheme, _, value = header.partition(" ")
            if scheme.lower() != "bearer" or value.strip() != expected:
                return JSONResponse({"detail": "unauthorized"}, status_code=401)
        return await call_next(request)
