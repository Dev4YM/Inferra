# Release checklist (v0.2.0)

Use this list before tagging **v0.2.0** and publishing GitHub Release assets.

## Quality gates

1. **Tests**: `python -m pytest -q` (full matrix locally or via CI) and `python -m pytest -q -m chaos` on Linux (SIGKILL SQLite scenario).
2. **Rust checks**: `cargo fmt --manifest-path src/Cargo.toml --all --check`, `cargo clippy --manifest-path src/Cargo.toml --workspace --all-targets -- -D warnings`, `cargo test --manifest-path src/Cargo.toml --workspace`, and `cargo build --manifest-path src/Cargo.toml -p inferra-cli --release`.
3. **Frontend + docs**: `npm ci && npm run build` in `src/web/frontend`, plus `mkdocs build --strict`.
4. **Python support checks**: `python -m compileall tests deploy deprecated` and `python -m ruff check tests deploy deprecated`.
5. **Native runtime smoke**: run `python tests/scripts/rust_runtime_smoke.py --binary ./target/release/inferra --repo-root .` after the frontend build so the built Rust CLI serves the real UI bundle and responds on `/api/health`, `/api/overview`, and `/api/collectors`.
6. **Repository readiness**: confirm ignored-artifact notes are intentional and that release docs match the current Rust-first command surface.
7. **Performance**: `python -m pytest -q -m perf` with `PERF_REPORT_PATH` set; confirm budgets in `tests/perf/test_budgets.py` still pass.

## Documentation

8. **Threat model**: confirm `docs/security/threat_model.md` reflects current binding, auth, CSP, and redaction behavior.
9. **CHANGELOG**: update `CHANGELOG.md` for v0.2.0 user-visible changes.
10. **Roadmap**: tick the resilience slice in `docs/implementation_roadmap.md`.

## Versioning and artifacts

11. **Version**: both `pyproject.toml` and `src/Cargo.toml` workspace version match the tag (0.2.0).
12. **Git tag**: `git tag -a v0.2.0 -m "Inferra 0.2.0"` after a green main branch.
13. **Artifacts**: build the native Rust Windows bundle, Helm chart, Rust CycloneDX SBOM, and the container image; sign where policy requires (cosign for images, signtool for Windows binaries).
14. **GitHub Release**: upload artifacts, attach the Rust SBOM produced by CI, publish release notes from `CHANGELOG.md`.

## Post-release

15. **Container registry**: push versioned image tags (`:v0.2.0`, `:0.2`) alongside `:latest` if applicable.
16. **Announce**: note breaking changes (storage layout, API fields such as expanded `/api/health`) for operators upgrading from v0.1.x.
