#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


def run(cmd: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=cwd,
        check=True,
        text=True,
        capture_output=True,
    )


def choose_port() -> int:
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    return int(port)


def fetch_json(url: str) -> dict[str, object]:
    with urllib.request.urlopen(url, timeout=5) as response:
        body = response.read().decode("utf-8")
    return json.loads(body)


def wait_for_json(url: str, timeout_seconds: float) -> dict[str, object]:
    deadline = time.time() + timeout_seconds
    last_error: Exception | None = None
    while time.time() < deadline:
        try:
            return fetch_json(url)
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as error:
            last_error = error
            time.sleep(1)
    raise RuntimeError(f"Timed out waiting for {url}: {last_error}")


def terminate_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=15)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=10)


def main() -> int:
    parser = argparse.ArgumentParser(description="Smoke-test the built Rust Inferra runtime.")
    parser.add_argument("--binary", required=True, help="Path to the built inferra binary")
    parser.add_argument("--repo-root", required=True, help="Repository root")
    parser.add_argument("--ui-dist", help="Optional UI dist path override")
    args = parser.parse_args()

    repo_root = Path(args.repo_root).resolve()
    binary = Path(args.binary).resolve()
    if not binary.exists():
        raise SystemExit(f"Binary not found: {binary}")

    ui_dist = Path(args.ui_dist).resolve() if args.ui_dist else repo_root / "src" / "web" / "ui_dist"
    if not ui_dist.exists():
        raise SystemExit(f"UI dist not found: {ui_dist}")

    with tempfile.TemporaryDirectory(prefix="inferra-rust-smoke-") as temp_dir:
        root = Path(temp_dir)
        config_path = root / "inferra.toml"
        data_dir = root / "data"
        data_dir.mkdir(parents=True, exist_ok=True)
        port = choose_port()

        base_cmd = [str(binary), "--config", str(config_path)]
        run(base_cmd + ["setup", "--yes", "--skip-connection-test", "--data-dir", str(data_dir)], repo_root)
        run(base_cmd + ["config", "set", "server.host", "127.0.0.1"], repo_root)
        run(base_cmd + ["config", "set", "server.port", str(port)], repo_root)
        run(base_cmd + ["config", "set", "collectors.auto_start", "false"], repo_root)
        run(base_cmd + ["config", "set", "ai.enabled", "false"], repo_root)
        run(base_cmd + ["init-db"], repo_root)

        serve_log = root / "serve.log"
        with serve_log.open("w", encoding="utf-8") as log_file:
            process = subprocess.Popen(
                base_cmd + ["--ui-dist", str(ui_dist), "serve"],
                cwd=repo_root,
                stdout=log_file,
                stderr=subprocess.STDOUT,
                text=True,
            )

        try:
            health = wait_for_json(f"http://127.0.0.1:{port}/api/health", 30)
            overview = wait_for_json(f"http://127.0.0.1:{port}/api/overview", 15)
            collectors = wait_for_json(f"http://127.0.0.1:{port}/api/collectors", 15)

            if not isinstance(health, dict) or "status" not in health:
                raise RuntimeError(f"Unexpected health payload: {health}")
            if not isinstance(overview, dict) or "dashboard" not in overview:
                raise RuntimeError(f"Unexpected overview payload: {overview}")
            if not isinstance(collectors, dict) or "collectors" not in collectors:
                raise RuntimeError(f"Unexpected collectors payload: {collectors}")

            cli_status = run(base_cmd + ["--json", "collectors", "status"], repo_root)
            json.loads(cli_status.stdout)
        except Exception:
            terminate_process(process)
            log_text = serve_log.read_text(encoding="utf-8", errors="replace") if serve_log.exists() else ""
            print(log_text, file=sys.stderr)
            raise
        finally:
            terminate_process(process)

        if shutil.which("python") is None:
            return 0
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
