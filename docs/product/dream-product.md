# hugit — the dream product

> **Working backwards, step 1: the finished product.** This document describes
> hugit as if the founder snapped his fingers and it exists, complete, in his
> hand. No roadmap, no phasing, no engineering constraints — those come later
> (decompose → white paper → refine → specs).
>
> **The bar (owner mandate, 2026-06-05):** *"Tudo que não for SOTA, não for
> fucking awesome, não for um wow factor, não entra."*
>
> **v0.3 (owner correction, 2026-06-05):** v0.2 modernized the infrastructure
> *around* git and left git's primitives intact at the center. The owner's pain
> is the primitives themselves — commit, branch, push, PR, merge — all designed
> for human pace, human memory, human trust. v0.3 puts **the Inversion** at the
> core: hugit doesn't wrap git for agents; it replaces git's unit of meaning.
>
> **Owner:** Gustavo Schneiter · **Draft v0.3:** 2026-06-05 · status: REFINING

---

## 0. The thesis

> **Git made code the source of truth and threw the intent away** — a perfect
> trade in 2005, when understanding lived in the developer's head and the
> developer was still there. In LLM-driven development the understanding lives
> in a context window that **evaporates** — so every merge, every review, every
> resumed session pays to reconstruct it. That reconstruction — the endless
> reconciliation round — is the dominant cost of agent-driven software.
>
> **hugit inverts it: version the intent, the context, and the proof — and
> treat the code diff as their verified projection.**
>
> Git was the revolution of human-driven development.
> **hugit is the revolution of LLM-driven development.**

Git remains underneath as the projection format — compatibility is sacred,
nothing to relearn, every tool and every LLM's training corpus keeps working.
But git becomes the `.o` file, not the source.

---

## 1. The five inversions (the core of the product)

### Inversion 1 — The unit of versioning: from commit to **Intent**

Git versions text snapshots; the *why* dies in a prose message. In LLM-driven
development the expensive artifact is the intent + its verification; the diff
is cheap, regenerable output.

