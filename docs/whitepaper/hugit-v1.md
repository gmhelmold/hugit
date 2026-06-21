# hugit — Intent-Based Version Control and the LLM-Native Forge

**Whitepaper v1 · formal product design**
Gustavo Schneiter · HumanGuardrail · 2026-06-05 · status: v1 DRAFT

Implementation status: see [README](../../README.md) for the live-vs-hermetic-vs-absent scope.

---

## Abstract

Software in 2026 is increasingly written by fleets of LLM agents under human
command — 26.9% of production code is already AI-authored, and the share of
multi-file agentic sessions doubled in a year. The version-control and
collaboration stack underneath this shift — git and GitHub — was designed for
a different species of worker: humans, at human pace, with human memory.
The measurable result is an **integration crisis**: 27.67% of agentic PRs hit
merge conflicts (avg 540 conflicting lines), agent PRs wait 4.6× longer than
human PRs and land at 32.7% acceptance vs 84.4%, and every agent session ends
in a *reconciliation round* — re-paying for understanding that the tooling
threw away.

hugit is a version-control system and forge built on one inversion: **version
the intent, the context, and the proof — and treat the code diff as their
verified projection.** Git's object model is preserved whole as the projection
format (full wire-protocol compatibility; zero relearning for humans or
models); above it, hugit introduces a small set of new content-addressed
objects — Intent, Context Snapshot, Trajectory, Claim, Verdict, Check Result —
that make agent-fleet development coordinated, reviewable, reproducible, and
safe by construction.

hugit is not built on bare infrastructure. It is the fourth layer of
**CoreLink**, a multi-tenant content-addressed storage and computation
platform already in production: a global CAS (R2-backed, multi-region,
tenant-isolated), a memoizing Action Cache, a workspace snapshot/hydrate
client (`clw`, shipped), and an ephemeral runner fabric (in flight). This
substrate gives hugit an economic property no incumbent can match without
destroying its own revenue model: **the costs that dominate a forge — CI
compute, storage, egress, environment setup — are memoized, deduplicated, or
zero-rated by construction.** GitHub bills the waste; CoreLink deletes it.

---

## 1. The problem, measured

1. **The landing problem.** Orchestrated agents produce parallel work cheaply;
   reconciling it is the new bottleneck. "Change A can be green on its own.
   Change B can be green on its own. A plus B can still be red." Existing
   orchestrators isolate *execution* and stop at the merge boundary.
2. **The review collapse.** AI-heavy teams ship 98% more PRs, 154% larger,
   with 91% longer review cycles. Human LGTM is the constraint of the entire
   machine; LLM reviewers fed raw diffs reach only 24–46% precision.
3. **The reconciliation round.** Git versions text and discards the
   understanding that produced it. In human teams the understanding persisted
   in heads; in agent fleets it evaporates with the context window — so every
   merge, every review, every resumed session pays to rebuild it.
4. **The economics of waste.** CI re-verifies the already-verified on every
   push (queues of hundreds of runs; p50 queue age measured in hours);
   worktrees and node_modules duplicate gigabytes per agent; egress is billed
   per byte. The incumbent's pricing (per-minute compute, usage-billed AI,
   per-byte egress on competitors) monetizes exactly this waste — which is why
   it cannot remove it.
5. **A platform not built for machine actors.** Human-paced rate limits,
   10-second fire-and-forget webhooks, prose-thread reviews, 257 incidents in
   12 months. GitHub's own primitives are overwhelmed by its own agent push.

(All figures sourced in a 4-lane evidence sweep, 2026-06-05: git core pains, GitHub platform pains, multi-agent workflow pains, competitive landscape.)

---

## 2. The Inversion (thesis)

> Git made **code** the source of truth and threw the intent away — the right
> trade in 2005. hugit makes **intent + context + proof** the versioned truth
> and treats code as their verified projection. Git was the revolution of
> human-driven development; hugit is the revolution of LLM-driven development.

