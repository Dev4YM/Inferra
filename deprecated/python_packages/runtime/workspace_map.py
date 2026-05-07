"""Build a workspace map: discovered projects + service-to-project mappings with confidence.

This is read-only; it cannot mutate observed systems or send data anywhere. It combines:
- explicit user mappings from config (highest confidence)
- compose-style filename heuristics (medium confidence)
- service-name token matches against project paths (low confidence)

Each mapping carries the signals that produced it so the UI can explain the call.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

from config import InferraConfig
from runtime.workspace_scan import CodeProjectHit, discover_code_projects, projects_to_json

_TOKEN_SPLIT = re.compile(r"[^a-z0-9]+")


@dataclass(slots=True)
class ServiceMappingSignal:
    name: str
    confidence: float
    detail: str


@dataclass(slots=True)
class ServiceProjectMapping:
    service_id: str
    project_path: str
    confidence: float
    source: str
    signals: list[ServiceMappingSignal] = field(default_factory=list)
    notes: str = ""


def _tokenize(value: str) -> list[str]:
    return [token for token in _TOKEN_SPLIT.split(value.lower()) if len(token) >= 3]


def _path_tokens(path: str) -> set[str]:
    parts: list[str] = []
    for piece in Path(path).parts:
        parts.extend(_tokenize(str(piece)))
    return set(parts)


def _project_marker(project: CodeProjectHit) -> str:
    base = project.marker or "marker"
    return f"{project.kind} ({base})"


def _project_score_for_service(service_id: str, project: CodeProjectHit) -> tuple[float, list[ServiceMappingSignal]]:
    signals: list[ServiceMappingSignal] = []
    service_tokens = set(_tokenize(service_id))
    if not service_tokens:
        return 0.0, signals
    project_tokens = _path_tokens(str(project.root))
    if not project_tokens:
        return 0.0, signals
    overlap = service_tokens & project_tokens
    if not overlap:
        return 0.0, signals
    score = min(1.0, 0.25 + 0.2 * len(overlap))
    signals.append(
        ServiceMappingSignal(
            name="path_token_match",
            confidence=round(score, 2),
            detail="shared tokens: " + ",".join(sorted(overlap)),
        )
    )
    if any(part.lower() == service_id.lower() for part in Path(project.root).parts):
        score = max(score, 0.7)
        signals.append(
            ServiceMappingSignal(
                name="exact_path_segment",
                confidence=0.7,
                detail="service id is a directory segment",
            )
        )
    return score, signals


def build_workspace_map(
    config: InferraConfig,
    *,
    services: Iterable[str] | None = None,
) -> dict[str, Any]:
    """Return a structured workspace map for CLI/web/AI consumers."""
    cfg = config.workspace
    if not cfg.enabled:
        return {
            "enabled": False,
            "projects": [],
            "service_mappings": [],
            "unmapped_services": [],
            "config_mappings": [],
        }
    roots: list[Path] = [Path(item) for item in cfg.roots] or None  # type: ignore[assignment]
    project_hits = discover_code_projects(
        roots=roots,
        max_depth=int(cfg.max_depth),
        max_results=int(cfg.max_results),
    )
    project_rows = projects_to_json(project_hits)

    explicit: list[ServiceProjectMapping] = []
    for entry in cfg.service_mappings:
        explicit.append(
            ServiceProjectMapping(
                service_id=str(entry.service_id),
                project_path=str(entry.project_path),
                confidence=float(entry.confidence),
                source=str(entry.source),
                signals=[
                    ServiceMappingSignal(
                        name="user_mapping",
                        confidence=float(entry.confidence),
                        detail=str(entry.notes or "explicit workspace.service_mappings entry"),
                    )
                ],
                notes=str(entry.notes or ""),
            )
        )

    derived: dict[tuple[str, str], ServiceProjectMapping] = {}
    if services:
        for service_id in services:
            best: ServiceProjectMapping | None = None
            for project in project_hits:
                score, sig = _project_score_for_service(service_id, project)
                if score <= 0:
                    continue
                if best is None or score > best.confidence:
                    best = ServiceProjectMapping(
                        service_id=service_id,
                        project_path=str(project.root),
                        confidence=round(score, 2),
                        source="auto",
                        signals=sig + [
                            ServiceMappingSignal(
                                name="project_marker",
                                confidence=0.1,
                                detail=_project_marker(project),
                            )
                        ],
                    )
            if best is not None:
                derived[(best.service_id, best.project_path)] = best

    explicit_keys = {(m.service_id, m.project_path) for m in explicit}
    merged: list[ServiceProjectMapping] = list(explicit)
    for key, mapping in derived.items():
        if key in explicit_keys:
            continue
        merged.append(mapping)

    mapped_services = {m.service_id for m in merged}
    unmapped = [str(item) for item in (services or []) if item not in mapped_services]

    return {
        "enabled": True,
        "projects": project_rows,
        "service_mappings": [_mapping_to_dict(m) for m in merged],
        "unmapped_services": sorted(unmapped),
        "config_mappings": [_mapping_to_dict(m) for m in explicit],
    }


def _mapping_to_dict(item: ServiceProjectMapping) -> dict[str, Any]:
    return {
        "service_id": item.service_id,
        "project_path": item.project_path,
        "confidence": round(float(item.confidence), 3),
        "source": item.source,
        "notes": item.notes,
        "signals": [
            {"name": signal.name, "confidence": signal.confidence, "detail": signal.detail}
            for signal in item.signals
        ],
    }


def inspect_project(project_path: str | Path) -> dict[str, Any]:
    """Return high-level metadata about a single project (markers, likely commands).

    All inspection is metadata-only: file *existence* not contents (per dossier guidance,
    deeper file content is opt-in developer mode and is not implemented here).
    """
    root = Path(project_path).resolve()
    if not root.is_dir():
        return {"path": str(root), "exists": False}
    markers = []
    for marker_name in (
        "package.json",
        "pnpm-workspace.yaml",
        "yarn.lock",
        "pyproject.toml",
        "requirements.txt",
        "setup.py",
        "Cargo.toml",
        "go.mod",
        "composer.json",
        "Gemfile",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "Makefile",
        "Dockerfile",
        "compose.yaml",
        "compose.yml",
        "docker-compose.yaml",
        "docker-compose.yml",
        ".env",
        ".env.example",
    ):
        candidate = root / marker_name
        if candidate.is_file():
            markers.append(marker_name)
    return {
        "path": str(root),
        "exists": True,
        "markers": markers,
        "has_compose": any(name in markers for name in ("compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml")),
        "has_dockerfile": "Dockerfile" in markers,
        "has_env_file": any(name in markers for name in (".env", ".env.example")),
    }
