"""Read the canonical Inferra release version for Windows packaging scripts."""

from __future__ import annotations

import sys
from pathlib import Path


def main() -> None:
    repo = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
    version_file = repo / "VERSION"
    if not version_file.is_file():
        raise SystemExit(f"Missing VERSION file: {version_file}")
    print(version_file.read_text(encoding="utf-8").strip())


if __name__ == "__main__":
    main()
