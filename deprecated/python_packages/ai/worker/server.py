from __future__ import annotations

import argparse
from typing import Any

import uvicorn
from fastapi import FastAPI

from .runtime import run_payload


app = FastAPI(title="Inferra AI Worker", version="1")


@app.get("/health")
async def health() -> dict[str, Any]:
    return {"ok": True, "service": "inferra-ai-worker"}


@app.post("/v1/investigate")
async def investigate(payload: dict[str, Any]) -> dict[str, Any]:
    return await run_payload(payload)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Inferra internal AI worker")
    parser.add_argument("--listen", default="127.0.0.1:7444", help="host:port to listen on")
    args = parser.parse_args(argv)
    host, _, port_str = args.listen.partition(":")
    port = int(port_str or "7444")
    uvicorn.run(app, host=host, port=port, log_level="info")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
