# ADR-0005 — Worktree dock: hook-born physical binding for cost + verification

- **Status:** Accepted — 2026-09-04 (owner-adjudicated; companion design:
  `docs/design/2026-09-04-dock-worktree-v1.md`)
- **Date:** 2026-09-04
- **Applies to:** hugit (CLI/hooks/log/insights) · Omnirouter (gateway metering, contract-owner)
- **Supersedes:** "merge-as-re-execution" as the *only* per-intent cost path (P2);
  this ADR makes cost per-dock via the gateway, runner optional.

## 1. Context

The forge must show **real per-unit cost + verified landing** for agent work.
Naive approaches fail:
- merge-as-re-execution measures a re-run, not what happened; multiplies cost; dies on the runner dep.
- declared ceremony (campaign forms) kills adoption.
- post-hoc reconstruction (`why --walk`) is narrative, not verification.

The turn: **the invocation IS the intent.** `git worktree add` / `git clone` /
first-checkout is the natural moment of birth. The system listens there and
cunea the **dock** — the physical binding.

**Why now:** the owner hold (2026-06-28, `/insights` until cost is non-zero)
lifts with a real per-unit cost source. The gateway makes that possible
without the runner; per-dock cost is the unlock.

## 2. Decisions

1. **Unit of insight = branch (business); dock = physical sub-unit.** `/insights`
   aggregates cost per branch/PR; the dock (worktree/gitdir) is the physical
   sub-unit the cost descends through. Multi-head worktrees on one branch
   aggregate under the branch — never duplicated cost.

2. **Dock identity = gitdir (+ branch hash).** Stable, unique, survives
   re-branch / detached-HEAD / path re-creation; lives in `.git/`, never committed.

3. **Origin = post-checkout hook** (already installed by `hugit init`; worktree-safe,
   detach, non-blocking). No new verb; `dock ls/show` are discovery only.

4. **Env fast-path, cwd as truth; cwd wins on gitdir mismatch** (A1 closed —
   fork/env inheritance never misroutes cost). **One global rule on BOTH sides —
   gateway AND resolver: cwd (physical truth) wins; env is only a fast-path when
   it agrees.** The gateway stamps env (orchestrator belief); the resolver registers
   cwd (physical truth); a mismatch is reconciled to cwd (R5 closed).

5. **Cost from gateway metering** (Omnirouter), stamped `{dock_id, model, tokens,
   cost_usd_micros, ts}`, spooled locally (`OutageQueue`), attested via the
   existing `attest_keyset` seam (#57), landed in the attestation chain `why`
   reads. **Never re-derived; never fabricated.** Runner is optional accelerator.

6. **Degradation is explicit and honest**: repo-scope (no worktree) · `investigated`
   (cost, nothing landed) · `reconciled-late` (cost before dock) · `unlabeled`
   (no dock) · residual bucket. All visible in `/insights`, never swept away.

## 3. Consequences

- **Zero friction**: the user's gesture is standard git; the hook does the rest.
- **Real verification**: `land` per-dock (byte-identity + acceptance-list via
  `run_memoized`/`verdict`) matches the declared promise, not narrative.
- **Decoupled from the irmão**: product runs honest today (spool + `unlabeled`);
  live cost lights when the gateway ships. `CloseResponse` attestation block stays v2.
- **Airtight where git is the model; honest where it isn't** — the design's
  managed edges (F1/F2/F4 degrade to repo-scope; F3 is delivery).

## 4. Rejected alternatives

- **Declared campaign/intent ceremony as the only path** — friction kills adoption;
  campaign remains a *human* declaration; the dock is the physical instance.
- **Merge-as-re-execution as only cost source** — post-hoc, multiplies cost, runner-bound.
- **A new `hugit work` verb** — adds friction; `git worktree add` already is the gesture.