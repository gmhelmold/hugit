# Design — Worktree-dock: hook-born physical binding for cost + verification

- **Date:** 2026-09-04
- **Status:** Proposed (owner-approved direction as of 2026-09-04; ADR-0005 freezes the decisions)
- **Applies to:** hugit (CLI/hooks/log/insights seam) · Omnirouter (gateway metering, irmão, contract-owner)
- **Supersedes:** the P2 "merge-as-re-execution" cost path as the *only* answer to per-intent cost; this design makes cost land per-dock via the gateway, runner becomes optional accelerator.

## 1. Problem

The drive-to-awesome question: how does the forge show **real, per-unit cost AND verified landing** for agent work — when cost only exists *after* the model call, and attribution needs a binding that exists *before* the work starts?

**Why now (M3):** the owner hold (2026-06-28, "hold the first public `/insights` until cost is non-zero") is lifted *only* when cost is real and non-zero per unit. Today the per-intent cost path is `None` (honest) until the runner ships. This design makes the first honest per-unit `/insights possible without the runner — the gateway is the source; the runner becomes an optional accelerator. That is the unlock that makes this worth building now.

The naive answers fail:
- **Merge-as-re-execution** (re-run the diff off-box to measure): measures a *re-run*, not what happened; multiplies cost; post-hoc reconstruction disguised as attribution; dies on the runner dependency.
- **Declared campaign/intent ceremony**: friction — agents and models won't fill forms; "the dock before the barque" is exactly the ceremony that kills adoption.
- **Post-hoc reconstruction** (what `why --walk` does today): audit-*narrative*, not audit-*verification*; can't answer "did the committed work match the declared promise".

## 2. The turn

**The invocation IS the intent.** The gesture that opens the work — `git worktree add`, `git clone`, a session's first checkout — is the natural declaration. No new command, no form. The system only *listens* at the moment the work is born, and cunea **the dock** (the physical binding) there.

The binding is the invariant:
> **The worktree is the dock. The branch is the barque. The intent record is the manifest. The model calls inside it are the cargo. The cost arrives later and lands on the dock — which already existed when the barque left.**

Temporal paradox resolved *physically*: the dock (a place) exists before the work; the cost (the bill) arrives after; the binding is mechanical (cwd + env), never reconstructed.

## 3. Identities (the load-bearing choices)

### 3.1 gitdir is the physical identity
The worktree's **gitdir** (`.git/worktrees/<name>`) is:
- unique per worktree,
- stable across re-branch / detached-HEAD / path re-creation (it is the *workspace* identity),
- never committed (lives in `.git/`).

`dock_id = hash(gitdir + branch)`. Stable, unique, survives everything that matters.

### 3.2 Branch is the business identity
The *insight* unit is the **branch/PR** (what the business asks: "how much did rate-limiting cost?"). Two worktrees on the same branch (multi-head) aggregate under one branch identity — **never duplicated cost**. `dock ls` shows worktrees; `/insights` is keyed by branch, with the doca as the physical sub-unit (F5 decision).

### 3.3 Binding of cost: env fast-path, cwd as truth
- `HUGIT_DOCK_ID` exported at session start = fast-path (orchestrators set it; inherited by all descendants).
- The resolver (cwd → gitdir → dock) is the truth.
- **A1 closed — cwd beats env when gitdir differs:** if env's dock.gitdir ≠ cwd's gitdir, env is *ignored*, the child dock is coined, parent_id recorded. Cost never lands on the wrong dock because of fork-inheritance.
- **R5 closed — the SAME "cwd wins" rule applies on BOTH sides (gateway and resolver).** The gateway stamps whatever the caller presents (env fast-path OR cwd-resolved); the resolver registers the cwd truth. If the gateway stamped env A (orchestrator belief) but the process really lives in worktree B, the resolver's registration (B) is authoritative and the gateway's A is reconciled to B via the same mechanism as A4 — *otherwise cost is measured on dock A and committed on dock B and reconciliation never matches*. **One global rule: cwd (the physical truth) wins on both sides; env is only a fast-path when it agrees.**

## 4. The hook as origin (zero-friction origin)

`git worktree add -b feat/x path` (or `git clone`) fires **post-checkout** (already installed by `hugit init`, worktree-safe, detach, non-blocking). Inside: detect new gitdir + `flag==1` → coin the dock:

```
dock_id = hash(gitdir + branch)
charter = derived from branch name ("feat/rate-limit" → "add rate limit")   [marked `derived`]
dock.record → .hugit/log.json   (the TRUTH, durable, hash-chained)
marker → .git/worktrees/<name>/hugit-dock   (created_ts + pid)
```

Self-heal: if the resolver finds a gitdir without a marker, it coins (the dock always exists by reading-time). Lazy-close: when the worktree is removed (no hook), the gitdir disappears → the dock closes at next read.

> **R1 closed — `git clone` does NOT fire the local post-checkout.** A clone of a repo without hugit hooks creates no dock at origin. The dock then has to come from the OUTSIDE: the orchestrator (who knows hugit) exports `HUGIT_DOCK_ID` at clone time; or the resolver self-heals by gitdir (covers *cost*, but charter/verification need the dock coined before). **Rule: clone-without-hooks ⇒ doca from env (orchestrator) or self-heal; never silently missing.**

> **R2 closed — no silent env-vs-cwd divergence.** When the hook/registrar coins cwd dock B while env says dock A (gitdir mismatch), it MUST log `parent_id = A` AND emit a visible warning (stderr, but never fail the git op). The orchestrator must be able to detect that the child became a different dock — without this it is silent divergence between what the orchestrator *believes* and what the system *recorded*.

> **R4 closed — lazy-close is proactive, not read-lazy.** A dock whose gitdir vanished is marked `ghost` (open-with-worktree-gone) **at the moment the resolver observes it** — never waits for an explicit read of dock *X* to discover *X* is gone. `dock ls` lists ghosts; reconciliation (A4) runs on ghosts; the dock is closed on the durable log (record appended) and stays `ghost` in listings until reconciled. Without this, `investigated`/reconcile never runs on the right dock.

## 5. Cost metering (gateway, not runner)

- The **gateway** (Omnirouter, irmão) measures each model call and stamps `{dock_id, model, tokens, cost_usd_micros, ts}`. It sees the bill where the model actually runs.
- **Spool local** (reuse `OutageQueue`): cost is spooled per-dock locally and flushed when the gateway is up — offline never loses metering.
- **Attestation**: the minted entry is signed / ids verified via the existing `attest_keyset` seam (#57) — cost lands in the attestation chain (which `why` already reads), never re-derived.
- **F3 mitigation — decoupled from the irmão**: the product runs honest (spool + local, `unlabeled`/zero buckets) today; the live cost lights when the gateway ships. The `CloseResponse` attestation block stays v2 (per existing CLAUDE.md decision).

## 6. Verification per unit (the auditor's question)

`land` per-dock:
- **byte-identity** — already exists (the mirror's per-push verify).
- **acceptance-list execution** — reuse `run_memoized` / `verdict` (the existing machinery): the dock's acceptance list runs, the union verdict decides. This is *verification* (matching the declared promise), not *narrative*.

`hugit why --dock` aggregates (branch → docks → commits → cost).

## 7. The holes, closed or managed

### Closed (deterministic, tested)
| # | Hole | Fix |
|---|---|---|
| A1 | Boss env-fork inheritance | cwd beats env when gitdir differs |
| A2 | Cost before dock (micro-window) | `reconciled-late` bucket; never fabricated |
| A3 | Path re-created (same name) | marker carries `created_ts+pid`; reborn ⇒ new dock |
| A4 | Split-brain cost↔commit | close reconcile by branch; `investigated` bucket |
| B3 | jj colocated | has `.git` ⇒ same auto-dock covers |
| B4 | 2 worktrees concurrent | reuse PR `filelock` |
| B5 | Wire with irmão drifts | contract frozen v1 (DTO + conformance vector + tripwire) |
| R1 | `git clone` fires no local post-checkout | doca from env (orchestrator, at clone time) or self-heal by gitdir; never silently missing |
| R2 | Silent env-vs-cwd divergence | hook logs `parent_id` + visible warning (stderr, never fails the git op) |
| R4 | Lazy-close is read-lazy (ghost never marked) | resolver marks `ghost` at observation; reconcile runs on ghosts; closed on the durable log |
| R5 | Split-brain gateway↔resolver | one global rule: cwd wins on BOTH sides; env only agree-fast-path; mismatch reconciled to cwd |

### Managed (existential; mitigated by incentive + degradation, never fabrication)
| # | Edge | Managed as |
|---|---|---|
| F1 | No worktree (checkout principal) | repo-scope dock (coarse but real); worktree adds the fine; migrate smoothly |
| F2 | No orchestrator isolation | enabler is *already* standard practice (worktree-per-agent); product rewards, never requires |
| F3 | Gateway/live not shipped yet | product works honest today (spool + `unlabeled`); live cost owner-gated with the irmão |
| F4 | jj standalone (no `.git`) | repo-scope honest; never fabricated per-unit |
| C5 | `CloseResponse` attestation block | v2 (existing tracked additive change) |
| M5 | Existing repos (no docks yet) | resolver **auto-coins a repo-scope dock** on first read; existing unbound intents link to it; state marked `upgraded` — honest, no fabrication, no migration ceremony |

### The core honesty rule (never bends)
**Cost is measured at the model call; never re-derived, never fabricated.** `unlabeled` / `reconciled-late` / `investigated` / residual buckets are *visible*, not swept away.

## 8. Phases

| | Scope | Test |
|---|---|---|
| **F1** | hook auto-dock + `dock ls/show` + resolver cwd→gitdir→dock + A1-A4 + B3-B4 + lazy-close + self-heal | hermetic + e2e (real local worktree) |
| **F2** | contract metering v1 (frozen DTO + vector + spool + flush) + `/insights` per-branch (F5) | e2e + live owner-gated (irmão) |
| **F3** | `land` per-dock (byte-identity + acceptance via verdict) + buckets + reconciliation | hermetic + e2e |

## 9. Decisions to ratify (ADR)

1. **Unit of insight = branch (business); dock = physical sub-unit.** (F5)
2. **Dock identity = gitdir (+branch hash); never committed.**
3. **Origin = post-checkout hook** (already installed); no new verb; `dock ls/show` only as discovery.
4. **Env fast-path, cwd truth; cwd wins on gitdir mismatch.**
5. **Cost from gateway metering, spooled, attested; never derived; runner optional.**
6. **Degradation is explicit and honest** (repo-scope / investigated / reconciled-late / unlabeled), never fabrication.

## 10. What is NOT this design
- Not a new forge, not a new VCS, not a new CI.
- Not meta-command ceremony (`campaign` remains a declaration for humans; the dock is the physical instance).
- Not multi-tenant / identity (downstream, githugr infra).
- Not a runner dependency for the cost killer (gateway is the source).