Five concrete inversions follow (formalized in §4–§6):

| # | From (git/GitHub) | To (hugit) |
|---|---|---|
| 1 | commit/PR as the unit of meaning | **Intent** — charter + context + trajectory + diff + evidence + verdicts |
| 2 | context destroyed at session end | **context versioned** — snapshot, resume, replay, audit, diff-minds |
| 3 | merge as textual patch | **merge as re-execution** — regenerative rebase; claims collide at dispatch |
| 4 | review as reading lines | **review as interrogation** — claim verification (machine) + ledger & conversational review (human) |
| 5 | ceremony performed by workers | **ceremony emitted by the system** — commit messages, branches, PRs, CI config: generated |

And one invariant above all: **git is never broken.** Every hugit repository
is at all times a valid git repository served over the standard wire protocol.
If every intelligent layer fails, what remains is fast, healthy git.

---

## 3. The operating model

```
HUMAN STAKEHOLDER      decides · approves · interrogates  (never performs ceremony)
   └─ ORCHESTRATOR     (e.g. Claude Opus — the tech-lead seat)
        owns CAMPAIGNS: plans waves, declares claims, dispatches, lands
        └─ AGENT SQUADS one intent each, fenced by claims
             campaign "checkout" · campaign "perf" · campaign "security"
                          (one repository, in parallel)
```

- **Campaign** — a long-running stream of work; the unit of *team* parallelism.
- **Intent** — one unit of work; the unit of *work* parallelism.
- Cross-campaign concurrency is arbitrated by **claims + the landing queue**,
  not by humans deconflicting in chat.
- Every principal — human, orchestrator, worker agent, model — is a
  first-class identity with permissions, budgets and full attribution.

---

## 4. The object model (formal)

All objects are content-addressed, immutable, and stored in the CoreLink CAS.
Mutable state (refs, queues, policies) is a thin event-sourced namespace over
them.

| Object | Schema (essentials) | Replaces |
|---|---|---|
| **Intent** | `{id, campaign, charter, constraints, acceptance[], claims, parent_intents[], ctx_start, trajectory, diff_ref, evidence[], verdicts[], state}` | commit-as-meaning, PR, issue |
| **Context Snapshot** | `{model, charter_ref, files_read[(path,hash)], tool_calls[], conversation_ref (redacted per policy), journal_ref, env_manifest}` | nothing — today destroyed |
| **Trajectory** | append-only event log of one intent's execution; replayable, time-travelable | scrollback, tribal memory |
| **Claim** | `{paths[], build_targets[], contracts[]}` — declared at plan time, enforced physically | CODEOWNERS, "hope" |
| **Workspace** | source + deps + toolchain + build state as one manifest; materialized **by claims**; local/remote transparent | clones, worktrees, dev envs, CI checkout |
| **Verdict** | `{intent, tree_hash, lens, verdict ∈ {APPROVE, FIX-FIRST, REJECT}, claims_checked[], evidence[]}` | PR review threads |
| **Check Result** | memo of `check(tree_hash, def_digest, toolchain_digest)` | CI runs |
| **Blob/Tree/Commit** | git's objects, byte-exact — **the projection** | (kept whole) |

**Projection rule:** every landed intent deterministically emits git commits
(generated messages embedding the intent id). `git log` is the machine
altitude; `hugit log` is the intent altitude. Same store, two zooms; they can
never disagree because one is derived from the other.

### 4.1 Intent lifecycle (state machine)

```
PROPOSED ─plan→ PLANNED ─dispatch→ EXECUTING ─seal→ SEALED ─verify→ VERIFIED
   │               │ (claims validated,            (evidence       (verdicts per
   │               │  DAG position set)             assembled)      policy lenses)
   │               └─ BLOCKED (claim conflict → re-slice or serialize)
   └─ (an "issue" is simply an Intent parked here)
VERIFIED ─queue→ LANDABLE ─union-test→ LANDED
                    └─ UNION-FAIL (structured failure → owner intent re-enters EXECUTING)
any state ─→ REJECTED | ABANDONED   (all transitions are events; all reversible)
```

