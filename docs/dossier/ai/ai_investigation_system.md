# AI Investigation System

AI should move from "optional explanation layer" to "investigation assistant." It still must respect the read-only safety model.

## AI Responsibilities

AI may:

- summarize current runtime state
- explain incidents
- compare hypotheses
- prioritize next inspection steps
- ask clarifying questions
- identify missing evidence
- generate safe commands for the user to run manually
- explain config implications
- translate developer-level signals into human language
- produce reports

AI may not:

- execute commands
- restart services
- edit files
- change config silently
- change deterministic scores
- create hidden conclusions without evidence
- present guesses as facts

## Investigation Flow

Target flow:

```text
User opens dashboard or runs CLI
  -> Inferra builds current situation snapshot
  -> deterministic engine ranks incidents and hypotheses
  -> AI receives structured evidence bundle
  -> AI returns investigation plan
  -> UI/CLI displays prioritized next steps
  -> user asks follow-up or inspects evidence
  -> AI updates plan from new context
```

Implemented first pass:

- The native Rust API/runtime builds structured investigation outputs with deterministic fallback behavior and persistence hooks.
- The shipped HTTP surface exposes investigation, ask, doctor, report, and trace paths through the active Rust product contract.
- The native Rust CLI exposes AI investigation, ask, report, doctor, and trace commands.
- The React control plane includes an AI Investigator workspace that consumes these APIs.

## Evidence Bundle

AI should receive structured bundles:

```json
{
  "mode": "operator",
  "incident": {},
  "hypotheses": [],
  "events": [],
  "services": [],
  "runtime": {},
  "workspace": {},
  "constraints": {
    "read_only": true,
    "do_not_execute": true,
    "must_cite_evidence": true
  }
}
```

## AI Output Contract

AI investigation output should be structured:

```json
{
  "headline": "Short summary",
  "risk_level": "low|medium|high|critical",
  "confidence": 0.0,
  "what_happened": [],
  "why_it_matters": [],
  "likely_causes": [],
  "evidence": [],
  "missing_evidence": [],
  "next_steps": [
    {
      "title": "Check recent service errors",
      "reason": "Evidence shows error spike after restart",
      "safety": "read_only",
      "command": "inferra services events api --limit 25",
      "requires_user_action": true
    }
  ],
  "uncertainty": [],
  "citations": []
}
```

## Investigation Commands

Implemented CLI:

```powershell
inferra investigate now
inferra investigate latest
inferra investigate incident inc-123
inferra investigate service api
inferra services events api --limit 25
inferra incidents show inc-123
inferra ai doctor
inferra ai ask "what should I inspect first?"
inferra ai investigate latest
inferra ai investigate incident inc-123
inferra ai investigate service api
inferra ai report inc-123 --mode operator
inferra ai report inc-123 --mode developer
inferra ai trace inc-123
```

The AI commands are read-only. They may propose safe checks for the user to run, but they do not execute remediation.

## Web AI UX

The AI panel should be attached to:

- overview
- incident detail
- service detail
- workspace detail

It should always show:

- evidence used
- safety boundary
- provider mode
- confidence
- next steps

## Prompt Trace

Developer mode must expose:

- system prompt
- sanitized user prompt
- redaction summary
- model
- latency
- token limits if available
- guardrail warnings

## Remote Provider Safety

If `ai.allow_remote = true`, the UI and CLI should show clear warnings:

- remote provider enabled
- token env configured or missing
- raw log redaction state
- paths/IP/env redaction state

## Guardrail Rule

Every AI conclusion must connect back to deterministic evidence, runtime context, or explicit uncertainty.
