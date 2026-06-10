# Handoff → githugr lead: PRs and campaigns now carry their own envelope (ADR-0001 extension)

> 2026-06-10, from the hugit session. Owner ratified ADR-0001 §7 today AND
> extended the design. This changes what a **PR** and a **campaign** ARE in the
> product's data model — githugr displays both, so this is yours to absorb.
> Canonical source: `docs/adr/0001-intent-context-envelope.md` (this repo,
> @ `2089b3b`); reference mock: `docs/handoff/2026-06-10-pr-detail-envelope-mock.html`
> (@ `d69ea13`, render-verified, owner has seen it).

## 1. What changed, in one paragraph

Until today the **intent** was the only unit that carried a captured context
envelope (trajectory + snapshot + metrics); the PR and the campaign only got
**derived metric rollups** (cost decomposition, time, efficiency — computed,
not captured). The owner closed that gap: **the envelope now exists at all
three altitudes.** Each **PR carries the orchestrator-session envelope** — the
session transcript that planned, dispatched, cold-verified and bundled the
intents, plus that session's context snapshot and campaign. Each **campaign
carries its own** (the campaign-level session(s)). Owner rationale, verbatim:
*"assim teríamos as camadas: transcript da sessão, snapshot de contexto e cia
da campanha, de cada PR, e dos intents — muito mais transparente e auditável."*

## 2. The ratified knobs (these are LAW, not recommendations)

| Knob | Decision | Owner verbatim |
|---|---|---|
| Default capture level | **`full`, always, at every altitude — non-negotiable** | "transcript 100% tem que ser salvo sempre, inegociável" |
| Retention | **forever — no TTL, no GC.** Content leaves storage only via the explicit erasure path (tombstone: "o conteúdo é apagável; a prova, não") | "salvo **sempre**" |
| `cost_usd` visibility | **STILL OPEN** (all repo members vs owner/admin only) — gates display copy only, not data | — |

The capture-level ladder (`off|metrics|task|full`) survives **only** as a
per-repo privacy opt-DOWN for customer tenants. Never our default.

## 3. The data shapes (what your screens will read)

One envelope shape, three altitudes — only the discriminator and unit id
change (`ContextEnvelope`, being frozen in hugit-contracts as WP-F1 right now):

```jsonc
{ "altitude": "pr",            // intent | pr | campaign
  "pr_id": "128", "campaign": "auth-hardening",
  "authorship": { "kind": "orchestrator", "model": "…", "run_id": "orq-014" },
  "trajectory": { "raw_transcript_ref": "cas:…",   // 100%, kept forever
                  "task_transcript_ref": "cas:…", "summary": "…" },
  "snapshot":   { "files_read": [...], "prompt_ref": "cas:… (redacted)" } }
```

- `PrRecord` (derived, unchanged otherwise) gains **`envelope_ref`** → the PR's
  captured envelope. Same for `CampaignRollup`.
- Nesting is strict and unchanged: `intent (commit) ⊂ PR ⊂ campaign`.
  Authorship ascends subagent → orchestrator/human → human (authz: a PR author
  is NEVER a subagent).
- One orchestrator session may author several PRs: each PR refs the same
  session blob (CAS dedupes); span markers disambiguate. Don't render that as
  N different sessions.

## 4. Display obligations (your side — DDD: mock first, owner approves)

| Surface | What it must now show |
|---|---|
| **`pr-detail` Overview** | the "Envelope do PR" section — session summary/task/raw at 3 altitudes, snapshot chips, campaign line, the `altitude:"pr"` context.json block, "transcript 100% · salvo sempre" badge, the `campanha ⊃ PR ⊃ intent` nesting strip. **Reference mock exists** (see header) — owner has eyeballed it; absorb/refine it in the corpus, don't start from zero. |
| **`pr-detail` rail · Proveniência** | `envelope do PR cas:… ✓` + `transcripts 100% · pra sempre` rows (in the mock). |
| **`landing` drawer** | the PR drawer should hint the envelope exists (one line/affordance, subliminal — don't pollute Landing). |
| **`insights#ledger` / campaign views** | campaign rollup now also links the campaign's own envelope. |
| **`replay`** (killer #4) | replay now legitimately applies to **orchestrator sessions**, not only intents — the PR envelope is the entry point. |
| **⌘K** | "Abrir envelope da sessão (PR)" item (in the mock). |

Pending knob §7.3 only affects whether `cost_usd` renders for all members or
admins — keep cost display behind one decision point in your VMs.

## 5. What hugit delivers (so you don't build on sand)

- **WP-F1** (in flight now): `ContextEnvelope` + `IntentMetrics` frozen in
  `hugit-contracts` (altitude discriminator, golden round-trips), `PrRecord` /
  `CampaignRollup` derived shapes with `envelope_ref`.
- **WP-F2**: producers emit — subagent envelopes at intent close; the
  orchestrator emits its own per PR (transcript + snapshot + coordination
  metrics + waste); campaign-level same.
- **WP-F3**: the three nested rollups computed (cost decomposed work /
  orchestration / verification / ci / waste; span-vs-sum time; efficiency).
- Plan + scopes: `docs/plan/2026-06-09-adr-0001-context-envelope-rework.md`
  (unblocked + rescoped today).

Until F2/F3 land, anything you wire reads fixture shapes — same parity
discipline as the hugit-web spine (wave-1 recipe).