The ceremony of Inversion 5 is this machine running itself: `seal` is the only
verb a worker performs; everything else is the system.

---

## 5. Architecture — the CoreLink substrate

hugit is **layer 4 of an existing platform**, not a greenfield stack. Status
per layer is stated honestly:

| Layer | Component | Status (2026-06-05) |
|---|---|---|
| **L0** | **CoreLink CAS** — R2-backed, multi-region, chunked (SplitBlob/SpliceBlob), Merkle manifests, HMAC-derived tenant prefixes, fail-CLOSED audit | **in production** |
| **L1** | **CoreLink AC** — memoized computation results; surfaces already live: Bazel REAPI v2, Turborepo, sccache | **in production** |
| **L2** | **CoreLink Workspaces (`clw`)** — `snapshot / hydrate / status / run (AC-memoized) / ls` against the live API | **built, phase 1 shipped** |
| **L3** | **CoreLink Runners** — ephemeral Firecracker-class compute on commodity metal, cache-warm boot | in flight |
| **L4** | **hugit core** — intent store, claims planner, landing engine, policy engine, event-log refs (one Durable Object per repo), semantic index | to build (this paper) |
| **L5** | **Surfaces** — git wire protocol (projection), hugit CLI/API, guaranteed event stream, Ledger & Mission Control, GitHub mirror | to build |

### 5.1 How each Inversion rides the substrate

| Inversion | CoreLink primitive it rides |
|---|---|
| Intents & context versioned | CAS objects — storage is deduped and effectively free at the relevant sizes |
| Workspaces born <1s, fenced | `clw snapshot/hydrate` + claim-filtered manifests (sparse materialization IS the fence) |
| Merge as re-execution | re-execution is cheap **because** builds/tests hit the AC; regenerating beats patching only when verification is near-free — CoreLink makes it near-free |
| Checks that never repeat | the AC, generalized: `clw run` already memoizes commands; checks-as-code is its formalization |
| Union testing at fleet scale | affected-target computation + memoized checks: testing A+B+C costs only the *delta* novelty |
| The semantic index | AC entries keyed by tree-hash: parse/summarize/index once per subtree, ever, across all tenants for public code |
| Diagnoses & auto-culprit | bisect over memoized checks ≈ free; culprit-finding becomes a default, not a luxury |

### 5.2 The economic physics (why this beats GitHub structurally)

| Cost driver of a forge | GitHub | hugit/CoreLink |
|---|---|---|
| CI compute | re-runs everything; **billed per minute — waste IS revenue** | memoized by content; affected-targets only; waste deleted |
| Environment setup | per-job reinstall/recompile | snapshot hydration from warm CAS (seconds) |
| Storage | per-repo copies; LFS double-billed | global content-addressed dedup; one physical copy of `tokio 1.x` for all tenants |
| Egress | competitors pay per byte (S3); GitHub eats it inside per-minute pricing | **R2 zero egress** — hydration, clones, artifact pulls cost $0 to serve |
| AI review/agents | usage-billed (10–50× shocks) | flat; BYO-orchestrator — we sell the ground, not the tokens |

> **The structural checkmate:** GitHub's revenue model **bills the waste**
> (per-minute CI, usage-based AI, per-seat ceremony). CoreLink's margin model
> **deletes the waste** (memoize, dedupe, zero-rate). For GitHub to match
> hugit's economics it must destroy its own P&L; for hugit, the customer's
> delight (speed) and the company's margin (less compute) are the same number.
> This is not a feature war — it is an incentive war, and the incumbent's
> incentives are on our side.

