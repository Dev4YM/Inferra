# ADR 0004: Windows-First Collectors

## Status

Accepted

## Context

Inferra targets developers and operators on Windows desktops and Windows Server as often as Linux or Kubernetes. Collectors must behave correctly without mandatory POSIX-only APIs, and optional dependencies such as pywin32 must not break imports on non-Windows platforms.

## Decision

- First-class collectors exist for Windows Event Log and Windows service state, alongside shared collectors for host metrics and process snapshots.
- Windows-specific code paths live under `src/collectors/` with runtime guards; imports remain safe when pywin32 is absent.
- CLI one-shot commands (`collect-eventlog`, `collect-services`) are registered only where supported; attempting them on the wrong OS returns a clear error instead of importing Win32 modules at module load time.

## Consequences

- Documentation and presets treat Windows Server scenarios as equal to Linux node scenarios.
- CI runs Windows-marked tests on Windows runners without forcing Linux-only collectors on that platform.
