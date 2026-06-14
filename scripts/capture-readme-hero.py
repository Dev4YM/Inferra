#!/usr/bin/env python3
"""Capture docs/assets/readme-hero.png from the real Overview showcase page."""

from __future__ import annotations

import argparse
import os
import socket
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FRONTEND = ROOT / "src" / "web" / "frontend"
OUTPUT = ROOT / "docs" / "assets" / "readme-hero.png"
DEFAULT_PORT = 5199


def npm_command() -> str:
    return "npm.cmd" if os.name == "nt" else "npm"


def port_open(port: int) -> bool:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.settimeout(0.4)
        return sock.connect_ex(("127.0.0.1", port)) == 0


def wait_for_server(port: int, timeout_seconds: float = 45.0) -> None:
    deadline = time.time() + timeout_seconds
    while time.time() < deadline:
        if port_open(port):
            return
        time.sleep(0.25)
    raise RuntimeError(f"Vite dev server did not start on port {port} within {timeout_seconds}s")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    parser.add_argument("--output", type=Path, default=OUTPUT)
    parser.add_argument("--keep-server", action="store_true")
    args = parser.parse_args()

    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("playwright is required: python -m pip install playwright && python -m playwright install chromium", file=sys.stderr)
        return 1

    args.output.parent.mkdir(parents=True, exist_ok=True)
    dev = subprocess.Popen(
        [npm_command(), "run", "dev", "--", "--port", str(args.port), "--strictPort", "--host", "127.0.0.1"],
        cwd=FRONTEND,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        shell=os.name == "nt",
    )
    try:
        wait_for_server(args.port)
        url = f"http://127.0.0.1:{args.port}/readme-hero.html"
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch()
            page = browser.new_page(viewport={"width": 1920, "height": 1080}, device_scale_factor=2)
            page.goto(url, wait_until="networkidle")
            page.wait_for_selector("#readme-hero-capture")
            page.wait_for_timeout(1200)
            capture = page.locator("#readme-hero-capture")
            capture.screenshot(path=str(args.output), type="png")
            browser.close()
        print(f"Wrote {args.output}")
        return 0
    finally:
        if args.keep_server:
            print(f"Leaving Vite running on http://127.0.0.1:{args.port}/readme-hero.html")
        else:
            dev.terminate()
            try:
                dev.wait(timeout=10)
            except subprocess.TimeoutExpired:
                dev.kill()


if __name__ == "__main__":
    raise SystemExit(main())
