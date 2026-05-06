# Modes and Personas

Inferra should not pick one user type and force everyone else through that lens. It should provide modes that change the density, language, and workflows of both CLI and web.

## Modes

### Operator Mode

Default mode.

Purpose:

- make the system understandable fast
- reduce stress during incidents
- guide the next inspection step
- hide raw detail until needed

Characteristics:

- plain-language summaries
- health cards with clear severity
- incident narratives
- evidence previews
- recommended next checks
- safety labels on AI suggestions
- fewer raw JSON blocks
- confidence and uncertainty explained in human terms

Example CLI behavior:

```powershell
inferra status
inferra investigate now
inferra investigate latest
inferra incidents show inc-123
```

### Developer Mode

Advanced mode for users who want raw evidence and workspace-linked debugging.

Purpose:

- expose full system detail
- support debugging Inferra itself and the observed local project
- let advanced users inspect raw evidence
- allow precise filtering, export, and verification

Characteristics:

- raw events
- scoring components
- hypothesis weights
- timelines
- payload diffs
- prompt traces
- graph edges
- collector health internals
- storage diagnostics

Example CLI behavior:

```powershell
inferra mode set developer
inferra --json status
inferra incidents show inc-123
inferra services events api --limit 25
inferra ai trace inc-123
```

## Mode Configuration

Modes should be configurable during onboarding:

```powershell
inferra setup --mode operator
inferra onboard --mode operator --ai-role investigator
inferra setup --mode developer
```

Mode should also be switchable later:

```powershell
inferra config set experience.mode developer
inferra mode set operator
inferra mode set developer
```

The web UI should expose a mode switch that persists to config or user-local preferences.

## Persona Coverage

### New User

Needs:

- setup guidance
- defaults that work
- plain-language dashboard
- no need to understand collectors immediately

Primary mode:

- Operator

### Developer

Needs:

- runtime-to-code connection
- local process/container context
- logs, traces, configs, projects
- useful AI questions

Primary mode:

- Developer

### Operator

Needs:

- health overview
- active incidents
- service status
- next checks
- safe reports

Primary mode:

- Operator

### Power User

Needs:

- raw detail
- filters
- exports
- full scoring visibility
- API parity

Primary mode:

- Developer

## Rule

Modes change presentation and workflow defaults. They must not change stored evidence, deterministic analysis, or safety boundaries.
