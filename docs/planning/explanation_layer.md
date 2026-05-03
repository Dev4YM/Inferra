# Explanation Layer

## Purpose

The explanation layer converts structured hypothesis data into human-readable narratives. It is the only component in Inferra that uses an LLM, and it is strictly constrained: the LLM explains what the deterministic engine already decided. It does not influence scoring, ranking, or hypothesis generation.

---

## Architectural Position

```
Scoring Engine → Ranked Hypotheses (structured data)
                          │
                          ▼
              ┌───────────────────────┐
              │ Explanation Layer      │
              │                       │
              │ 1. Build prompt        │
              │ 2. Call LLM            │
              │ 3. Validate response   │
              │ 4. Apply guardrails    │
              │ 5. Return explanation  │
              └───────────┬───────────┘
                          │
                          ▼
              ExplanationResult (human-readable text)
```

If the LLM is unavailable, the system falls back to template-based explanations. The UI displays "structured summary" instead of a natural language narrative. All functionality except the explanation quality is unaffected.

---

## LLM Provider Strategy

The explanation layer supports multiple LLM backends through a provider abstraction:

```python
class LLMProvider(Protocol):
    async def generate(self, prompt: str, max_tokens: int = 2000,
                        temperature: float = 0.3) -> str:
        ...

    def is_available(self) -> bool:
        ...

    @property
    def model_name(self) -> str:
        ...
```

### Supported Providers

| Provider | Configuration | Notes |
|---|---|---|
| Local Ollama | `ollama://localhost:11434/model_name` | Preferred. Fully local, no data leaves machine. |
| OpenAI-compatible API | `openai://api.openai.com/v1` | Requires API key. Data sent to remote server (sanitized). |
| Template fallback | `template://` | Always available. No LLM, structured text templates. |

**Provider selection order**:
1. User-configured provider (from `inferra.toml`)
2. Auto-detect local Ollama (check if port 11434 responds)
3. Template fallback (always succeeds)

```toml
[explanation]
provider = "ollama://localhost:11434/llama3"
# provider = "openai://api.openai.com/v1"
# api_key_env = "OPENAI_API_KEY"
fallback = "template://"
timeout_seconds = 30
max_retries = 2
temperature = 0.3         # low temperature for factual, consistent output
max_tokens = 2000
```

---

## LLM Boundary Enforcement

### What the LLM Receives (Explicitly Defined)

The LLM prompt is constructed from a **fixed schema**. The system never passes raw, unstructured data directly to the LLM. Everything is filtered through the `ExplanationRequest` contract (see `data_flow_contracts.md`).

**Allowed in prompt**:
| Data | Format | Example |
|---|---|---|
| Incident ID | String | `"inc-abc123"` |
| Time range | ISO 8601 timestamps | `"14:30:00Z to 14:35:00Z"` |
| Service names | Canonical identifiers | `"api-gateway"`, `"postgres"` |
| Severity levels | Enum names | `"ERROR"`, `"CRITICAL"` |
| Event summaries | Sanitized 1-line strings (max 200 chars each) | `"connection refused: too many clients"` |
| Hypothesis descriptions | Structured text from hypothesis engine | `"postgres failure cascaded to 3 services"` |
| Score breakdowns | Float values | `"temporal_alignment: 0.92"` |
| Contradictions | Structured descriptions | `"health check passed at 14:32"` |
| Topology edges | Service-to-service relations | `"api-gateway calls postgres"` |
| Resource summary | Summarized metrics | `"memory: 72%, CPU: 45%"` |

**Forbidden — never included in prompt**:
| Data | Reason |
|---|---|
| Raw log lines | May contain secrets, credentials, PII |
| IP addresses | Stripped during sanitization |
| Environment variable values | Stripped; only keys may appear |
| File paths (full) | Stripped to basenames only |
| API keys, passwords, tokens | Stripped by regex + known patterns |
| Structured data values from events | May contain request bodies, user data |
| Raw SQL queries from logs | May contain user data |
| Container environment blocks | Contains secrets |