The **Intent** is hugit's atom: `{charter, constraints, acceptance criteria,
starting context snapshot, trajectory, resulting diff, evidence, verdicts}` —
content-addressed, immutable, linked into a DAG. Commits still exist (git
compatibility) but become machine-generated implementation detail: nobody
*writes* them, nobody *reads* them as the unit of meaning. **History reads as
a DAG of intents** — 14 things that were meant and proven, not 400 robot
micro-commits.

### Inversion 2 — Context becomes versioned ("git for context windows")

The mental state that produced a change — what the agent read, knew, tried,
discarded; the conversation; the journal — is snapshotted, content-addressed,
and bound to its Intent. The owner's words: *"snapshots de contexto começam a
ser gitados."* Four superpowers fall out:

- **Resume** — a replacement agent inherits the *comprehension*, not the bytes.
  No agent ever starts cold; the discovery is paid once.
- **Reproduce** — re-run the intent from the same context (model, prompt,
  inputs pinned by hash). The change is a replayable computation.
- **Audit** — "what information produced this line?" is a query. Provenance
  for compliance, debugging, and trust.
- **Diff minds** — "what did agent A know that agent B didn't?" Context diffs
  explain divergent outputs the way code diffs never can.

**This kills the reconciliation round at the root**: reconciliation exists
because today this object is destroyed at the end of every session.

### Inversion 3 — Merge: from textual patch to **re-execution**

Nobody merges `.o` files — you recompile from source. When the source is the
intent, a textual conflict between *orthogonal* intents is resolved by
**re-applying the intent on the new base** and re-validating with its own
acceptance tests: the **regenerative rebase**. Patching text is the fallback,
not the default.

What remains as a *real* conflict is an **intersection of claims** (files,
build targets, contracts) — and claims are declared at dispatch, so true
conflicts surface **before the work starts**, not three hours later in a
540-line surprise. Conflicting text became a cheap event; conflicting
*intentions* is the only conflict left, and it appears first.

(Generated files — lockfiles, codegen, snapshots — are the degenerate case:
declared derived, always regenerated, never merged. The #1 measured git pain
ceases to exist.)

### Inversion 4 — Review: from reading lines to **interrogating the change**

Reviewing-by-reading-diffs scales with neither reviewer:

- **For the LLM reviewer — surgical precision.** Review = **claim
  verification**. The change *asserts* "implements X without breaking Y"; the
  reviewer receives the surgical packet — semantic-altitude diff, build-graph
  blast radius, touched contracts and invariants, the acceptance evidence —
  and verifies each claim, querying the index instead of guessing who calls
  what. Output: a structured verdict (APPROVE / FIX-FIRST / REJECT + evidence
  per claim), not prose comments. Today's LLM reviewers hit 24–46% precision
  because they review raw diffs, blind; precision comes from the forge serving
  exactly the needed truth.
- **For the human — track, control, follow; review only when chosen.** The
  human's default surface is the **intent ledger**: a live narrative of what
  was asked → what was done → what was proven, with a risk-ranked attention
  queue. Four altitudes of drill-down: intent → narrative → semantic diff →
  raw lines. And when the human *chooses* to truly review, the PR is a
  **conversation**: *"where does this touch the money path?"* — *"show me
  what changes for the end user"* — *"convince me this is safe."* The change
  answers for itself, because intent + context + evidence travel with it.
  LLM-assisted review isn't "the LLM summarizes the diff" — it is **the
  change being interrogable**.

### Inversion 5 — The ceremony collapses: commit/push/PR become effects, not acts

The agent doesn't "open a PR," invent a branch name, write a description, or
perform a push ritual. Work streams continuously: every save is a snapshot
(CAS makes this free), evidence accumulates, and when the acceptance criteria
go green the intent **becomes landable by itself** and enters the landing
queue. Branch names, PR descriptions, commit messages, CI YAML — all
generated, none performed. The ceremony is output of the system, not labor of
the worker.

And worktree separation was never really about directories: **isolation is by
claims.** A workspace materializes exactly what its intent claims — nothing
else exists inside it. (Safety falls out: the blast radius *is* the claim.)

---

## 2. The object model

Everything content-addressed, immutable, in the CAS. The **Intent** is the
atom; git objects are its projection.

| Object | What it is | What it replaces |
|---|---|---|
| **Intent** | the atom: charter, constraints, acceptance criteria, links to everything below; DAG-linked | the commit-as-meaning, the PR, the issue |
| **Context snapshot** | the versioned mind-state: what was read/known/tried, conversation, journal, model+prompt pins | nothing — today it's destroyed (the root of reconciliation) |
| **Trajectory** | the step log of executing an intent (event-sourced, replayable, time-travelable) | scrollback, tribal memory |
| **Workspace** | source + deps + toolchain + build state as one snapshot, materialized by claims; machines are cursors | clones, worktrees, dev envs, CI checkout |
| **Verdict** | structured judgment bound to claims at a tree-hash (APPROVE / FIX-FIRST / REJECT + per-claim evidence) | PR review threads |
| **Check result** | memoized `check(tree-hash, check-def)` — shared, replayable, attestable | CI runs |
| **Blob/Tree/Commit** | git's data model, unchanged, served over the git wire protocol — the *projection* | (kept whole — compatibility is sacred) |

---

## 3. The snap of fingers — a day commanding a fleet

09:00 — *"Ship the checkout flow."* The orchestrator declares 14 intents with
claims and acceptance criteria. hugit checks claim intersections **now**: two
intents would collide on the payments contract — re-sliced before any work
begins. Each agent receives its dispatch packet (minimal context, assembled by
the forge) and a workspace materializing exactly its claims. Nobody is born
blind; nobody can touch what it didn't claim.

10:55 — a check goes red. The agent receives a **diagnosis** (culprit intent
auto-bisected, diff vs last green, two suspect lines, a similar failure fixed
three weeks ago), not 4,000 lines of log. The month's flaky test? Quarantined
by statistics. Nobody chased it.

11:05 — six intents are green. The landing queue tests their **union** —
A green + B green ≠ A+B green — finds WP-4+WP-9 break an invariant *together*,
tells WP-9's agent exactly why. The other five land in dependency order. Two
of them touched the same file; their intents were orthogonal, so the second
was **regeneratively rebased** — re-applied on the new base, re-validated by
its own acceptance tests. No human saw a conflict marker. Checks re-executed
across all five landings: **zero** (memoized). Nobody wrote a commit message,
named a branch, or opened a PR — the ceremony emitted itself.

14:00 — your phone buzzes **once**. The intent ledger has been narrating all
day (what was asked → done → proven); policy flags one intent as
human-mandatory (it claims an auth contract). You don't read a diff — you
**interrogate the change**: *"what does this change for a logged-out user?"* —
*"show me where it touches token validation"* — *"convince me this is safe."*
It answers with its intent, its context, its evidence. Approve. 90 seconds.

16:00 — agent 12's session dies mid-intent. The replacement attaches to the
workspace and **inherits the context snapshot + journal**: the hypotheses, the
dead ends, the "it's THIS file." It resumes the understanding, not the bytes.
Re-discovery cost: zero.

18:00 — live. History reads as 14 intents, each carrying its why, replayable
from its context, addressable forever. Six months later an agent asks *"why
does checkout retry 3 times?"* and the intent answers — the reasoning, not
`git blame`'s shrug. Your Mac idled at load 1.2 all day. The bill is flat.

---

## 4. The supporting layers (each one still a wow, all serving the inversions)

### 4.1 The repo that explains itself *(serves Inversions 2 & 4)*
Living semantic index, memoized per tree-hash: symbols, call graph, contracts,
ownership, decision links — incrementally maintained by the forge, never
stale. **Altitude-controlled reading** (API surface → contracts → signatures →
source), cached per hash, served in tokens-not-files. **Impact queries as
API**: "who calls this / what breaks / blast radius of this intent" in
milliseconds. **Forge-generated dispatch packets**: dispatching an agent =
handing it one URL.

### 4.2 Ambient truth *(serves Inversions 3 & 5)*
Shadow checks on every write (affected targets, speculative, memoized = nearly
free): a continuous green/red signal follows the work like a language server
covering the whole truth of the repo. **Failures arrive as diagnoses** —
culprit auto-bisected (memoization makes bisect ~free), diff-vs-green, suspect
lines, similar past failures. **Fleet-wide flake intelligence**: statistical
detection, quarantine by policy.

### 4.3 Checks that never repeat *(the economics of Inversion 5)*
`check(tree-hash, check-def)` memoized in the AC, shared across branches,
stacks and (public-deterministic) tenants. Affected-target execution via the
build graph. Locally replayable and byte-identical (no YAML push-and-pray).
**Dependency updates as a forge feature**: speculative, pre-tested, landed
silently by policy — dependabot's job without dependabot's flood.

### 4.4 Safety by construction *(the fence that unlocks autonomy)*
Capability-scoped workspaces (reach = claims, physically). Secrets never enter
workspaces — a broker signs privileged operations. Every action by every
principal (human, agent, orchestrator, model) event-sourced: **universal
undo on the forge itself**. You stop approving each edit not because models
got better, but because the fences got real.

### 4.5 Mission Control — the human command plane *(Inversion 4's cockpit)*
One screen: the intent DAG, every agent's trajectory live, the speculative
union state, conflict heat-map, landing queue, cache-hit rate, spend per
principal. Fleet verbs: approve, redirect, abort, re-plan. Time-travel into
any trajectory ("agent 7's workspace 40 minutes ago"). The attention queue is
the human's whole inbox: risk-ranked, policy-driven, one item at a time.

### 4.6 Tournament intents *(exploration as a primitive)*
Run N competing implementations of one intent as siblings; a judge panel
(machine verdicts + optional human) picks; losers stay addressable as
evidence. Today's "try it twice and clean up the mess" becomes a verb.

### 4.7 The platform for machine actors
Machine-paced API (budgets by policy, not human rate limits), guaranteed-
delivery replayable event stream (the forge's history IS an event log),
everything hash-addressable (hand an agent a conflict, a verdict, a workspace-
at-timestamp as one URL). Multi-region reliability as a marketed feature.

### 4.8 The bridge — nobody has to know you left
One-command GitHub import (history, issues, PRs). Bidirectional mirror,
hugit-authoritative: branches/PRs write back so teammates and OSS contributors
see a normal GitHub repo — **the projection layer makes this trivial**, since
git is already hugit's output format. Exit guaranteed in writing: full-
fidelity export, any moment. Embrace requires the door visibly unlocked.

---

## 5. What ceases to exist

| Today (measured in `../research/`, or lived) | In hugit |
|---|---|
| The reconciliation round after every session | context versioned (Inv. 2) — understanding is never destroyed, so never reconstructed |
| Humans can't review at LLM pace; 20 prose PRs/day | intent ledger + interrogable changes + attention queue (Inv. 4) |
| LLM reviewers at 24–46% precision on raw diffs | claim verification with surgically served context (Inv. 4) |
| Merge conflicts: 27.67% of agent PRs, 540 lines avg | claims collide at dispatch; orthogonal text conflicts regeneratively rebase (Inv. 3) |
| Lockfile/codegen conflicts (#1 git pain) | derived files regenerate, never merge |
| Commit messages, branch names, PR descriptions as agent labor | ceremony is emitted, not performed (Inv. 5) |
| A+B red when A and B are green | speculative union testing before landing |
| CI re-runs everything; 382-run queues | memoized checks + affected targets |
| Agents burning 30–50% of context on cold-start discovery | dispatch packets + semantic index |
| Session dies → understanding dies | context snapshots + journals (Inv. 2) |
| Babysitting agents edit-by-edit | claims as physical fences; universal undo |
| 28 GB of worktrees, 700 stale branches | claim-scoped workspaces, auto-GC with total recall |
| Dependabot floods, rebase storms | silent pre-tested dep landings |
| Force-push data loss, reflog spelunking | event-sourced forge; nothing is ever lost |
| 4,000-line prose logs on failure | structured diagnoses, auto-bisected culprits |
| Per-minute billing on your own hardware; 10–50× shocks | flat, predictable; your compute never metered |
| The founder's Mac as CI infrastructure (load 30) | the laptop is a window; load 1.2 |

---

## 6. Design tenets

1. **Version the intent, the context, and the proof; project the code.** The
   diff is output. (The Inversion — everything else serves it.)
2. **Don't deviate from git as the projection format.** Compatibility is
   sacred; additions are layered, never forked. Nothing to relearn.
3. **Attention is the scarce resource.** Minimal sufficient truth, right
   altitude, speed of thought — for machines and humans.
4. **Understanding, once paid, is never paid again.** Context snapshots,
   journals, memoized checks, semantic index: four faces of one rule.
5. **Conflicts are objects surfaced at dispatch or write time — never walls
   at merge time.** Re-execute before you patch.
6. **Review is interrogation, not reading.** Claims verified by machines;
   narratives followed by humans; drill-down always available.
7. **The ceremony is emitted, not performed.** No ritual labor for agents.
8. **Fences enable autonomy.** Claims as capabilities + universal undo
   convert babysitting into delegation.
9. **Generated files are derived, not authored.** Regenerate; never merge.
10. **Policy is code; judgment is configured once, enforced always.**
11. **Flat, predictable pricing. Never meter the customer's own compute.**
12. **Exit is guaranteed.** Embrace requires the door visibly unlocked.

---

## 7. Open questions for refinement (owner input wanted)

1. **How far does "code as projection" go in v1?** Regenerative rebase as the
   default merge for orthogonal intents, or opt-in per repo while trust
   builds? Draft leans: **opt-in → default**, gated by acceptance-suite
   confidence.
2. **Context snapshot privacy/size:** full conversation capture by default
   (storage is CAS-cheap; value is enormous) with policy-based redaction, or
   opt-in capture? Draft leans: **on by default, tenant-private, redactable.**
3. **Scope: issues/planning.** Intents subsume tickets (acceptance-suite-as-
   the-issue). Own it minimally or integrate trackers? Draft leans: **own it,
   minimally.**
4. **Open-source posture.** Protocol + CLI + intent/context formats open
   (community trust, jj-style), cloud forge paid? Draft leans: **open-core,
   with the object formats published as an open spec** — the "intent format"
   wants to be a standard others adopt.
5. **Does hugit run the agents?** BYO-orchestrator first-class; hosted
   sandboxes as upsell. Draft leans: **yes — hugit is the ground the war is
   fought on, not one of the armies.**
6. **The human surface:** Mission Control web-first; CLI/TUI at parity; IDE
   extension priority?
