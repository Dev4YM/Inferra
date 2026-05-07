from __future__ import annotations

import sys
from pathlib import Path

from fastapi import FastAPI, HTTPException
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles


def web_ui_dist_dir() -> Path:
    """Resolve the React control-plane bundle in source and packaged builds."""
    meipass = getattr(sys, "_MEIPASS", None)
    if getattr(sys, "frozen", False) and isinstance(meipass, str) and meipass:
        bundled = Path(meipass) / "web" / "ui_dist"
        if bundled.is_dir():
            return bundled
    repo_bundle = Path(__file__).resolve().parents[3] / "src" / "web" / "ui_dist"
    if repo_bundle.is_dir():
        return repo_bundle
    legacy_bundle = Path(__file__).resolve().parent / "ui_dist"
    return legacy_bundle


def mount_frontend_assets(app: FastAPI, ui_dist: Path | None = None) -> Path:
    """Mount immutable frontend assets and return the bundle directory used."""
    resolved = ui_dist or web_ui_dist_dir()
    assets_dir = resolved / "assets"
    if assets_dir.is_dir():
        app.mount("/assets", StaticFiles(directory=assets_dir), name="ui_assets")
    return resolved


def register_frontend_routes(app: FastAPI, ui_dist: Path) -> None:
    """Register the dashboard entrypoint and SPA fallback routes."""

    @app.get("/")
    async def index():
        ui_index = ui_dist / "index.html"
        if ui_index.is_file():
            return FileResponse(ui_index)
        raise HTTPException(status_code=503, detail="Web UI bundle is missing. Run npm run build in src/web/frontend first.")

    @app.get("/{full_path:path}")
    async def spa_fallback(full_path: str):
        if full_path.startswith("api/") or full_path == "ws" or full_path.startswith("ws/"):
            raise HTTPException(status_code=404, detail="Not found")
        ui_index = ui_dist / "index.html"
        if ui_index.is_file():
            return FileResponse(ui_index)
        raise HTTPException(status_code=404, detail="Not found")
