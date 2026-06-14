# Versioning

Inferra uses **strict [Semantic Versioning](https://semver.org/)** (`MAJOR.MINOR.PATCH`) with one canonical source of truth.

## Canonical version

The release number lives in the repository root:

```text
VERSION
```

Example: `0.3.0`

Every shipping surface must match this file:

| Artifact | Field |
| --- | --- |
| `src/Cargo.toml` | `[workspace.package].version` |
| `pyproject.toml` | `project.version` |
| `src/web/frontend/package.json` | `version` |
| `src/web/frontend/package-lock.json` | root `version` |
| `deploy/helm/inferra/Chart.yaml` | `version`, `appVersion` |
| `deploy/helm/inferra/values.yaml` | `image.tag` |

Runtime surfaces read the Rust workspace version at build time:

- CLI: `inferra --version`
- HTTP: `GET /api/version`

## Staging completed work

1. **During development** — land features on `main` without bumping `VERSION`.
2. **When a release slice is complete** — add a dated section to `CHANGELOG.md` under the target version (Added / Changed / Fixed).
3. **Before tagging** — bump `VERSION`, run sync + verify, finish release checklist.

```bash
# Edit VERSION (e.g. 0.3.0 -> 0.3.1)
python scripts/version.py sync
python scripts/version.py verify
```

4. **Tag** — `git tag -a v0.3.0 -m "Inferra 0.3.0"` after CI is green on `main`.
5. **Publish** — follow [release.md](release.md) for artifacts, SBOM, and GitHub Release notes copied from `CHANGELOG.md`.

### Semver rules for Inferra

| Bump | When |
| --- | --- |
| **PATCH** | Bug fixes, performance, CI/docs-only changes; no breaking operator contract |
| **MINOR** | New API routes, UI capabilities, collectors, or config keys with backward-compatible defaults |
| **MAJOR** | Breaking storage layout, removed API fields, or operator migration steps |

Pre-1.0 (`0.x.y`): **MINOR** may include breaking changes; document them prominently in `CHANGELOG.md`.

## CI enforcement

CI runs `python scripts/version.py verify` on every push and pull request. A mismatch fails the build.

## Windows packaging

`deploy/windows/read_pyproject_version.py` prints the canonical `VERSION` file for PowerShell helpers (`InferraWindows.psm1`, install scripts).