And the network effect compounds it — **once the staged cross-tenant lever
ships** (dedup is intra-tenant at CoreLink's GA; cross-tenant sharing of
public-deterministic artifacts is designed in, `CAP-DEDUP-CROSS-TENANT`,
post-GA): every tenant's public-deterministic artifacts and check results warm
the cache for all tenants, and customer #500 arrives to a workspace that is
already ~40–70% hot. **The product gets better as it grows; a competitor
without scale cannot match either the cost or the hit-rate.** (Boundary,
non-negotiable: public-deterministic shares; private bytes never cross
tenants.)

---

## 6. Core algorithms (formal sketches)

### 6.1 Claims & conflict-at-dispatch
```
claims(I) = closure over build graph of {paths ∪ targets ∪ contracts}
independent(I, J) ⇔ claims(I) ∩ claims(J) = ∅
plan(P): for all pairs in wave → intersect? planner must re-slice or add DAG edge
runtime: a workspace materializes ONLY claims(I); writes outside are
         physically impossible (the file isn't there) + policy event
```
True conflicts surface **before work begins**. Write-time textual overlap
within shared claims emits an event to both orchestrators within seconds.

### 6.2 Checks (pure, memoized)
```
key = H(tree_root ‖ check_def_digest ‖ toolchain_digest)
check(key) → AC hit? return proof : execute on runner, store, return
affected(Δtree) = build-graph reachable check set
shadow: on every workspace snapshot, speculatively evaluate affected(Δ)
```
Local `hugit check` and forge execution are the same function — byte-identical
by construction (this is `clw run`, formalized).

### 6.3 Regenerative rebase
```
rebase(I, base B0→B1):
  if claims(I) ∩ Δ(B0,B1) = ∅:    textual fast-path (guaranteed clean)
  else:
    re-execute I: agent(ctx_start(I), charter(I)) on workspace(B1) → diff D′
    require acceptance(I) green on B1+D′
    if distance(D, D′) > policy.threshold → re-verdict required
    land as new revision of the SAME intent (both revisions kept, addressable)
```
Patching text is the fallback; re-executing the (cheaper) source — the intent
— is the default for orthogonal work. Trust ships **opt-in → default** as
acceptance-confidence data accumulates.

### 6.4 Landing (union testing at fleet scale)
```
queue = landable intents, DAG-ordered
batch B: U = fold(regen-rebase, head, B)
run affected checks on U          # mostly AC hits — only novelty executes
green → land batch atomically (one event-log ref move; main always green)
red   → bisect batch over memoized checks (≈free) → minimal failing pair
        → structured UNION-FAIL to owner intent; rest of batch proceeds
disjoint-claims intents land in parallel lanes
```
This is Uber SubmitQueue's blueprint with two upgrades it lacked: claims
declared ahead (directed search, not discovery) and memoized verification
(union tests cost only their novelty).

### 6.5 The event log & universal undo
One Durable Object per repository owns an append-only, hash-chained event log.
Refs are a derived view. Every operation — landing, policy change, ref move —
is an event; `hugit undo` appends a compensating event. **Nothing is ever
rewritten; history is re-projected.** Force-push data loss is not "protected
against" — it is unexpressible.

---

## 7. The product surface (summary — full catalog in appendix)

| Principal | Verbs (complete set) |
|---|---|
| **Human** | `ledger` · `review` (interrogation) · `approve/reject` · `watch` · `why` · `undo` · `policy` |
| **Orchestrator** | `campaign` · `plan apply` · `dispatch` · `fleet` · `land` · `verdict request` · `tournament` |
| **Worker agent** | `ctx snap/resume/diff/audit` · `map` · `impact` · `status` · `diag` · `check` · `intent seal` · `journal` |
| **Everyone** | `ask` · `log` · `ws spawn/attach/snap/gc` — and **every git command, unchanged, forever** |

Three namespace laws: git verbs never shadowed; refs auto-managed under
`refs/hugit/campaigns/…`; the degradation invariant (worst case = healthy git).

---

## 8. The human experience (the stakeholder never loses the thread)

