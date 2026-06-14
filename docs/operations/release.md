# Release checklist

Use this list before tagging a release. The canonical version is in [`VERSION`](../../VERSION); policy and semver rules are in [versioning.md](versioning.md).

Replace `{version}` below with the contents of `VERSION` (e.g. `0.3.0`) and tag `v{version}` (e.g. `v0.3.0`).

## Quality gates

1. **Version sync**: `python scripts/version.py verify` (also enforced in CI).
2. **Tests**: `python -m pytest -q` (full matrix locally or via CI) and `python -m pytest -q -m chaos` on Linux (SIGKILL SQLite scenario).
3. **Rust checks**: `cargo fmt --manifest-path src/Cargo.toml --all --check`, `cargo clippy --manifest-path src/Cargo.toml --workspace --all-targets -- -D warnings`, `cargo test --manifest-path src/Cargo.toml --workspace`, and `cargo build --manifest-path src/Cargo.toml -p inferra-cli --release`.
4. **Frontend + docs**: `npm ci && npm run build` in `src/web/frontend`, plus `mkdocs build --strict`.
5. **Python support checks**: `python -m compileall tests deploy` and `python -m ruff check tests deploy`.
6. **Native runtime smoke**: run `python tests/scripts/rust_runtime_smoke.py --binary ./target/release/inferra --repo-root .` after the frontend build so the built Rust CLI serves the real UI bundle and responds on `/api/health`, `/api/overview`, and `/api/collectors`.
7. **Repository readiness**: confirm ignored-artifact notes are intentional and that release docs match the current Rust-first command surface.
8. **Performance**: `python -m pytest -q -m perf` with `PERF_REPORT_PATH` set; confirm budgets in `tests/perf/test_budgets.py` still pass.

## Documentation

9. **Threat model**: confirm `docs/security/threat_model.md` reflects current binding, auth, CSP, and redaction behavior.
10. **CHANGELOG**: add or finalize the `## {version}` section with user-visible Added / Changed / Fixed entries.
11. **Roadmap**: tick completed slices in `docs/implementation_roadmap.md` when applicable.

## Versioning and artifacts

12. **Bump**: set `VERSION`, run `python scripts/version.py sync`, commit the synced manifests.
13. **Git tag**: `git tag -a v{version} -m "Inferra {version}"` after a green `main` branch.
14. **Artifacts**: build the native Rust Windows bundle, Helm chart, Rust CycloneDX SBOM, and the container image; sign where policy requires (cosign for images, signtool for Windows binaries).
15. **GitHub Release**: upload artifacts, attach the Rust SBOM produced by CI, publish release notes from `CHANGELOG.md`.

## Post-release

16. **Container registry**: push versioned image tags (`:v{version}`, `:{major}.{minor}`) alongside `:latest` if applicable.
17. **Announce**: note breaking changes (storage layout, API fields) for operators upgrading from earlier `0.x` releases.
