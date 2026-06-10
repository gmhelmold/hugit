# ADR-0001 — Intent context envelope (`context.json`)

- **Status:** Partially ratified (owner, 2026-06-10): §7.1 capture + §7.2
  retention DECIDED; §7.3 cost visibility still open. Owner-directed extension
  same date: **the envelope exists at all three altitudes** — the PR carries
  the orchestrator-session envelope (trajectory + snapshot), the campaign its
  own (§2.4) — not just computed metric rollups.
- **Date:** 2026-06-09 (extended + ratified 2026-06-10)
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
data** (whitepaper §13).

**One envelope per authored unit, at every altitude** (owner-directed
2026-06-10): the **intent** carries the subagent's envelope; the **PR** carries
the **orchestrator-session envelope** (the session transcript that planned,
dispatched and landed the bundle, plus its context snapshot); the **campaign**
carries its own. Metric **rollups** (§2.3) stay derived/computed; the
**envelopes are captured**, not derived. Same shape at every altitude — only
`altitude` + the authored-unit id change. This is what makes the stack
auditable top-down: campaign session ⊃ PR session ⊃ intent trajectory.

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
  "altitude": "intent",               // intent | pr | campaign (owner 2026-06-10)
  "intent_id": "a31",                 // the authored-unit id (pr_id / campaign at higher altitudes)
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

### 2.3 Metrics — three altitudes: intent → PR → campaign

Metrics roll up at **three altitudes**, each consolidating the one below, each
with its own author and the **same cost-decomposition vocabulary**:

| Altitude | Authored by | Unit |
|---|---|---|
| **intent** | a **subagent** (ephemeral) | a commit |
| **PR** | the **orchestrator or a human** — never a subagent | a **bundle of intents** (commits) |
| **campaign** | a **human** (the goal owner) | a **bundle of PRs** in the landing queue — **NOT** a bundle of commits |

> **Crucial:** the landing queue unions and tests a **bundle of PRs** (same
> campaign), never a bundle of commits. Nesting is strict: `commit ⊂ PR ⊂
> campaign-bundle`. The union-test / landing unit is the **PR**; the campaign is
> the set of PRs landed together.

**Per intent:** `metrics` above — tokens (with cache split), wall-clock,
active time, tool-call count + breakdown, model turns, derived cost.

**Per PR — the PR record (NOT just a sum):** Authorship matters here.
**An intent (commit) is authored by a subagent; a PR is authored by the
orchestrator or a human — NEVER by a subagent** (forge-authz rule, enforced by
D14). Consequence: the PR's true cost is **not** Σ(intents). It is the **work**
(the subagents) **plus** the **coordination** the PR author spent on top
(planning, decomposing, dispatching, cold-verifying, landing) **plus**
**verification** (the adversarial panels) **plus** **CI** — with **waste shown,
not hidden**. The forge computes this record; it is not stored per envelope.

```jsonc
{
  "pr_id": "128",
  "author": { "kind": "orchestrator", "model": "opus-4.8", "run_id": "orq-014" },
  // kind ∈ {orchestrator, human} — NEVER subagent. human → { "principal": "…" }.
  "intent_ids": ["a31","a2f","a30"],
  "intent_count": 3, "agent_count": 3, "models_used": ["opus-4.8","sonnet-4.6"],

  // — the PR's OWN captured envelope (owner 2026-06-10): the orchestrator
  //   session that planned/dispatched/landed this bundle. Same shape as the
  //   intent envelope (§2.2, altitude:"pr"); ref'd here, captured not derived.
  //   If one session authors several PRs, each PR refs the same session blob
  //   (CAS dedupes) with its own span markers.
  "envelope_ref": "cas:…",            // → ContextEnvelope{altitude:"pr"} w/ trajectory + snapshot

  // cost decomposed by WHERE it went — the SOTA part
  "cost": {
    "work":          { "tokens": 0, "tool_calls": 0, "cost_usd": 0 },             // Σ intents (subagents)
    "orchestration": { "tokens": 0, "tool_calls": 0, "turns": 0, "cost_usd": 0 }, // the PR author's own spend
    "verification":  { "tokens": 0, "verdict_panels": 0, "cost_usd": 0 },         // adversarial review calls
    "ci":            { "cache_hit": 0, "exec": 0, "cost_usd": 0, "saved_usd": 0 },// memoization economics
    "waste":         { "discarded_intents": 0, "retried_agents": 0,
                       "tokens_not_landed": 0, "cost_usd": 0 },                   // spent-but-not-landed
    "total":         { "tokens": 0, "cost_usd": 0 }       // work + orchestration + verification + ci
  },
  "time": {
    "wall_span_ms": 0,   // first activity → landed (cycle time)
    "agent_sum_ms": 0,   // Σ per-intent active (agent-time; > span when parallel)
    "queue_wait_ms": 0,  // time held in the landing queue
    "human_touches": 0,  // human decisions/comments on the PR
    "landed_at": 0
  },
  "efficiency": {
    "overhead_pct": 0,        // orchestration ÷ total — lean fleet vs bloated
    "cache_savings_pct": 0,   // CI saved ÷ would-be
    "first_pass_yield": 0,    // intents landed without rework
    "cost_per_net_kloc": 0
  }
}
```

