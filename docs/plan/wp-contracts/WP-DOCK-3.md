# WP-DOCK-3 — dock lifecycle + reconciliation (A4: cost↔commit by branch)

squad: hugit-core · M · opus · 80k · branch: wp/dock-3

## Charter
**What:** the dock lifecycle end-to-end: open → (close | ghost) → reconciled,
with the close-time reconciliation that matches *cost* (which dock spent) to
*commits* (which branch landed what) — the audit that makes per-unit cost
honest, not just attributed. Also: `investigated` (cost but nothing landed)
and `unlabeled`/residual buckets visible in `/insights`.
**Why:** A4 is the difference between "numbers per dock" and "truth per
branch".

## Owned acceptance (VERBATIM from design §6, §7)
- (A4) — close reconcile by branch: commits of that branch match cost of that
  dock; mismatch → `reconciled` bucket honest; cost w/o commits =
  `investigated`; commits w/o cost = `unlabeled` visible.
- (§7 F5) — unit of insight = branch (business); dock = physical sub-unit;
  multi-head worktrees on one branch aggregate under the branch, never
  duplicate cost.
- (R4) — ghosts reconciled: reconciliation runs on ghosts; dock closed on
  durable log, stays `ghost` in listings until reconciled.
- (M5) — unbound intents link to the auto-coined repo-scope dock.
- (B4) — 2 worktrees concurrent (hooks racing the log): reuse the existing
  PR `filelock` — reconciliation and coinage serialize; never a torn append.

## Contract deps
- Dock record (WP-DOCK-1), resolver + ghost (WP-DOCK-2).
- `intent` sidecar + `campaign` (the acceptance list; `IntentionSidecar`).
- `hugit_ledger` rollup (branch rollups; cost rollups exist for campaigns).

## Claims (paths)
- `crates/hugit-cli/src/dock/reconcile.rs` — close reconcile by branch.
- `crates/hugit-cli/src/dock/close.rs` — finalize (mark closed, run
  reconciliation, append close record).
- `crates/hugit-cli/src/dock/insights.rs` — the per-branch projection (F5)
  + residual/unlabeled buckets.
- `crates/hugit-cli/tests/dock_reconcile_*.rs`.

## Dispatch packet
- This contract + design §6, §7 (F5, A4, R4, M5).
- Anchor: WP-DOCK-1/2 dock record + resolver; existing `land`/`verdict`
  machinery for acceptance.

## Properties (Lamport-style)

**R1 (safety — exact-once reconciliation):** every cost sample and every
commit is attributed to EXACTLY ONE bucket at close: dock-matched, reconciled,
investigated, or unlabeled. No sample/commit appears in two buckets.

**R2 (safety — branch aggregation):** the branch view shows the SUM of its
docks' costs — multi-head docks aggregate, never duplicated. `Σ(docks on
branch)` == branch total.

**R3 (safety — honest gaps):** a dock with cost but no landing is
`investigated` (never hidden); commits with no dock cost are `unlabeled`
(never silently zero).

**L3 (liveness — close):** IF a dock's gitdir is removed OR `work close` is
called, THEN (eventually) the dock is marked `closed` on the durable log AND
its reconciliation runs (buckets finalized). No open dock with a dead gitdir
remains forever (ghost → closed).

## Implementation notes (pre-decided)
- 1. Reconcile input: dock records (WP-DOCK-1/2) + cost spool (F2 seam, see
  WP-DOCK-4) + commits on the branch (from EventLog ref updates).
- 2. Bucket priority: dock-matched > reconciled > investigated > unlabeled.
- 3. `close` is idempotent (re-running on a closed dock = no-op, returns
  existing result).
- 4. The per-branch view (F5) reads the log projection — no parallel store.

## DoD
- Global gate green.
- A4/R4/M5 + F5 red→green, hermetic AND e2e (two worktrees same branch,
  cost-without-landing, landing-without-cost, ghost close).
- R1-R3 + L3 proven by ≥1 hermetic test each.
- Cold-verify pass by non-author.

## Completeness
All owned items green · zero writes outside claims · evidence bundle attached
to SEAL.

## Return shape (SEAL)
status, evidence refs, deviations.