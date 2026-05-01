# Inferra

## Tagline
Understand why your system fails — instantly.

---

## What is Inferra?

Inferra is a local-first AI debugging radar that observes runtime signals (logs, containers, services), builds causal hypotheses about system failures, and presents ranked, evidence-backed explanations.

It does NOT fix systems.  
It does NOT act autonomously.  
It only explains what is happening and why.

---

## Core Idea

Instead of reading logs manually or relying on generic observability dashboards, Inferra:

1. Ingests runtime events
2. Correlates them across services and time
3. Builds a causal graph of possible failure reasons
4. Generates multiple hypotheses
5. Scores and ranks them deterministically
6. Uses an LLM only to explain results clearly

---

## Key Feature

> Hypothesis-driven debugging, not log summarization.

---

## Scope

Optimized for:
- Backend APIs
- Dockerized services
- Node.js / Python systems

---

## Architecture Overview

Event Stream → Correlation Engine → Incident Clustering → Hypothesis Generator → Scoring Engine → Explanation Layer (LLM)

---

## Key Output

When a failure occurs, Inferra outputs:

- Primary cause hypothesis
- Confidence score (deterministic)
- Supporting evidence logs
- Timeline of events
- Alternative hypotheses
- Suggested debugging steps

---

## Non-Goals

- No system modification
- No auto-remediation
- No cloud dependency
- No alerting system replacement

---

## Why Inferra Exists

Because debugging failures is not a data problem —  
it is a reasoning problem.

Inferra compresses that reasoning.
