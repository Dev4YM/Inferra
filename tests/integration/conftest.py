"""Integration test fixtures (Rust runtime)."""

from __future__ import annotations

import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from collections.abc import Iterator
from pathlib import Path

import pytest

_REPO_ROOT = Path(__file__).resolve().parents[2]


def _choose_port() -> int:
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = int(sock.getsockname()[1])
    sock.close()
    return port


def _inferra_binary() -> Path:
    env = os.environ.get("INFERRA_BINARY")
    if env:
        return Path(env)
    name = "inferra.exe" if sys.platform == "win32" else "inferra"
    candidates = [
        _REPO_ROOT / "src" / "target" / "release" / name,
        _REPO_ROOT / "src" / "target" / "debug" / name,
        _REPO_ROOT / "target" / "release" / name,
        _REPO_ROOT / "target" / "debug" / name,
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return candidates[0]


def _run(cmd: list[str], cwd: Path) -> None:
    subprocess.run(cmd, cwd=cwd, check=True, text=True, capture_output=True)


def _fetch_json(url: str, headers: dict[str, str] | None = None, timeout: float = 10) -> tuple[int, dict]:
    request = urllib.request.Request(url, headers=headers or {})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode("utf-8")
            return response.status, json.loads(body) if body.strip() else {}
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        try:
            payload = json.loads(body) if body.strip() else {}
        except json.JSONDecodeError:
            payload = {"detail": body}
        return error.code, payload


def _post_json(
    url: str,
    payload: dict,
    headers: dict[str, str] | None = None,
    timeout: float = 60,
) -> tuple[int, dict]:
    data = json.dumps(payload).encode("utf-8")
    merged = {"Content-Type": "application/json", **(headers or {})}
    request = urllib.request.Request(url, data=data, headers=merged, method="POST")
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode("utf-8")
            return response.status, json.loads(body) if body.strip() else {}
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        try:
            parsed = json.loads(body) if body.strip() else {}
        except json.JSONDecodeError:
            parsed = {"detail": body}
        return error.code, parsed


def _put_json(url: str, payload: dict, headers: dict[str, str] | None = None, timeout: float = 60) -> tuple[int, dict]:
    data = json.dumps(payload).encode("utf-8")
    merged = {"Content-Type": "application/json", **(headers or {})}
    request = urllib.request.Request(url, data=data, headers=merged, method="PUT")
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode("utf-8")
            return response.status, json.loads(body) if body.strip() else {}
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")
        try:
            parsed = json.loads(body) if body.strip() else {}
        except json.JSONDecodeError:
            parsed = {"detail": body}
        return error.code, parsed


def _wait_for_json(base_url: str, path: str, timeout_seconds: float = 45) -> dict:
    deadline = time.time() + timeout_seconds
    last_error: Exception | None = None
    while time.time() < deadline:
        try:
            status, payload = _fetch_json(f"{base_url}{path}")
            if status == 200 and isinstance(payload, dict):
                return payload
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
            last_error = error
        time.sleep(0.5)
    raise RuntimeError(f"Timed out waiting for {base_url}{path}: {last_error}")


def _wait_for_health(base_url: str, timeout_seconds: float = 45) -> dict:
    return _wait_for_json(base_url, "/api/health", timeout_seconds)


def _terminate(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    if sys.platform == "win32":
        subprocess.run(
            ["taskkill", "/F", "/T", "/PID", str(process.pid)],
            capture_output=True,
            check=False,
        )
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=10)
        return
    process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=10)