Two principles this encodes:
- **Cost is decomposed, not lumped** — work (subagents) vs coordination (the PR
  author) vs verification vs CI. The **overhead ratio** (orchestration ÷ total)
  is how you tell a lean fleet from a bloated one; no other forge surfaces it.
- **Waste is shown, not hidden** — tokens spent on discarded/retried intents
  that never landed. Gross spend vs landed spend = honest **first-pass yield**.

**Span vs sum (inside `time`) is deliberate and a product asset:** `wall_span_ms`
= *"this PR took 14 min of clock time"* (reflects fleet parallelism);
`agent_sum_ms` = *"3 h of agent-time went into it"*. The forge shows both; no
other forge can.

**Per campaign — the third altitude.** A campaign (the bundle key: PRs of the
same campaign are tested/landed together) is **owned by a human** and is the
unit the Ledger reports on (*pedido → feito → provado, por campanha*). It
consolidates its PRs the same way a PR consolidates its intents — same
decomposition, plus campaign progress:

```jsonc
{
  "campaign": "auth-hardening",
  "charter": "endurecer a borda de autenticação",   // human-defined goal
  "owner": { "principal": "gustavo@humangr.com" },  // human — never a subagent
  "envelope_ref": "cas:…",            // campaign's own envelope (altitude:"campaign"):
                                      // the campaign-level session(s) + snapshot (owner 2026-06-10)
  "pr_ids": ["128","129"], "pr_count": 2,
  "intent_count": 5, "agent_count": 5, "models_used": ["opus-4.8","sonnet-4.6"],
  "cost": { /* work · orchestration · verification · ci · waste · total — Σ over PRs */ },
  "time": {
    "wall_span_ms": 0,   // campaign opened → last PR landed (lead time)
    "agent_sum_ms": 0,
    "queue_wait_ms": 0
  },
  "efficiency": { "overhead_pct": 0, "cache_savings_pct": 0,
                  "first_pass_yield": 0, "cost_per_net_kloc": 0 },
  "progress": { "landed": 1, "in_flight": 1, "blocked": 0 }   // campaign completion
}
```

So: **intent → PR → campaign**, three nested rollups, one vocabulary. Authorship
ascends subagent → orchestrator/human → human. Cost stays decomposed (work vs
coordination vs verification vs CI, waste shown) at every altitude — that is how
a fleet stays legible from one commit all the way up to a whole campaign.

## 3. Privacy, redaction, capture levels

Transcripts may carry secrets/PII (whitepaper §13). Therefore:

- **Always tenant-private, never cross-tenant, never training data.** Scrubbed
  with `REDACTED_MARKER` per `redaction_policy` before the blob is written.
