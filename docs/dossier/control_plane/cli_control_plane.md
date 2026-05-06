# CLI Control Plane

The CLI is the primary control plane. The web UI should never be the only way to configure, inspect, or operate Inferra.

## CLI Principles

- Every major web action should have a CLI equivalent.
- Every CLI command should support human output and JSON output where useful.
- Operator mode should be friendly by default.
- Developer mode should expose raw detail.
- Commands should guide the next step.
- CLI should be safe: no observed-system mutation.

## Target Command Groups

```text
inferra setup
inferra onboard
inferra mode
inferra status
inferra dashboard
inferra investigate
inferra incidents
inferra events
inferra services
inferra collectors
inferra ai
inferra workspace
inferra config
inferra storage
inferra service
inferra export
inferra doctor
```

## Setup

Target:

```powershell
inferra setup
inferra onboard
inferra setup --yes
inferra setup --mode operator
inferra setup --mode developer
inferra setup --preset windows-server --mode operator --model gemma4:e4b
```

Setup should configure:

- data directory
- experience mode
- collector preset
- AI provider
- model
- local/remote AI policy
- service install recommendation
- workspace scan
- dashboard launch

## Mode

Target:

```powershell
inferra mode
inferra mode show
inferra mode set operator
inferra mode set developer
```

Mode affects:

- CLI output density
- default columns
- whether raw JSON appears
- AI prompt style
- dashboard default view

## Status and Dashboard

Target:

```powershell
inferra status
inferra dashboard
inferra dashboard --json
```

Operator output:

- overall state
- active incidents
- top concern
- AI availability
- collector status
- next recommended command

Developer output:

- database paths
- queue depth
- degraded reasons
- collector health internals
- storage state
- AI resolved model
- API reachability

## Investigation

Target:

```powershell
inferra investigate now
inferra investigate latest
inferra investigate service api
inferra investigate incident inc-123
inferra investigate workspace
```

Investigation should return:

- situation summary
- prioritized findings
- evidence list
- missing evidence
- suggested safe checks
- links to incidents/events/services
- confidence and uncertainty

Implemented first pass:

```powershell
inferra investigate now
inferra investigate latest
inferra investigate incident inc-123
inferra investigate service api
inferra investigate workspace
```

These commands are read-only. They prioritize evidence and print safe next commands, but they do not execute remediation.

## Runtime Inspection

Implemented first pass:

```powershell
inferra incidents list
inferra incidents show inc-123
inferra events list --limit 25
inferra events show evt-123
inferra services list
inferra services show api
inferra services events api --limit 25
inferra doctor
```

`doctor` combines local config validation with live API reachability and suggests safe next steps.

## AI

The first AI control-plane pass is implemented. AI is now available both as setup/status tooling and as an investigation assistant.

Target:

```powershell
inferra ai setup
inferra ai status
inferra ai models
inferra ai pull gemma4:e4b
inferra ai test
inferra ai doctor
inferra ai ask "what is most suspicious right now?"
inferra ai investigate latest
inferra ai investigate incident inc-123
inferra ai investigate service api
inferra ai report inc-123 --mode operator
inferra ai report inc-123 --mode developer
inferra ai trace inc-123
```

Investigation is exposed through both `inferra investigate ...` and `inferra ai investigate ...`. The former is the user-facing investigation namespace; the latter makes the AI role explicit for users who want that framing.

Implemented first pass:

```powershell
inferra ai doctor
inferra ai ask "what should I inspect first?"
inferra ai investigate latest
inferra ai investigate incident inc-123
inferra ai investigate service api
inferra ai report inc-123 --mode operator
inferra ai report inc-123 --mode developer
inferra ai trace inc-123
```

`ai doctor` should check:

- enabled flag
- provider URL
- local/remote policy
- token env
- installed models
- selected model availability
- latency
- prompt redaction settings
- remote safety warnings

## Workspace

Implemented first pass:

```powershell
inferra workspace
inferra workspace map
inferra workspace services
inferra workspace inspect D:\Projects\app
inferra investigate workspace
inferra services list
inferra services show api
```

Future target:

```powershell
inferra workspace explain
```

Workspace commands should become core because the product is not just observing runtime. It is connecting runtime behavior to local project context.

## Service

Inferra service commands manage Inferra itself.

Target:

```powershell
inferra service status
inferra service install --startup auto
inferra service start
inferra service stop
inferra service restart
inferra service remove
inferra service logs
inferra service repair
```

`service repair` should inspect:

- service runtime file
- config path
- data dir
- log path
- port conflicts
- missing exe
- permissions
- pywin32 availability

Implemented first pass:

```powershell
inferra service repair
```

The repair command is read-only: it inspects Inferra's own service setup and prints suggested corrective actions instead of mutating the observed system.

## JSON Contract

All investigation and control commands should support:

```powershell
inferra <command> --json
```

JSON output should be stable enough for scripts.

## Implementation Note

The first command split exists under `src/cli_core/commands/`. Keep moving parser registration and remaining legacy command glue out of `src/cli.py` before adding another large command family.