### Context Budget

The prompt has a hard token budget to prevent context window overflow and reduce hallucination surface:

```python
MAX_PROMPT_TOKENS = 4000  # conservative; works with 7B local models
MAX_TIMELINE_ENTRIES = 30  # show top-30 events, not all 500
MAX_ALTERNATIVES = 3       # top 3 alternative hypotheses
MAX_CONTRADICTIONS = 5     # top 5 contradictions
MAX_TOPOLOGY_EDGES = 20    # topology summary, not full graph
```

Events included in the timeline are selected by: (1) key events flagged by the hypothesis, (2) highest severity, (3) one per service minimum, (4) chronological order. The selection algorithm is deterministic.

### Drift Prevention

Prompt drift occurs when the prompt template is modified (accidentally or intentionally) to include information that leaks structured data back into the reasoning pipeline.

**Enforced boundaries**:
1. The LLM response is **never parsed for structured data**. The `ExplanationResult` has text-only fields (`summary`, `primary_hypothesis_text`, etc.). No field is extracted from the LLM response and fed back into scoring, hypothesis generation, or incident state.
2. The prompt template is **version-controlled and frozen per release**. Changes to `EXPLANATION_PROMPT` require explicit review.
3. The LLM call is **fire-and-forget with respect to analysis state**. If the LLM call fails, times out, or returns garbage, the analysis pipeline is completely unaffected. The UI falls back to the template explanation.
4. **No multi-turn conversation**. Each explanation is a single prompt → single response. The LLM has no memory of previous incidents or explanations.

---

## Prompt Construction

### Input Sanitization

Before constructing the prompt, all data is sanitized:

```python
def sanitize_for_llm(request: ExplanationRequest) -> ExplanationRequest:
    """Strip sensitive data before sending to LLM."""
    sanitized = deepcopy(request)

    for entry in sanitized.timeline:
        entry.summary = strip_ips(entry.summary)
        entry.summary = strip_env_values(entry.summary)
        entry.summary = strip_file_paths(entry.summary, keep_basename=True)

    # Service names are kept (they are canonical identifiers, not secrets)
    # Timestamps are kept (needed for timeline narrative)
    # Severity levels are kept

    return sanitized
```

### Prompt Template

```python
EXPLANATION_PROMPT = """You are a systems reliability engineer analyzing an infrastructure incident.
You must explain what happened based ONLY on the evidence provided below. Do not invent or assume
any information not present in the data.

## Incident Summary
- Incident ID: {incident_id}
- Time range: {time_range_start} to {time_range_end}
- Affected services: {affected_services}
- Severity: {severity}

## Top Hypothesis (Rank 1, Score: {top_score})
- Type: {cause_type} / {cause_subtype}
- Description: {description}
- Confidence: {confidence_label}
- Supporting evidence: {supporting_count} events
- Contradicting evidence: {contradicting_count} events

## Evidence Timeline
{timeline_entries}

## Service Topology
{topology_description}

## Runtime Context
{runtime_summary}

## Contradictions
{contradictions_list}

## Alternative Hypotheses
{alternatives_list}

---

Produce the following sections:

### Summary
A 2-3 sentence overview of what happened and the most likely cause.

### What Happened (Timeline)
A chronological narrative of the incident. Reference specific timestamps and services.

### Most Likely Cause
Explain the top-ranked hypothesis in plain language. Describe the evidence chain from root cause to observed symptoms.

### Evidence Assessment
How strong is the evidence? What supports the hypothesis and what contradicts it?

### Other Possibilities
Brief mention of alternative hypotheses and why they ranked lower.

### Recommended Actions
Concrete next steps the operator should take to verify and resolve the issue.

### Uncertainty Notes
Where is the analysis uncertain? What additional data would help?

RULES:
- Only reference services, timestamps, and events present in the data above.
- Do not invent causes, services, or events not in the evidence.
- If evidence is weak or contradictions exist, say so explicitly.
- Be concise. Each section should be 2-5 sentences maximum.
- Do not use marketing language or vague qualifiers like "leveraging AI".
- Use technical terminology appropriate for a systems engineer audience.
"""
```

