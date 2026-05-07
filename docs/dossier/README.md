# Inferra Reset Dossier

This dossier is the working source of truth for rebuilding Inferra into a polished, AI-integrated local control plane.

The current product now has a strong engine and the first real control-plane shell. It collects, stores, correlates, reasons, explains, maps workspace context, and exposes AI-assisted investigation through both CLI and web. The next phase is not about proving the direction; it is about making the direction feel polished, coherent, and release-grade.

## Product Thesis

Inferra should become a local-first runtime intelligence control plane.

It observes systems, builds evidence, investigates failures, explains what matters, links runtime behavior back to workspace context, and guides the user through next steps without taking destructive action. Runtime failure explanation remains a major feature, but it is no longer the whole identity.

The primary product promise:

> Inferra helps anyone understand what is happening in their local or server runtime, why it matters, what evidence supports it, and what to inspect next.

## Non-Negotiables

- CLI is the main control plane.
- Web is the visual investigation and operations control plane.
- AI drives investigation flow, prioritizes next steps, and suggests safe actions.
- AI does not execute remediation or mutate observed systems.
- Workspace and code awareness are core, not an add-on.
- Operator mode is the default user experience.
- Developer mode exists everywhere: CLI and web.
- The repo must look professional: no dropped artifacts, no duplicate UI generations, no committed dependency folders, no ambiguous ownership.

## Dossier Map

- [Product Vision](product/product_vision.md)
- [Modes and Personas](product/modes_and_personas.md)
- [Current State Audit](architecture/current_state_audit.md)
- [Target Architecture](architecture/target_architecture.md)
- [Repository Reset Plan](architecture/repository_reset_plan.md)
- [CLI Control Plane](control_plane/cli_control_plane.md)
- [Web Control Plane](control_plane/web_control_plane.md)
- [AI Investigation System](ai/ai_investigation_system.md)
- [Workspace Intelligence](workspace/workspace_intelligence.md)
- [Execution Roadmap](execution/execution_roadmap.md)
- [Acceptance Gates](execution/acceptance_gates.md)
- [Open Decisions](governance/open_decisions.md)

## Immediate Direction

The reset implementation has completed the first meaningful pass. The next implementation phase should harden and polish the control plane:

1. Continue hardening the official React source now under `src/web/frontend`.
2. Keep refining the native Rust control plane across `src/crates/inferra-api/`, `src/crates/inferra-cli/`, `src/crates/inferra-core/`, and `src/crates/inferra-storage/`.
3. Expand browser coverage for the Overview, AI Investigator, Workspace, Control, and Settings screens.
4. Reconcile web mode persistence with persisted config mode.
5. Deepen incident, service, evidence, and workspace drilldowns so the UI feels less like a shell and more like an investigator.
6. Keep frontend dependencies and build caches ignored while intentionally packaging `src/web/ui_dist`.
