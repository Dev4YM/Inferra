"""Print [project].version from pyproject.toml; argv[1] = repository root.

Called from InferraWindows.psm1 (avoid fragile python -c quoting on Windows).
"""

from __future__ import annotations

import pathlib
import sys
import tomllib


def main() -> None:
    if len(sys.argv) < 2:
        print("usage: read_pyproject_version.py <repo-root>", file=sys.stderr)
        raise SystemExit(2)
    root = pathlib.Path(sys.argv[1])
    path = root / "pyproject.toml"
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    print(str(data["project"]["version"]), end="")


if __name__ == "__main__":
    main()