1. **The file tree is just a file tree** — locally, on the web UI, on the
   GitHub mirror. Nothing new stands between a human and a file.
2. **History defaults to the Ledger** (asked → done → proven, by campaign),
   toggleable to raw commits. Map ↔ satellite; same data.
3. **Live work is a dashboard, not a branch list**: campaigns → intents →
   agents, with progress, risk, cost. `hugit watch` (TUI) and Mission Control
   (web) read the same event stream.
4. **The attention queue is the inbox**: policy × blast-radius ×
   verdict-confidence ranks what needs a human; everything else is narration
   you *may* read, never homework you *must*.
5. **Review is interrogation**: ask the change anything — "where does this
   touch the money path?", "what changes for a logged-out user?", "convince me
   this is safe" — answered from intent + context + evidence, with deep links
   down to the byte. Four clicks from "today" to "the line", meaning preserved
   at every altitude.
6. **The permanent escape hatch**: the GitHub mirror renders everything in the
   most familiar UI on earth. Refusing to learn anything new costs you live
   narration — never visibility.

---

## 9. Security & trust model (five locks + the boundary)

1. **Claims as physical fences** — capability-scoped workspaces; the blast
   radius IS the claim; `rm -rf` has nowhere to go.
2. **Secrets never enter workspaces** — a broker executes privileged
   operations on the workspace's behalf (CoreLink's write-only secret model,
   generalized). Agents never see credentials.
3. **Event-sourced everything → universal undo** — every principal's every
   action attributable and reversible; nothing is ever lost.
4. **Policy gates fail closed** — locally testable, identically enforced.
5. **The degradation invariant** — intelligence layers down ⇒ a valid git
   repository keeps serving.

**Tenant boundary (inherited from CoreLink, non-negotiable):** HMAC-derived
prefixes, fail-closed audit; public-deterministic artifacts share, private
bytes never cross. Context snapshots are tenant-private, policy-redactable,
retention-bound, and are **never** training data.

**Provenance:** every artifact traces to `{tree_hash, check_def, runner,
model, prompt, principal chain}` — SLSA-class attestation falls out of the
object model, including the layer GitHub cannot express: *which model wrote
this under whose instruction at what cost.*

---

## 10. Compatibility & absorption

The doctrine in three lines:

- **Absorb and exceed (🟢):** CI→memoized checks, Codespaces→workspaces,
  LFS→native blobs, Dependabot→silent pre-tested landings, code search→
  semantic index, merge queue→union landing, packages→CAS registry,
  attestation→model-level provenance.
- **Transform (🔁):** PR/issue→Intent, review→verdicts+interrogation,
  boards→campaigns, notifications→attention queue, wiki→knowledge layer.
- **Ride the mirror (🪞) / not our war (⛔):** the social graph (stars,
  discussions, sponsors) — drained, not stormed. Every absorption ships with
  its compat shim (Actions-YAML runner, status/badge API); the ecosystem must
  never notice a seam. **Nothing is absorbed worse** — if our version isn't
  strictly better or honestly transformed, it waits.

---

## 11. Economics & pricing doctrine

- **COGS physics:** zero egress (R2), global dedup, memoized verification,
  ephemeral cache-warm compute. CoreLink's audited template: ~80% gross margin
  at $30/mo SMB self-serve; pinning margins 85–90% effective via dedup.
- **The consolidation prize:** agent-heavy developers today pay $200–600/mo
  across GitHub + Copilot agents + runners + stacked-PR + AI review +
  sandboxes. hugit is one flat, predictable bill.
- **Doctrine (four laws):** flat pricing per unit-that-scales (orchestrator
  seats, parallel runners, warm workspaces, pinned storage); **never** meter
  the customer's own compute; **never** usage-billing whiplash; expansion
  comes from fleet growth, not metering surprises.
