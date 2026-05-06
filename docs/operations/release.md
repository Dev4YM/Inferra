# Release checklist (v0.2.0)

Use this list before tagging **v0.2.0** and publishing GitHub Release assets.

## Quality gates

1. **Tests**: `python -m pytest -q` (full matrix locally or via CI) and `python -m pytest -q -m chaos` on Linux (SIGKILL SQLite scenario).
2. **Static checks**: `python -m compileall src tests` and `python -m ruff check src tests`.
3. **Performance**: `python -m pytest -q -m perf` with `PERF_REPORT_PATH` set; confirm budgets in `tests/perf/test_budgets.py` still pass.

## Documentation

4. **MkDocs**: `python -m pip install -e ".[docs]"` then `mkdocs build --strict` (or project-standard doc build).
5. **Threat model**: confirm `docs/security/threat_model.md` reflects current binding, auth, CSP, and redaction behavior.
6. **CHANGELOG**: update `CHANGELOG.md` for v0.2.0 user-visible changes.
7. **Roadmap**: tick the resilience slice in `docs/implementation_roadmap.md`.

## Versioning and artifacts

8. **Version**: `pyproject.toml` project `version` matches the tag (0.2.0).
9. **Git tag**: `git tag -a v0.2.0 -m "Inferra 0.2.0"` after a green main branch.
10. **Artifacts**: build wheel/sdist (and platform packages per `docs/operations/release_signing.md`); sign where policy requires (cosign for images, signtool for Windows binaries).
11. **GitHub Release**: upload artifacts, attach SBOM if produced by CI, publish release notes from `CHANGELOG.md`.

## Post-release

12. **Container registry**: push versioned image tags (`:v0.2.0`, `:0.2`) alongside `:latest` if applicable.
13. **Announce**: note breaking changes (storage layout, API fields such as expanded `/api/health`) for operators upgrading from v0.1.x.