- **Default capture level is `full` — RATIFIED, non-negotiable (owner
  2026-06-10):** *"transcript 100% tem que ser salvo sempre, inegociável — o
  fato de ser maior é ainda mais motivo pra salvar tudo."* Applies at every
  altitude (subagent intents, orchestrator PR sessions, campaign sessions).
  The level ladder below survives only as a **per-repo privacy opt-DOWN for
  customer tenants** (their data, their dial) — never as our default:

  | Level | Stores | Use |
  |---|---|---|
  | `off` | envelope metadata only | max privacy / min storage |
  | `metrics` | + `metrics` | cost/time visibility, no transcript |
  | `task` | + `task_transcript_ref`, `summary` | mid privacy dial |
  | `full` | + `raw_transcript_ref` | **the default** — forensics / replay |

  Refs absent below their level are `null`; consumers must tolerate nulls.
- **Retention: keep forever by default — RATIFIED by the same directive**
  ("salvo **sempre**"): no automatic TTL/GC on trajectory blobs. Content
  leaves storage only via the explicit erasure path (tenant request →
  tombstone: *"o conteúdo é apagável; a prova, não"*) — never via a timer.
- **Storage tier — owner-directed (2026-06-10): trajectory blobs do NOT ride
  the CoreLink hot path.** The AC/CAS fast tier exists for memoized CI
  (latency on the check path). Transcripts are write-once/read-rarely
  archive: they go to a **cheap cold object store** behind the same
  content-addressed ref scheme ("até Google Drive resolve" — the bar is
  cost, not latency). Refs in the envelope are **tier-agnostic** opaque
  content-addressed URIs; the resolver maps hash → tier. Dedupe-by-content
  still applies. The P2 tenant request is NOT sized for transcript growth.

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
  plus `IntentMetrics` and the derived (not-frozen, forge-computed) `PrRecord`
  and `CampaignRollup`. Additive to `IntentSidecar` (it already holds
  `context_ref`).
- **Producers must emit metrics:** the runner/agent harness reports tokens,
  wall/active ms, tool-call breakdown, and turns at intent close; the
  **orchestrator must emit its own** coordination metrics (the PR-author spend)
  and **waste** (discarded/retried intents). (New WP under the runner/journal
  area; sizing per memory `agent-task-sizing`.)
- **Forge-authz invariant (D14):** a **PR author ∈ {orchestrator, human}** and a
  **campaign owner is human** — **never a subagent**. Subagents author intents
  only. The authz layer must reject any other authorship.
- **githugr display obligation:** surface metrics at **all three altitudes**
  (intent · PR · campaign) and the three trajectory altitudes (summary → task →
  full, progressively disclosed). Recorded in the companion ADR; the mockups
  (`intent.html`, `pr-detail.html` Overview, plus `ledger.html`/`insights.html`
  for campaign) must show the decomposed metrics.
- **Redaction is on the write path,** not the read path — the blob is scrubbed
  before it is stored.

## 6. Alternatives considered

- **Inline the transcript in `context.json`** — rejected: envelopes become MB,
  no dedupe, git/CAS bloat. Refs win.
- **One flat transcript only** — rejected: forces reading everything to learn
  anything; the three altitudes are the whole point.
- **Skip cost/time metrics** — rejected: the forge's differentiator is making
  agent work accountable; omitting metrics guts the value.

## 7. Owner ratification record

1. **Default capture level — RATIFIED (owner, 2026-06-10): `full`, always,
   non-negotiable.** Owner verbatim: *"Sub agents by default transcript 100%
   tem que ser salvo sempre, inegociável. […] o fato de ser maior é ainda mais
   motivo pra salvar tudo."* (The §7-recommended `task` default is rejected.)
2. **Retention — RATIFIED by the same directive: forever by default, no
   TTL/GC.** "Salvo sempre" reads literally; erasure only via the explicit
   tombstone path, never a timer. (The 90-day GC recommendation is rejected.)
3. **`cost_usd` visibility (all members vs owner/admin only) — STILL OPEN.**
   Recommend visible to all (trust by transparency); reveals COGS.

**Owner-directed extension (2026-06-10), same authority as ratification:**
the envelope exists at all three altitudes — **each PR carries its
orchestrator-session envelope** (session transcript + context snapshot +
campaign), **each campaign its own**, alongside the intents' envelopes
(§2, §2.3 `envelope_ref`). Owner rationale: *"assim teríamos as camadas:
transcript da sessão, snapshot de contexto e cia da campanha, de cada PR, e
dos intents — muito mais transparente e auditável."*