### Prompt Assembly

```python
def build_prompt(request: ExplanationRequest) -> str:
    sanitized = sanitize_for_llm(request)
    top_hyp = sanitized.top_hypotheses[0]

    timeline_text = "\n".join(
        f"- [{e.timestamp.strftime('%H:%M:%S')}] [{e.severity.name}] {e.service_id}: {e.summary}"
        + (" ⭐" if e.is_key_event else "")
        for e in sanitized.timeline
    )

    topology_text = "\n".join(
        f"- {r.source} {r.relation_type} {r.target}"
        for r in sanitized.service_topology
    )

    contradictions_text = "\n".join(
        f"- {c.contradiction_type}: {c.explanation}"
        for c in sanitized.contradictions
    ) or "None detected."

    alternatives_text = "\n".join(
        f"- (Rank {h.rank}, Score {h.total_score:.2f}) {h.cause_type.value}/{h.description[:100]}"
        for h in sanitized.top_hypotheses[1:]
    ) or "No alternatives."

    return EXPLANATION_PROMPT.format(
        incident_id=sanitized.incident_id,
        time_range_start=sanitized.timeline[0].timestamp.isoformat() if sanitized.timeline else "unknown",
        time_range_end=sanitized.timeline[-1].timestamp.isoformat() if sanitized.timeline else "unknown",
        affected_services=", ".join(top_hyp.affected_services),
        severity=top_hyp.cause_type.value,
        top_score=f"{top_hyp.total_score:.2f}",
        cause_type=top_hyp.cause_type.value,
        cause_subtype=getattr(top_hyp, "cause_subtype", ""),
        description=top_hyp.description,
        confidence_label=top_hyp.confidence_label,
        supporting_count=len(top_hyp.supporting_events),
        contradicting_count=len(top_hyp.contradicting_events),
        timeline_entries=timeline_text,
        topology_description=topology_text,
        runtime_summary=format_runtime_summary(sanitized.runtime_context_summary),
        contradictions_list=contradictions_text,
        alternatives_list=alternatives_text,
    )
```

---

## Response Validation (Guardrails)

The LLM response is post-processed through a guardrail pipeline before being presented to the user.

### Guardrail 1: Service Name Verification

Every service name mentioned in the response must exist in the input data.

```python
def verify_service_names(response: str, known_services: set[str]) -> tuple[str, list[str]]:
    """Check that all service references in the response are valid."""
    violations = []
    words = set(re.findall(r'\b[\w\-\.]+\b', response))
    # Heuristic: words that look like service names but aren't in known set
    for word in words:
        if (looks_like_service_name(word) and
            word not in known_services and
            word.lower() not in COMMON_TECHNICAL_TERMS):
            violations.append(f"Unknown service reference: '{word}'")
            response = response.replace(word, f"[unknown: {word}]")
    return response, violations
```

### Guardrail 2: Timestamp Verification

Timestamps mentioned in the response should correspond to actual events.

```python
def verify_timestamps(response: str, event_timestamps: set[str]) -> tuple[str, list[str]]:
    """Check that timestamps in the response correspond to real events."""
    violations = []
    mentioned_times = re.findall(r'\d{2}:\d{2}:\d{2}', response)
    for ts in mentioned_times:
        if not any(ts in evt_ts for evt_ts in event_timestamps):
            violations.append(f"Timestamp {ts} not found in event data")
    return response, violations
```

### Guardrail 3: Causation Claim Verification

Claims about what caused what must map to hypotheses that were actually scored.

