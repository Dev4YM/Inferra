"""Shared CLI primitives.

`cli.py` remains the parser/orchestrator entry point (and the packaging
script target). The pure helpers it relies on live here so they can be
unit-tested in isolation and reused without importing the full CLI module.
"""

from cli_core.http_client import (
    LOCAL_API_TIMEOUT_SECONDS,
    local_api_json,
    require_local_api,
    server_url,
)
from cli_core.result import (
    CommandError,
    CommandResult,
    emit_result,
    json_ready,
    print_json,
)

__all__ = [
    "CommandError",
    "CommandResult",
    "LOCAL_API_TIMEOUT_SECONDS",
    "emit_result",
    "json_ready",
    "local_api_json",
    "print_json",
    "require_local_api",
    "server_url",
]
