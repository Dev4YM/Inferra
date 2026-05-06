"""Optional browser smoke tests; skipped when Playwright is not installed."""

from __future__ import annotations

import threading
import time
import urllib.error
import urllib.request

import pytest

pytest.importorskip("playwright.sync_api")
from playwright.sync_api import sync_playwright

from config.model import InferraConfig, StorageConfig
from web import create_app


def _serve(app, host: str, port: int) -> None:
    import uvicorn

    config = uvicorn.Config(app, host=host, port=port, log_level="warning")
    server = uvicorn.Server(config)
    server.run()


@pytest.mark.integration
def test_console_index_loads_offline_bundle(tmp_path) -> None:
    app = create_app(InferraConfig(storage=StorageConfig(data_dir=tmp_path)))
    host = "127.0.0.1"
    port = 37933
    thread = threading.Thread(target=_serve, args=(app, host, port), daemon=True)
    thread.start()
    deadline = time.time() + 15
    while time.time() < deadline:
        try:
            urllib.request.urlopen(f"http://{host}:{port}/", timeout=1)
            break
        except (urllib.error.URLError, OSError):
            time.sleep(0.15)
    else:
        pytest.fail("server did not become ready")

    with sync_playwright() as p:
        try:
            browser = p.chromium.launch(headless=True)
        except Exception as exc:
            pytest.skip(f"playwright browser unavailable: {exc}")
        page = browser.new_page()
        page.goto(f"http://{host}:{port}/", wait_until="domcontentloaded", timeout=30_000)
        page.wait_for_selector("#page-title", timeout=10_000)
        assert "Dashboard" in page.inner_text("#page-title")
        assert page.locator('link[href="/static/tailwind.css"]').count() == 1
        assert page.locator('script[type="module"][src="/static/js/main.js"]').count() == 1
        browser.close()