@pytest.fixture(scope="module")
def rust_runtime() -> Iterator[dict[str, object]]:
    binary = _inferra_binary()
    if not binary.exists():
        pytest.skip(f"Rust inferra binary not found: {binary} (run cargo build -p inferra-cli)")

    ui_dist = _REPO_ROOT / "src" / "web" / "ui_dist"
    if not ui_dist.exists():
        pytest.skip(f"UI dist not found: {ui_dist}")

    with tempfile.TemporaryDirectory(prefix="inferra-rust-it-") as temp_dir:
        root = Path(temp_dir)
        config_path = root / "inferra.toml"
        data_dir = root / "data"
        data_dir.mkdir(parents=True, exist_ok=True)
        port = _choose_port()
        base_url = f"http://127.0.0.1:{port}"

        base_cmd = [str(binary), "--config", str(config_path)]
        _run(base_cmd + ["setup", "--yes", "--skip-connection-test", "--data-dir", str(data_dir)], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "server.host", "127.0.0.1"], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "server.port", str(port)], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "collectors.auto_start", "false"], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "ai.enabled", "false"], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "workspace.enabled", "false"], _REPO_ROOT)
        _run(base_cmd + ["init-db"], _REPO_ROOT)

        process = subprocess.Popen(
            base_cmd + ["--ui-dist", str(ui_dist), "serve"],
            cwd=_REPO_ROOT,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            text=True,
        )

        try:
            health = _wait_for_health(base_url)
            yield {
                "base_url": base_url,
                "config_path": config_path,
                "data_dir": data_dir,
                "binary": binary,
                "health": health,
                "fetch_json": _fetch_json,
                "post_json": _post_json,
                "put_json": _put_json,
            }
        finally:
            _terminate(process)


def _configured_rust_runtime(
    *,
    name: str,
    auth_env_name: str | None = None,
    auth_token: str | None = None,
) -> Iterator[dict[str, object]]:
    binary = _inferra_binary()
    if not binary.exists():
        pytest.skip(f"Rust inferra binary not found: {binary} (run cargo build -p inferra-cli)")

    ui_dist = _REPO_ROOT / "src" / "web" / "ui_dist"
    if not ui_dist.exists():
        pytest.skip(f"UI dist not found: {ui_dist}")

    with tempfile.TemporaryDirectory(prefix=f"inferra-{name}-") as temp_dir:
        root = Path(temp_dir)
        config_path = root / "inferra.toml"
        data_dir = root / "data"
        data_dir.mkdir(parents=True, exist_ok=True)
        port = _choose_port()
        base_url = f"http://127.0.0.1:{port}"

        base_cmd = [str(binary), "--config", str(config_path)]
        _run(base_cmd + ["setup", "--yes", "--skip-connection-test", "--data-dir", str(data_dir)], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "server.host", "127.0.0.1"], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "server.port", str(port)], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "collectors.auto_start", "false"], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "ai.enabled", "false"], _REPO_ROOT)
        _run(base_cmd + ["config", "set", "workspace.enabled", "false"], _REPO_ROOT)
        if auth_env_name:
            _run(base_cmd + ["config", "set", "server.auth_token_env", auth_env_name], _REPO_ROOT)
            _run(base_cmd + ["config", "set", "server.require_loopback", "false"], _REPO_ROOT)
        _run(base_cmd + ["init-db"], _REPO_ROOT)

        env = os.environ.copy()
        if auth_env_name:
            env.pop(auth_env_name, None)
        if auth_env_name and auth_token is not None:
            env[auth_env_name] = auth_token

        process = subprocess.Popen(
            base_cmd + ["--ui-dist", str(ui_dist), "serve"],
            cwd=_REPO_ROOT,
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            text=True,
        )

        try:
            probe = _wait_for_json(base_url, "/healthz")
            yield {
                "base_url": base_url,
                "config_path": config_path,
                "data_dir": data_dir,
                "binary": binary,
                "probe": probe,
                "auth_env_name": auth_env_name,
                "auth_token": auth_token,
                "fetch_json": _fetch_json,
                "post_json": _post_json,
                "put_json": _put_json,
            }
        finally:
            _terminate(process)


@pytest.fixture(scope="module")
def rust_runtime_auth() -> Iterator[dict[str, object]]:
    yield from _configured_rust_runtime(
        name="rust-auth-it",
        auth_env_name="INFERRA_TEST_API_TOKEN",
        auth_token="secret-token",
    )


@pytest.fixture(scope="module")
def rust_runtime_auth_unset() -> Iterator[dict[str, object]]:
    yield from _configured_rust_runtime(
        name="rust-auth-unset-it",
        auth_env_name="INFERRA_TEST_API_TOKEN_UNSET",
        auth_token=None,
    )