```python
def verify_causal_claims(response: str, hypotheses: list[ScoredHypothesis]) -> tuple[str, list[str]]:
    """Check that causal claims in the response correspond to scored hypotheses."""
    violations = []
    CAUSAL_PHRASES = ["caused by", "led to", "resulted in", "triggered", "root cause"]

    for phrase in CAUSAL_PHRASES:
        if phrase in response.lower():
            # Verify the claimed cause maps to a hypothesis
            sentence = extract_sentence_containing(response, phrase)
            if not any(matches_hypothesis(sentence, h) for h in hypotheses):
                violations.append(f"Causal claim not backed by hypothesis: '{sentence[:100]}'")

    return response, violations
```

### Guardrail 4: Confidence Overstatement Check

The LLM must not express more confidence than the scoring engine assigned.

```python
OVERCONFIDENCE_PHRASES = [
    "definitely", "certainly", "100%", "guaranteed", "proven",
    "without a doubt", "clearly the cause", "must be",
]

def check_overconfidence(response: str, confidence_label: str) -> tuple[str, list[str]]:
    violations = []
    for phrase in OVERCONFIDENCE_PHRASES:
        if phrase in response.lower():
            violations.append(f"Overconfidence phrase detected: '{phrase}'")
            # Soften the language
            response = response.replace(phrase, "likely")
    return response, violations
```

### Guardrail 5: Structural Diff Check

The strongest guardrail. Instead of trying to catch hallucinations by pattern matching (which is fundamentally unreliable), verify that the explanation's **claims** are derivable from the input data.

```python
def structural_diff_check(response: str, request: ExplanationRequest,
                            hypotheses: list[ScoredHypothesis]) -> tuple[str, list[str]]:
    """Extract factual claims from the response and check each against input data.
    This catches fabricated technical details that pass name/timestamp checks."""
    violations = []

    # Extract claimed metric values (e.g., "memory at 94%", "CPU at 87%")
    metric_claims = re.findall(r'(\w+)\s+(?:at|was|reached|hit)\s+(\d+(?:\.\d+)?)\s*%', response)
    for metric_name, value in metric_claims:
        if not value_exists_in_context(metric_name, float(value), request.runtime_context_summary):
            violations.append(f"Fabricated metric: '{metric_name} at {value}%' not in runtime context")

    # Extract claimed event counts (e.g., "12 connection errors")
    count_claims = re.findall(r'(\d+)\s+(error|timeout|restart|connection|failure)s?', response, re.I)
    for count, event_type in count_claims:
        if not count_approximately_matches(int(count), event_type, request):
            violations.append(f"Fabricated count: '{count} {event_type}s' doesn't match evidence")

    # Extract claimed configuration details (e.g., "max_connections was reduced")
    config_claims = re.findall(r'(max_connections|memory_limit|timeout|pool_size|workers)\s+(?:was|set to|changed to|reduced to)\s+(\S+)', response, re.I)
    for config_key, config_value in config_claims:
        violations.append(f"Unverifiable config claim: '{config_key} = {config_value}' — no config data in evidence")

    return response, violations
```

### Guardrail Pipeline

```python
async def apply_guardrails(response: str, request: ExplanationRequest,
                             hypotheses: list[ScoredHypothesis]) -> tuple[str, list[str]]:
    all_violations = []
    known_services = set()
    for h in hypotheses:
        known_services.update(h.affected_services)

    response, v1 = verify_service_names(response, known_services)
    all_violations.extend(v1)

    timestamps = {e.timestamp.isoformat() for e in request.timeline}
    response, v2 = verify_timestamps(response, timestamps)
    all_violations.extend(v2)

    response, v3 = verify_causal_claims(response, hypotheses)
    all_violations.extend(v3)

    response, v4 = check_overconfidence(response, hypotheses[0].confidence_label)
    all_violations.extend(v4)

    response, v5 = structural_diff_check(response, request, hypotheses)
    all_violations.extend(v5)

    return response, all_violations
```

### What Guardrails Catch and What They Don't (Honest Assessment)

