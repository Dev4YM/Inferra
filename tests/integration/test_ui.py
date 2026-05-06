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
        page.wait_for_selector("#root", timeout=10_000)
        assert "Inferra" in page.inner_text("body")
        assert page.locator('script[type="module"][src^="/assets/"]').count() >= 1
        for label, heading in (
            ("Incidents", "Incidents"),
            ("Systems", "Systems"),
            ("Evidence", "Evidence"),
            ("AI Investigator", "AI Investigator"),
            ("Workspace", "Workspace"),
            ("Control", "Control"),
            ("Settings", "Settings"),
        ):
            page.get_by_role("link", name=label).click()
            page.wait_for_selector("h2.page-title", timeout=10_000)
            assert page.locator("h2.page-title").inner_text() == heading

        page.get_by_role("button", name="Developer").click()
        page.wait_for_function("() => document.body.innerText.includes('Mode saved')", timeout=10_000)
        saved_mode = page.evaluate(
            """async () => {
                const response = await fetch('/api/config');
                const payload = await response.json();
                return payload.config.experience.mode;
            }"""
        )
        assert saved_mode == "developer"
        browser.close()
