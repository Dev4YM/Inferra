"""CLI result and output primitives.

`CommandResult` carries everything a handler can return: the JSON payload, the
human-readable lines, the exit code, and an optional Rich-formatting hook.
`emit_result` renders it according to argparse flags.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable


@dataclass(slots=True)
class CommandResult:
    payload: dict[str, Any]
    stdout_lines: list[str] = field(default_factory=list)
    stderr_lines: list[str] = field(default_factory=list)
    exit_code: int = 0
    rich_hook: Callable[[], None] | None = None


class CommandError(RuntimeError):
    """Raised for user-facing CLI failures."""


def json_ready(value: Any) -> Any:
    if isinstance(value, Path):
        return str(value)
    if isinstance(value, tuple):
        return [json_ready(item) for item in value]
    if isinstance(value, list):
        return [json_ready(item) for item in value]
    if isinstance(value, set):
        return [json_ready(item) for item in sorted(value)]
    if isinstance(value, dict):
        return {str(key): json_ready(item) for key, item in value.items()}
    return value


def print_json(payload: Any) -> None:
    print(json.dumps(json_ready(payload), indent=2, sort_keys=True))


def emit_result(args: argparse.Namespace, result: CommandResult) -> int:
    if getattr(args, "json", False):
        print_json(result.payload)
        return result.exit_code
    if result.rich_hook is not None and os.environ.get("INFERRA_PLAIN") != "1":
        try:
            result.rich_hook()
        except Exception as exc:
            print(f"Display error: {exc}", file=sys.stderr)
            for line in result.stdout_lines:
                print(line)
            return result.exit_code or 1
        for line in result.stderr_lines:
            print(line, file=sys.stderr)
        return result.exit_code
    for line in result.stdout_lines:
        print(line)
    for line in result.stderr_lines:
        print(line, file=sys.stderr)
    return result.exit_code
