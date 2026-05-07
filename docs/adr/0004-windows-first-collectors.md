# ADR 0004: Windows-First Collectors

## Status

Accepted

## Context

Inferra targets developers and operators on Windows desktops and Windows Server as often as Linux or Kubernetes. Collectors must behave correctly without mandatory POSIX-only APIs, and the active runtime must not depend on pywin32.

## Decision

- First-class collectors exist for Windows Event Log and Windows service state, alongside shared collectors for host metrics and process snapshots.
- Windows-specific code paths live in the Rust collector/runtime crates (`src/crates/inferra-collectors/` and `src/crates/inferra-windows-service/`); no active Python collector import path is required.
- Operator workflows use the native Rust CLI and API rather than Python-only one-shot commands.

## Consequences

- Documentation and presets treat Windows Server scenarios as equal to Linux node scenarios.
- CI runs Windows-marked tests on Windows runners without forcing Linux-only collectors on that platform.
