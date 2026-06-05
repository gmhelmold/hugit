# WP-C8 — shadow checks
squad C · M · opus · 70k · branch: wp/C8

## Charter
Shadow checks on workspace-snapshot cadence (NOT per-write): N writes in one
snapshot window → exactly ONE shadow pass at the boundary; shadow runs decrement
tenant budget and the cap halts shadows while explicit jobs proceed; default-off,
per-repo opt-in; a failing shadow is signal/event only and NEVER gates an explicit
job; per-tenant cap isolation holds. Depends on C2.

## Owned acceptance
① N writes in one snapshot window → exactly ONE shadow pass at boundary (not N, not 0) · ② shadow runs decrement tenant budget; cap halts shadows, explicit jobs proceed per policy · ③ default-off; per-repo opt-in flag scoped to repo, zero runs when off · **④(R2) a failing shadow surfaces as signal/event and NEVER gates/blocks/fails any explicit job; a passing shadow produces an observable result** · **⑤(R2) per-tenant cap isolation: tenant A exhausting its shadow budget leaves tenant B's shadows unaffected**

## Contract deps
- `ShadowPolicy {cadence, budget, optin}` (frozen, from the WP-00 (+) set —
  THE policy object C8 enforces; consumed, never modified).
- `CheckResult` (frozen — a shadow pass produces an observable result; consumed).
- C7's per-tenant budget engine (consumed: shadow runs decrement the C7 budget;
  C7 owns the budget, C8 owns the shadow scheduler that draws on it).

## Claims
- `crates/hugit-checks/shadow/` — the snapshot-cadence shadow scheduler,
  opt-in flag handling, budget decrement + cap halt, and the non-gating
  signal/event surface. Disjoint from `hugit-checks/regen` (C4),
  `hugit-checks/affected` (B3), and the C7 budget engine.

## Dispatch packet
- Files received: this contract · decomposition §3 (C8 row) · warp-10-days
  (C8 is a (+) addition; shadow described in command-catalog) · command-catalog
  Phase C ("Shadow checks 🔧 HARDENED: NOT on every write … on workspace
  snapshot cadence, budget-capped per tenant, opt-in per repo") · whitepaper
  §6.2 (shadow: on every workspace snapshot, speculatively evaluate affected(Δ)) ·
  C7 SEALed budget API · `hugit-contracts` (`ShadowPolicy`, `CheckResult`).
- Anchors: the snapshot-window boundary as the single trigger; the opt-in flag
  scoped per repo; the budget-decrement + cap-halt; the non-gating surface.
- Conventions: failing acceptance suite committed first; multi-tenant cap-isolation
  fixture for ⑤.

## Implementation notes (every fork PRE-DECIDED)
- **Cadence = snapshot boundary, NOT per write (①):** N writes within one
  snapshot window collapse to exactly ONE shadow pass at the boundary — not N,
  not 0 (cost/noise were the engineer-review objection; this is the answer).
- **Budget (②):** shadow runs decrement the tenant's C7 budget; when the cap is
  hit, shadows halt while explicit jobs proceed per policy — shadows are the
  yielding workload.
- **Default-off + opt-in (③):** `ShadowPolicy.optin` is per-repo, scoped to that
  repo; zero shadow runs when off.
- **Non-gating (④):** a failing shadow surfaces as a signal/event ONLY — it never
  gates, blocks, or fails any explicit job; a passing shadow produces an
  observable result. Shadow is ambient truth, never a gate.
- **Cap isolation (⑤):** tenant A exhausting its shadow budget leaves tenant B's
  shadows unaffected — budgets are per-tenant (rides C7's per-tenant accounting).
- Shadow execution runs on the C2 runner (container-per-job, Hetzner box;
  Firecracker path documented, not built).

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims.

## Completeness
All owned items green · zero writes outside `crates/hugit-checks/shadow/` ·
evidence bundle (one-pass-per-window proof, budget-decrement + cap-halt trace,
default-off/opt-in assertion, non-gating proof, per-tenant cap-isolation
fixture) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.
