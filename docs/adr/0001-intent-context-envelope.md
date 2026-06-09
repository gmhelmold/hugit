# ADR-0001 — Intent context envelope (`context.json`)

- **Status:** Proposed (techlead-decided; owner-ratify the 3 knobs in §7)
- **Date:** 2026-06-09
- **Applies to:** hugit (produces + freezes the schema) · githugr (displays it)
- **Supersedes:** the narrative "Context Snapshot" sketch in whitepaper §4
  (`model, charter_ref, files_read[], tool_calls[], conversation_ref,
  journal_ref, env_manifest`) — this ADR is its first formal definition.

## 1. Context

In hugit an **intent is an enriched commit** (intent = commit, the hugr-native
unit; a PR groups intents). The commits are almost always produced by agents
that **spawn and die per intent** — the run that authored the change is gone by
the time a human or another agent reads it. The `context.json` carried by every
intent (behind `IntentSidecar.context_ref`, a CAS pointer) is therefore the
**only durable record of how the change came to exist**.

Today that record is underspecified (whitepaper sketch only) and captures none
of: the agent's transcript, token spend, wall-clock, or tool-call counts.
Without them we cannot answer the questions the forge exists to answer —
*why does this line exist, what did it cost, can I trust it* — and we cannot
roll those up to a PR. This ADR freezes the envelope.

Grounding (current code, for additivity): `IntentSidecar` is frozen with a
`context_ref: String` (`crates/hugit-contracts/src/intent_sidecar.rs`);
`Journal` exists in `hugit-ledger` keyed by `(tenant_id, workspace_id,
intent_id)`; `duration_ms` exists only on `CheckResult`; `model: String` is the
established field name (`VerdictObject`, `AttestationChain`); `REDACTED_MARKER`
is the established scrub token. **No** `context.json` struct exists yet — this
freezes a new contract, not a breaking change to an existing one.

## 2. Decision

`context.json` is the **Intent Context Envelope**: small structured metadata +
metrics held **inline**, with **content-addressed refs (`cas:…`)** to the large
blobs (transcripts, journal, prompt). Transcripts are never inlined — they are
stored once by content hash (deduped across intents), referenced here, scrubbed
by the redaction policy, retention-bounded, **tenant-private, never training
data** (whitepaper §13). One envelope per intent; PRs roll up by aggregation.

### 2.1 The trajectory — three altitudes (this is the core ask)

The agent's life is captured at three altitudes so meaning is preserved whether
you skim or audit (the "meaning at every altitude" doctrine):

| Altitude | Field | What it is | Size | Storage |
|---|---|---|---|---|
| **Full** | `raw_transcript_ref` | The complete agent loop, **born → die**: every model turn, every tool call **+ result**, system/charter prompts (redacted). Forensic, replayable. | MB | `cas:` blob |
| **Task** | `task_transcript_ref` | Task-scoped, mid-altitude: the brief in, the plan/step progression, key decisions, the result/handoff out. "What happened" without every token. | KB | `cas:` blob |
| **Summary** | `summary` | An LLM digest, a few sentences: *"Re-derived `iat` from `now()` to fix the refresh window; read 2 files; 14 tool calls; 3/3 verdicts green."* For the drawer. | bytes | inline |

`journal_ref` (the append-only `hugit-ledger` Journal) is retained as the
human-annotation track alongside the machine trajectory.

### 2.2 Full envelope schema

```jsonc
{
  "schema_version": "1.0.0",          // semver; deny_unknown_fields on freeze
  "intent_id": "a31",                 // == IntentSidecar.intent_id
  "commit": "a31f9c…",                // git commit this intent enriches
  "tree_hash": "…",

  // — provenance: who/what authored the commit (agents are ephemeral) —
  "authorship": {
    "model": "opus-4.8",              // field name consistent w/ VerdictObject
    "model_digest": "…",              // pinned model version
    "agent_type": "implementer",      // subagent type, or "main"
    "spawn": {
      "run_id": "…",
      "parent_run_id": "…",           // the orchestrator that spawned it (nullable)
      "born_at": 0, "died_at": 0      // unix ms — the agent's lifespan
    },
    "operator": "gustavo@humangr.com" // human principal who dispatched
  },

  // — intent: the "why" —
  "charter": "fix: refresh reusava o iat antigo → janela curta",
  "campaign": "auth-hardening",       // nullable; drives subliminal grouping
  "constraints": [],
  "acceptance": ["dura o TTL completo"],
  "parent_intents": [],

  // — trajectory: three altitudes (§2.1), tenant-private, redacted —
  "trajectory": {
    "raw_transcript_ref": "cas:7e1a…",
    "task_transcript_ref": "cas:9b2c…",
    "summary": "Re-derivou iat de now() … 14 tool calls … verdicts verdes.",
    "journal_ref": "cas:a902…",
    "redaction_policy": "default-v1"  // which policy scrubbed secrets/PII
  },

  // — snapshot: what it read / the environment —
  "snapshot": {
    "files_read": [{ "path": "auth/token.rs", "hash": "sha256:9c2f…" }],
    "prompt_ref": "cas:3b… (redacted)",
    "env_manifest": "rustc 1.96.0"
  },

  // — metrics: NEW; per-intent measurement (§2.3) —
  "metrics": {
    "tokens": { "input": 0, "output": 0, "cache_read": 0, "cache_write": 0, "total": 0 },
    "wall_ms": 0,                     // born → die wall-clock
    "active_ms": 0,                   // model+tool busy time (excludes idle)
    "tool_calls": 14,
    "tool_breakdown": [{ "tool": "Edit", "count": 6 }, { "tool": "Bash", "count": 5 }],
    "model_turns": 0,
    "cost_usd": 0.04                  // derived COGS; NOT what the customer is billed
  },

  "verdicts_ref": "cas:…"             // adversarial panel (VerdictObject) — nullable
}
```