- **TAM:** AI code tools $9.5B (2026) → $22B (2030); the deeper claim: hugit's
  category is not "code tools" but **the system of record for machine-built
  software** — the layer everything else plugs into.

---

## 12. The route (rethought with CoreLink in hand)

The original campaign sequence assumed runners before workspaces. Reality
changed it: **`clw` is built** — so the integrated developer experience starts
immediately, dogfooded on our own fleet, while runners mature in parallel.

| Phase | Ships | Substrate it stands on |
|---|---|---|
| **A — now** | CoreLink launches (cache + governance; untouched route). `clw` dogfood on our own repos | L0–L2 live |
| **B — hugit Dev Kit (first SKU)** | claim-fenced workspaces (`clw`+claims) + checks-as-code memoized in the AC + **the landing layer riding GitHub** (union testing, regen rebase, structured verdicts via API/mirror). Zero migration ask; our own fleet is design partner #0, then 10 external design partners | L0–L2 + GitHub's hosting |
| **C — the fabric** | CoreLink Runners take over check execution + agent sandboxes; flake intelligence + auto-culprit ship (they need the fabric's volume) | L3 |
| **D — the forge** | git wire protocol over the CAS; intents native; Ledger + Mission Control; jj first-class (change-ids); one-command GitHub import; one-way mirror | L4–L5 |
| **E — head-on** | bidirectional mirror (forge-authoritative) → authoritative hosting; parity push per the absorption map (Actions shim, packages, environments); the long-game social absorption by pull | full stack |

Standing constraints: CoreLink's launch route is untouchable; every phase
funds and de-risks the next; phase E begins only after the bridge has run on
our own repositories for months.

---

## 13. Risks & honest limits

1. **Regenerative rebase trust** — re-executed code can differ from reviewed
   code. Mitigation: acceptance must re-pass; distance-threshold re-verdicts;
   opt-in → default; every regen is itself auditable. Residual risk accepted
   and priced.
2. **Context capture is sensitive** — prompts may carry secrets/PII.
   Tenant-private always, redaction policy, retention bounds, never
   cross-tenant, never training data. Capture level is repo-configurable.
3. **Single-vendor Cloudflare on a source-of-truth path** — continuous
   full-fidelity export, the GitHub mirror as a live replica, independent
   escrow for paid tiers. The exit guarantee is also the DR plan.
4. **The window** — Cursor+Graphite (>$290M of intent in this exact layer),
   GitHub Agent HQ. Answer: phase B ships fast *because* it rides GitHub and
   L0–L2 exist; speed beats completeness.
5. **The social moat is real and is not ours to win** — we win the workflow;
   stars stay where they are until the community moves them.
6. **Two mental models** (git view ↔ intent view) — role-correct defaults;
   both views derive from one store and cannot disagree.
7. **Operating untrusted compute** is a heavier ops discipline than storage —
   inherited deliberately by CoreLink Runners, reused (never reinvented) by hugit.

---

## 14. The one-paragraph summary

git versions text written by humans and discards the understanding that
produced it; that discard is now the dominant cost of software built by agent
fleets. hugit versions the intent, the context, and the proof — projecting
flawless git underneath — and runs on a content-addressed substrate
(CoreLink: CAS + AC + workspaces + runners) that makes verification memoized,
environments instant, storage deduplicated and egress free. The result is a
forge where conflicts surface before work begins, merges re-execute instead
of re-fighting, main is always green at fleet scale, humans command through a
ledger and an attention queue instead of drowning in diffs — and the
incumbent cannot follow without billing itself out of its own revenue.
**Git was the revolution of human-driven development. hugit is the revolution
of LLM-driven development — with humans in command.**

---

## Appendices

- A. [`docs/adr/0001-intent-context-envelope.md`](../adr/0001-intent-context-envelope.md) — formal schema for the Intent Context Envelope
- B. [`docs/adr/0002-hugr-identity.md`](../adr/0002-hugr-identity.md) — identity model: one HuGR account on CoreLink machinery
