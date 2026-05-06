# Product Vision

## Historical Naming Problem

The README previously positioned Inferra as:

> Inferra is a local-first runtime failure explanation system.

That was too narrow for the intended product. Failure explanation is one feature. It should not define the whole tool.

The product is becoming broader:

- local observability cockpit
- AI-assisted incident investigator
- runtime researcher
- workspace-aware diagnostics assistant
- control plane for collectors, config, service state, AI provider, and evidence review

## Recommended Positioning

Recommended main description:

> Inferra is a local-first runtime intelligence control plane that observes systems, investigates incidents, explains evidence, and guides next steps without sending operational data away by default.

Short variants:

- Local-first runtime intelligence control plane.
- AI-assisted local observability and investigation cockpit.
- Smart observer and investigator for local, server, and workspace runtime systems.

## Product Pillars

### Observe

Inferra watches runtime signals:

- logs
- events
- host metrics
- processes
- services
- containers
- Kubernetes events
- application HTTP events
- workspace project markers

The observation layer is read-only. It gathers facts and health signals.

### Understand

Inferra turns raw signals into structured knowledge:

- normalized events
- fingerprints
- service identities
- timelines
- anomalies
- incidents
- hypotheses
- evidence chains
- contradictions
- confidence and uncertainty

The system should always show how it reached a conclusion.

### Investigate

AI becomes an investigation driver:

- summarizes what matters now
- prioritizes next questions
- suggests safe next checks
- highlights missing evidence
- compares hypotheses
- explains why one thing outranks another
- links runtime symptoms to workspace context

AI must never silently change deterministic scores.

### Control

Inferra is managed from CLI first:

- onboarding
- profiles and modes
- config
- collectors
- service lifecycle
- AI provider
- storage
- exports
- diagnostics

The web UI is the visual control plane:

- dashboard
- incidents
- systems
- evidence
- AI investigator
- workspace
- settings

### Adapt

The product must serve both new and experienced users.

Operator mode:

- plain language
- high signal
- guided next steps
- less raw data by default

Developer mode:

- full raw payloads
- event filters
- scoring internals
- graph data
- config diffs
- collector internals
- API and storage detail

## Product Scope

Inferra should be:

- local-first
- read-only toward observed systems
- AI-integrated
- workspace-aware
- cross-platform
- useful with or without AI
- useful with or without web UI
- useful for developers, operators, server admins, and technical power users

Inferra should not become:

- an auto-remediation system
- a cloud observability replacement
- a hidden agent that changes systems
- a chat wrapper around logs
- a dashboard that only lists services and events

## Product Promise

The user should be able to open Inferra and immediately answer:

- What is happening?
- What changed?
- What is unhealthy?
- What is likely related?
- What is the evidence?
- What is the confidence?
- What should I inspect next?
- What is safe to ignore?
- What workspace or code context may explain this?