### 2.3 Metrics — per intent and per PR

**Per intent:** `metrics` above — tokens (with cache split), wall-clock,
active time, tool-call count + breakdown, model turns, derived cost.

**Per PR (rollup):** PRs group intents; the forge computes the rollup, it is
not stored in each envelope. Sums across the PR's intents, plus a calendar span:

```jsonc
{
  "pr_id": "128",
  "intent_count": 3,
  "tokens_total": { "input": 0, "output": 0, "total": 0 },
  "tool_calls_total": 0,
  "model_turns_total": 0,
  "cost_usd_total": 0.04,
  "wall_ms_span": 0,    // first intent born → last intent died (clock time elapsed)
  "wall_ms_sum": 0,     // Σ per-intent wall (agent-time spent; > span when parallel)
  "models_used": ["opus-4.8", "sonnet-4.6"],
  "ci": { "cache_hit": 11, "exec_count": 0, "cost_usd": 0.00 }  // memoization economics
}
```

**Span vs sum is deliberate and a product asset:** `wall_ms_span` = *"this PR
took 14 min of clock time"* (the headline, reflecting fleet parallelism);
`wall_ms_sum` = *"3 h of agent-time went into it"* (the compute behind it). The
forge shows both; no other forge can.

## 3. Privacy, redaction, capture levels

Transcripts may carry secrets/PII (whitepaper §13). Therefore:

- **Always tenant-private, never cross-tenant, never training data.** Scrubbed
  with `REDACTED_MARKER` per `redaction_policy` before the blob is written.
- **Retention-bounded** per repo policy (TTL on `cas:` trajectory blobs).
- **Capture level is repo-configurable** — the cost/privacy dial:

  | Level | Stores | Use |
  |---|---|---|
  | `off` | envelope metadata only | max privacy / min storage |
  | `metrics` | + `metrics` | cost/time visibility, no transcript |
  | `task` | + `task_transcript_ref`, `summary` | the default sweet spot |
  | `full` | + `raw_transcript_ref` | full forensics / replay |

  Refs absent below their level are `null`; consumers must tolerate nulls.

## 4. Why (rationale)

- **Ephemeral authors demand a durable record.** The agent is gone; the envelope
  is the institutional memory of the change. Why-blame, replay, and trust all
  read from it.
- **Refs not blobs** keeps the envelope small and the heavy transcripts deduped
  by content (CAS) — consistent with "memoize by content, price flat".
- **Metrics are first-class** because the forge's job is to make agent work
  *legible and accountable*; cost/time/tool-calls per intent and per PR are the
  vocabulary of that accountability — and cost here is **COGS shown for trust,
  never a usage meter** (no billing whiplash).
- **Three altitudes** serve the reader loop (*changed → why → safe*) without
  forcing anyone to read a raw MB transcript to understand a one-line fix.

## 5. Consequences

- **New frozen contract in `hugit-contracts`:** a `ContextEnvelope` type (with
  `schema_version`, `#[serde(deny_unknown_fields)]`, golden round-trip test),
  plus `IntentMetrics` and a derived (not-frozen, forge-computed) `PrRollup`.
  Additive to `IntentSidecar` (it already holds `context_ref`).
- **Producers must emit metrics:** the runner/agent harness must report tokens,
  wall/active ms, tool-call breakdown, and turns at intent close. (New WP under
  the runner/journal area; sizing per memory `agent-task-sizing`.)
- **githugr display obligation:** surface per-intent and per-PR metrics, and the
  three trajectory altitudes (summary inline → task → full, progressively
  disclosed). Recorded in the githugr companion ADR; the design mockups
  (`landing.html` drawer `context.json` block, `pr-detail.html`) must be updated
  to show the new envelope + a metrics strip.
- **Redaction is on the write path,** not the read path — the blob is scrubbed
  before it is stored.

## 6. Alternatives considered

- **Inline the transcript in `context.json`** — rejected: envelopes become MB,
  no dedupe, git/CAS bloat. Refs win.
- **One flat transcript only** — rejected: forces reading everything to learn
  anything; the three altitudes are the whole point.
- **Skip cost/time metrics** — rejected: the forge's differentiator is making
  agent work accountable; omitting metrics guts the value.

## 7. Open for owner ratification

1. **Default capture level** — recommend `task` (summary + task transcript +
   metrics; raw transcript opt-in per repo). Confirm or change.
2. **Default retention TTL** for `full` raw-transcript blobs — recommend 90 days
   then GC to `task` (keep summary/metrics forever, they're tiny). Confirm.
3. **Is `cost_usd` shown to all repo members, or owner/admin only?** — recommend
   visible to all (trust by transparency); confirm given it reveals COGS.
