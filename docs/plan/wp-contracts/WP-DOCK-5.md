# WP-DOCK-5 — insights per-branch (F5) + residual buckets (the honest window)

squad: hugit-core · M · opus · 70k · branch: wp/dock-5

## Charter
**What:** the `/insights` projection keyed by **branch** (the business unit —
"what did rate-limiting cost"), with the dock as physical sub-unit; plus the
honest residual buckets (`investigated`, `unlabeled`, `reconciled-late`,
residual) always visible — the window where cost is never swept away.
**Why:** F5 is the product decision that made per-feature cost answerable;
without the buckets it would lie by omission.

## Owned acceptance (VERBATIM from design §6, §7 + ADR-0005 §2.1)
- (F5) — unit of insight = branch (business); dock = physical sub-unit;
  multi-head docks aggregate under one branch total, never duplicated.
- (§7) — buckets: `investigated` (cost, nothing landed), `unlabeled` (landed,
  no cost), `reconciled-late` (cost arrived after dock exist — marked),
  `residual` (never swept, never hidden).
- (M5) — unbound/upgraded repos link intents to the repo-scope dock; their
  cost shows in the branch view without fabrication.

## Contract deps
- Reconcile buckets (WP-DOCK-3), cost samples (WP-DOCK-4), intent/campaign
  rollups (`hugit_ledger`), branch refs (EventLog).
- `hugit-serve` handlers (`/v1/insights` seam — where the window reads).

## Claims (paths)
- `crates/hugit-cli/src/dock/insights.rs` — the branch-keyed projection +
  buckets (extends WP-DOCK-3's).
- `crates/hugit-serve/src/handlers/insights*.rs` — the endpoint reading the
  projection (additive; honors the owner's "hold until cost non-zero").
- `crates/hugit-cli/tests/dock_insights_*.rs`.

## Dispatch packet
- This contract + ADR-0005 §2.1 (F5) + design §7 buckets.
- Anchor: WP-DOCK-3's reconcile output; `hugit-serve` insights handler pattern.

## Properties (Lamport-style)

**I1 (safety — branch totals):** `insights(branch).cost == Σ(dock costs on
that branch)`; never inflated, never double-counted across multi-head docks.

**I2 (safety — buckets visible):** every cost sample / landed commit is
attributable in the projection: either branch-matched, or in an EXPLICIT
named bucket. No silent `null`/absent entry for a sample that has something.

**I3 (safety — no fabrication):** the projection shows `None`/`zero` for a
branch with no attested cost — NEVER a derived estimate.

**L5 (liveness — fresh):** the projection reflects the latest appended
records (no stale-forever view; recomputed from the log on read).

## Implementation notes (pre-decided)
- 1. Projection is DERIVED from the log at read time (no parallel store) —
  `why`/`insights` share the log as single source of truth.
- 2. Buckets render explicitly (names, not footnotes).
- 3. `/insights` route stays owner-gated (the 2026-06-28 hold) until cost is
  non-zero; the projection is buildable + tested behind the gate.

## DoD
- Global gate green.
- F5/I1-I3/L5 red→green, hermetic + e2e (branch with 2 docks, investigated,
  unlabeled, reconciled-late, residual).
- Cold-verify pass by non-author.

## Completeness
All owned items green · zero writes outside claims · evidence bundle attached
to SEAL.

## Return shape (SEAL)
status, evidence refs, deviations.