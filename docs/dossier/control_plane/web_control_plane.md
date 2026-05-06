# Web Control Plane

The web UI should become the visual investigation cockpit. It should not be a thin wrapper around JSON endpoints.

## Landing Experience

The first screen should be a comprehensive dashboard with an overview tab. It should answer:

- Is anything broken?
- What is the top concern?
- What changed recently?
- What is Inferra confident about?
- What is Inferra uncertain about?
- What should I inspect next?
- Are collectors and AI healthy?
- Which workspace projects are connected?

## Global Layout

Recommended top-level navigation:

```text
Overview
Incidents
Systems
Evidence
AI Investigator
Workspace
Control
Settings
```

Operator mode should be default.

Developer mode should be a visible toggle and should affect all views.

## Overview Tab

Operator mode:

- overall status
- top incident
- top services needing attention
- AI next-step summary
- recent changes
- collector health summary
- workspace mapping summary

Developer mode:

- queue depth
- storage paths
- database state
- collector internals
- event rate
- anomaly state
- API latency
- model/provider metadata

## Incidents

Incident list should show:

- state
- severity
- primary service
- confidence
- short title
- latest update
- evidence count
- AI investigation state

Incident detail should show:

- human summary
- timeline
- hypotheses
- evidence
- contradictions
- affected services
- workspace links
- AI next steps
- chat/investigation panel
- developer raw tab

## Systems

Systems should merge:

- services
- processes
- containers
- host metrics
- topology
- source health

The user should be able to inspect a service and see:

- current status
- event volume
- anomalies
- incidents
- dependencies
- related workspace project
- recent evidence

## Evidence

Evidence is more than logs.

It should include:

- normalized events
- raw payload previews
- fingerprints
- anomaly signals
- process snapshots
- collector events
- service state changes
- topology edges
- AI traces

Developer mode should allow filtering by service, severity, source, fingerprint, incident, and time.

## AI Investigator

This should be a first-class workspace, not just model status.

Capabilities:

- ask about current state
- investigate latest incident
- generate next-step plan
- compare hypotheses
- explain evidence
- list missing signals
- produce operator summary
- produce developer report

The UI must show:

- what AI used as evidence
- what it did not know
- whether prompt was redacted
- whether provider is local or remote
- whether suggestions are safe read-only checks

## Workspace

Workspace view should show:

- detected projects
- project type
- service mapping confidence
- Docker Compose files
- package metadata
- likely commands
- related incidents
- runtime processes connected to repo paths

Workspace awareness should feel core to the product.

## Control

Control view should manage Inferra itself:

- setup status
- mode
- collectors
- AI provider
- service state
- storage
- backups
- config

It should not manage observed systems.

## Settings

Settings should not be raw JSON first.

Operator mode:

- guided cards
- toggles
- presets
- validation messages

Developer mode:

- full TOML/JSON editor
- config diff
- validation report
- reset/export/import

## Design Direction

This is an operational tool. It should feel:

- calm
- dense but readable
- fast
- serious
- helpful
- transparent

Avoid:

- marketing-style hero sections
- decorative dashboards
- giant cards everywhere
- raw JSON as the primary interface
- unclear AI magic
