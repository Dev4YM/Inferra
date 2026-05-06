"""Discover local code projects by marker files (read-only, bounded scan)."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class CodeProjectHit:
    root: Path
    kind: str
    marker: str


_MARKERS: tuple[tuple[str, str], ...] = (
    ("package.json", "node"),
    ("pnpm-workspace.yaml", "pnpm_workspace"),
    ("yarn.lock", "yarn"),
    ("pyproject.toml", "python"),
    ("requirements.txt", "python_req"),
    ("setup.py", "python_legacy"),
    ("Cargo.toml", "rust"),
    ("go.mod", "go"),
    ("composer.json", "php"),
    ("Gemfile", "ruby"),
    ("pom.xml", "maven"),
    ("build.gradle", "gradle"),
    ("build.gradle.kts", "gradle_kts"),
    ("*.csproj", "dotnet"),
    ("*.sln", "dotnet_sln"),
    ("Makefile", "make"),
    ("Dockerfile", "docker"),
)


def _has_marker(directory: Path, marker: str) -> bool:
    if "*" in marker:
        return any(directory.glob(marker))
    return (directory / marker).is_file()


def discover_code_projects(
    roots: list[Path] | None = None,
    *,
    max_depth: int = 3,
    max_results: int = 40,
) -> list[CodeProjectHit]:
    """Walk likely directories under each root and return unique project roots."""
    if roots is None:
        roots = []
        cwd = Path.cwd()
        roots.append(cwd)
        home = Path.home()
        for name in ("Projects", "projects", "src", "Source", "workspace", "Workspace", "code"):
            candidate = home / name
            if candidate.is_dir():
                roots.append(candidate)
    seen: set[Path] = set()
    hits: list[CodeProjectHit] = []
    visited_inodes: set[Path] = set()

    def walk(root: Path, depth: int) -> None:
        nonlocal hits
        try:
            resolved = root.resolve()
        except OSError:
            return
        if depth > max_depth or len(hits) >= max_results:
            return
        try:
            if not root.is_dir():
                return
        except OSError:
            return
        if resolved in visited_inodes:
            return
        visited_inodes.add(resolved)
        found_here = False
        for marker, kind in _MARKERS:
            if len(hits) >= max_results:
                return
            if _has_marker(root, marker):
                found_here = True
                if resolved not in seen:
                    seen.add(resolved)
                    hits.append(CodeProjectHit(root=resolved, kind=kind, marker=marker.replace("*", "")))
                break
        if len(hits) >= max_results:
            return
        if found_here:
            return
        try:
            children = list(root.iterdir())
        except OSError:
            return
        # Skip heavy / system dirs
        skip_names = {
            "node_modules",
            ".git",
            ".venv",
            "venv",
            "__pycache__",
            "dist",
            "build",
            "target",
            ".cargo",
            ".npm",
            ".yarn",
            "Library",
            "AppData",
        }
        for child in sorted(children, key=lambda p: p.name.lower()):
            if len(hits) >= max_results:
                return
            if not child.is_dir():
                continue
            if child.name.startswith(".") and child.name not in {".github"}:
                continue
            if child.name in skip_names:
                continue
            walk(child, depth + 1)

    for r in roots:
        walk(Path(r), 0)

    hits.sort(key=lambda h: str(h.root).lower())
    return hits[:max_results]


def projects_to_json(rows: list[CodeProjectHit]) -> list[dict[str, str]]:
    return [{"path": str(row.root), "kind": row.kind, "marker": row.marker} for row in rows]
