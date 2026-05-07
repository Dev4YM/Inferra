# Rust Migration Matrix

This matrix records the historical migration decisions that produced the current
Rust-first layout. Treat it as a cleanup ledger, not as a guide to the live
`src/` runtime.

## Categories

- `completed-port`: the Rust workspace now owns the active path.
- `archived-legacy`: compatibility-only or historical surfaces that remain under
  `deprecated/`.

## Matrix

| Area | Current location | Target | Notes |
| --- | --- | --- | --- |
| Public HTTP API | `src/crates/inferra-api/` | `completed-port` | Rust Axum owns the public API surface. |
| Operator CLI | `src/crates/inferra-cli/` | `completed-port` | Rust CLI is the primary operator interface. |
| Windows service runtime | `src/crates/inferra-windows-service/` | `completed-port` | Native Rust service integration replaced the pywin32 path. |
| Collectors / ingest / storage / incidents | `src/crates/inferra-collectors/`, `src/crates/inferra-storage/`, `src/crates/inferra-core/` | `completed-port` | Native collectors, incident lifecycle, and investigation heuristics are live. |
| AI provider integration and investigation responses | `src/crates/inferra-api/`, `src/crates/inferra-core/` | `completed-port` | Rust owns provider probes, prompting, fallback behavior, and persisted audit artifacts. |
| React frontend | `src/web/frontend/`, `src/web/ui_dist/` | `keep` | React stays; the backend/runtime underneath it is Rust-first. |
| Legacy PyInstaller build path | `deprecated/windows-pyinstaller/` | `archived-legacy` | Archived for historical fallback only. |
| Legacy compatibility CLI / service helpers | `deprecated/inferra_legacy/` | `archived-legacy` | Kept only for compatibility reference and the archived legacy entry point. |

## Rule

Move a file into `deprecated/` only when:

1. The Rust/frontend runtime already owns the active path.
2. No shipped script, installer, or runtime artifact still invokes the legacy path.
3. The replacement is documented and verified.
