#!/usr/bin/env python3
"""Inferra release version — canonical VERSION file, sync, and strict verification."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VERSION_FILE = ROOT / "VERSION"
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+$")


def read_version() -> str:
    if not VERSION_FILE.is_file():
        raise FileNotFoundError(f"Missing canonical version file: {VERSION_FILE}")
    version = VERSION_FILE.read_text(encoding="utf-8").strip()
    if not SEMVER_RE.fullmatch(version):
        raise ValueError(f"VERSION must be semver MAJOR.MINOR.PATCH, got: {version!r}")
    return version


def _read_text(rel: str) -> str:
    return (ROOT / rel).read_text(encoding="utf-8")


def _cargo_workspace_version() -> str:
    text = _read_text("src/Cargo.toml")
    match = re.search(r'^\[workspace\.package\]\s*\n(?:[^\[]*\n)*?^version = "([^"]+)"', text, re.M)
    if not match:
        raise RuntimeError("Could not parse workspace.package.version from src/Cargo.toml")
    return match.group(1)


def _pyproject_version() -> str:
    text = _read_text("pyproject.toml")
    match = re.search(r'^version = "([^"]+)"', text, re.M)
    if not match:
        raise RuntimeError("Could not parse version from pyproject.toml")
    return match.group(1)


def _package_json_version() -> str:
    data = json.loads(_read_text("src/web/frontend/package.json"))
    return str(data["version"])


def _package_lock_version() -> str:
    data = json.loads(_read_text("src/web/frontend/package-lock.json"))
    return str(data["version"])


def _helm_chart_version() -> tuple[str, str]:
    text = _read_text("deploy/helm/inferra/Chart.yaml")
    chart = re.search(r"^version:\s*([^\s#]+)", text, re.M)
    app = re.search(r'^appVersion:\s*"([^"]+)"', text, re.M)
    if not chart or not app:
        raise RuntimeError("Could not parse deploy/helm/inferra/Chart.yaml versions")
    return chart.group(1), app.group(1)


def _helm_values_tag() -> str:
    text = _read_text("deploy/helm/inferra/values.yaml")
    match = re.search(r'^\s*tag:\s*"([^"]+)"', text, re.M)
    if not match:
        raise RuntimeError("Could not parse image.tag from deploy/helm/inferra/values.yaml")
    return match.group(1)


def collect_checks(version: str) -> list[tuple[str, str, str]]:
    chart_version, chart_app = _helm_chart_version()
    return [
        ("src/Cargo.toml [workspace.package].version", _cargo_workspace_version(), version),
        ("pyproject.toml version", _pyproject_version(), version),
        ("src/web/frontend/package.json version", _package_json_version(), version),
        ("src/web/frontend/package-lock.json version", _package_lock_version(), version),
        ("deploy/helm/inferra/Chart.yaml version", chart_version, version),
        ("deploy/helm/inferra/Chart.yaml appVersion", chart_app, version),
        ("deploy/helm/inferra/values.yaml image.tag", _helm_values_tag(), version),
    ]


def verify() -> int:
    version = read_version()
    mismatches = [
        (label, actual, expected)
        for label, actual, expected in collect_checks(version)
        if actual != expected
    ]
    if mismatches:
        print(f"Inferra version mismatch — canonical VERSION is {version}", file=sys.stderr)
        for label, actual, expected in mismatches:
            print(f"  - {label}: {actual!r} (expected {expected!r})", file=sys.stderr)
        print("Run: python scripts/version.py sync", file=sys.stderr)
        return 1
    print(f"Inferra version OK: {version}")
    return 0


def _replace_once(text: str, pattern: str, replacement: str, *, path: str) -> str:
    updated, count = re.subn(pattern, replacement, text, count=1, flags=re.M)
    if count != 1:
        raise RuntimeError(f"Expected one replacement in {path} for pattern {pattern!r}")
    return updated


def sync() -> int:
    version = read_version()

    cargo_path = ROOT / "src/Cargo.toml"
    cargo_text = cargo_path.read_text(encoding="utf-8")
    cargo_text = _replace_once(
        cargo_text,
        r'(^version = ")[^"]+(")',
        rf'\g<1>{version}\2',
        path="src/Cargo.toml",
    )
    cargo_path.write_text(cargo_text, encoding="utf-8")

    pyproject_path = ROOT / "pyproject.toml"
    pyproject_text = pyproject_path.read_text(encoding="utf-8")
    pyproject_text = _replace_once(
        pyproject_text,
        r'(^version = ")[^"]+(")',
        rf'\g<1>{version}\2',
        path="pyproject.toml",
    )
    pyproject_path.write_text(pyproject_text, encoding="utf-8")

    package_json_path = ROOT / "src/web/frontend/package.json"
    package_data = json.loads(package_json_path.read_text(encoding="utf-8"))
    package_data["version"] = version
    package_json_path.write_text(json.dumps(package_data, indent=2) + "\n", encoding="utf-8")

    lock_path = ROOT / "src/web/frontend/package-lock.json"
    lock_data = json.loads(lock_path.read_text(encoding="utf-8"))
    lock_data["version"] = version
    if "" in lock_data.get("packages", {}):
        lock_data["packages"][""]["version"] = version
    lock_path.write_text(json.dumps(lock_data, indent=2) + "\n", encoding="utf-8")

    chart_path = ROOT / "deploy/helm/inferra/Chart.yaml"
    chart_text = chart_path.read_text(encoding="utf-8")
    chart_text = _replace_once(chart_text, r"(^version:\s*)[^\s#]+", rf"\g<1>{version}", path="Chart.yaml")
    chart_text = _replace_once(
        chart_text,
        r'(^appVersion:\s*")[^"]+(")',
        rf'\g<1>{version}\2',
        path="Chart.yaml",
    )
    chart_path.write_text(chart_text, encoding="utf-8")

    values_path = ROOT / "deploy/helm/inferra/values.yaml"
    values_text = values_path.read_text(encoding="utf-8")
    values_text = _replace_once(
        values_text,
        r'(^(\s*)tag:\s*")[^"]+(")',
        rf'\g<1>{version}\3',
        path="values.yaml",
    )
    values_path.write_text(values_text, encoding="utf-8")

    print(f"Synced Inferra version {version} to all release manifests.")
    return verify()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("show", help="Print canonical VERSION")
    sub.add_parser("verify", help="Fail if any release manifest disagrees with VERSION")
    sub.add_parser("sync", help="Write VERSION to all release manifests, then verify")
    args = parser.parse_args()

    if args.command == "show":
        print(read_version())
        return 0
    if args.command == "verify":
        return verify()
    if args.command == "sync":
        return sync()
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