| Guardrail | Catches | Misses |
|---|---|---|
| Service name verification | Hallucinated service names | Correct names used in wrong context |
| Timestamp verification | Fabricated timestamps | Real timestamps attributed to wrong events |
| Causal claim verification | Causal claims with no matching hypothesis | Plausible causal chains using real entities |
| Overconfidence check | Obvious certainty language | Subtle overconfidence ("this clearly shows...") |
| Structural diff check | Fabricated metrics, counts, config values | Plausible-sounding details within ranges |

**Estimated overall hallucination catch rate: ~50–60%.** The remaining 40% are fabrications that use correct entity names, plausible values, and legitimate-sounding technical details. No post-hoc regex system can catch these reliably.

**Recommendation**: For high-stakes debugging (production outages), prefer the template fallback over LLM explanations. The template output is less readable but guaranteed to contain only data that actually exists. The LLM mode is best for learning and exploration, not for situations where a fabricated detail could send an operator down a wrong investigative path.

The UI should make this trade-off visible:
```
[LLM Explanation] — natural language, may contain inaccuracies
[Structured Summary] — data only, guaranteed accurate
```

---

## Template Fallback

When no LLM is available, explanations are generated from templates:

```python
def generate_template_explanation(request: ExplanationRequest) -> ExplanationResult:
    top = request.top_hypotheses[0]

    summary = (
        f"Incident affecting {', '.join(top.affected_services)}. "
        f"Most likely cause: {top.cause_type.value} ({top.description[:100]}). "
        f"Confidence: {top.confidence_label}. Score: {top.total_score:.2f}."
    )

    timeline_text = "\n".join(
        f"  [{e.timestamp.strftime('%H:%M:%S')}] {e.severity.name} | {e.service_id}: {e.summary}"
        for e in request.timeline
    )

    evidence_text = (
        f"Supporting evidence: {len(top.supporting_events)} events. "
        f"Contradicting evidence: {len(top.contradicting_events)} events."
    )

    actions = "\n".join(f"  - {check}" for check in top.suggested_checks)

    alternatives = "\n".join(
        f"  - Rank {h.rank} (score {h.total_score:.2f}): {h.description[:80]}"
        for h in request.top_hypotheses[1:]
    )

    return ExplanationResult(
        incident_id=request.incident_id,
        summary=summary,
        primary_hypothesis_text=f"{top.cause_type.value}: {top.description}",
        evidence_narrative=evidence_text,
        timeline_narrative=f"Event timeline:\n{timeline_text}",
        alternative_explanations=[h.description for h in request.top_hypotheses[1:]],
        suggested_actions=top.suggested_checks,
        uncertainty_notes=["LLM unavailable — showing structured summary"],
        generation_model="template_fallback",
        guardrail_violations=[],
    )
```

---

## Caching

Explanations are cached per incident version. If the incident hasn't changed since the last explanation was generated, the cached explanation is returned:

```python
def should_regenerate(incident: Incident, existing: ExplanationResult | None) -> bool:
    if existing is None:
        return True
    if incident.updated_at > existing.created_at:
        return True  # incident has new data
    return False
```

---

## Performance Budget

| Operation | Budget | Notes |
|---|---|---|
| Prompt construction + sanitization | <50ms | String formatting |
| LLM API call (local ollama) | 2–15 seconds | Depends on model size |
| LLM API call (remote) | 1–10 seconds | Network latency |
| Guardrail pipeline | <100ms | Regex scans |
| Template fallback | <10ms | String formatting only |

The explanation is generated asynchronously and does not block the analysis pipeline. The UI shows "Generating explanation..." while the LLM call is in-flight.

---

## Configuration

```toml
[explanation]
provider = "ollama://localhost:11434/llama3"
fallback = "template://"
timeout_seconds = 30
max_retries = 2
temperature = 0.3
max_tokens = 2000
cache_enabled = true

[explanation.sanitization]
strip_ips = true
strip_env_values = true
strip_paths = true
keep_service_names = true
keep_timestamps = true

[explanation.guardrails]
verify_service_names = true
verify_timestamps = true
verify_causal_claims = true
check_overconfidence = true
```
