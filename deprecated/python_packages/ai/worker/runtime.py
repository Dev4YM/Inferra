from __future__ import annotations

from pathlib import Path
from typing import Any

from fastapi import HTTPException

from ai.investigation import (
    EvidenceBundle,
    investigation_result_to_dict,
    run_investigation,
)
from config import load_config


def coerce_bundle(payload: dict[str, Any]) -> EvidenceBundle:
    bundle = payload.get("bundle")
    if not isinstance(bundle, dict):
        raise HTTPException(status_code=422, detail="'bundle' object is required")
    return EvidenceBundle(
        mode=str(bundle.get("mode") or "operator"),
        incident=bundle.get("incident"),
        hypotheses=list(bundle.get("hypotheses") or []),
        events=list(bundle.get("events") or []),
        services=list(bundle.get("services") or []),
        runtime=dict(bundle.get("runtime") or {}),
        workspace=dict(bundle.get("workspace") or {}),
        user_question=str(bundle.get("user_question") or ""),
        constraints=dict(bundle.get("constraints") or {}),
    )


def load_worker_config(payload: dict[str, Any]):
    config_path = payload.get("config_path")
    if config_path:
        return load_config(Path(str(config_path)))
    return load_config()


async def run_payload(payload: dict[str, Any]) -> dict[str, Any]:
    config = load_worker_config(payload)
    bundle = coerce_bundle(payload)
    result = await run_investigation(config, bundle)
    return investigation_result_to_dict(result)
