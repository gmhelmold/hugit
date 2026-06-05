# WP-C10 — pricing no-shock
squad C · S · sonnet · 50k · branch: wp/C10

## Charter
Pricing-no-shock guard: driving a tenant to budget exhaustion on EACH metered
surface (runner minutes, shadow spend, storage) makes the system cap/degrade
(pause or fall back) with a pre-exhaustion warning, and generates ZERO overage
charge — flat means flat. Depends on C7's budget engine.

## Owned acceptance
① driving a tenant to budget exhaustion on EACH metered surface (runner minutes, shadow spend, storage) → system caps/degrades (pauses or falls back) with pre-exhaustion warning · ② zero overage charge generated — flat means flat (billing fixture assert)

## Contract deps
- `QueueApi` (frozen — C10 asserts behavior over the C7 budget surfaces; consumed,
  never modified).
- C7's per-tenant budget + metering engine (consumed: C10 drives it to exhaustion
  and asserts cap/degrade + zero-overage; C7 owns the budgets, C10 owns the
  no-shock guard + billing assertion).

## Claims
- `crates/hugit-queue/budget/no_shock/` — the per-surface exhaustion-drive guard
  (runner minutes / shadow spend / storage), the pre-exhaustion warning, and the
  zero-overage billing fixture/assertion. Disjoint from the C7 budget core
  modules under `hugit-queue/budget/`.

## Dispatch packet
- Files received: this contract · decomposition §3 (C10 row) · whitepaper §11
  (pricing doctrine four laws: flat per unit-that-scales; never meter the
  customer's own compute; never usage-billing whiplash) · command-catalog
  ("the pricing doctrine") · C7 SEALed budget API · `hugit-contracts`
  (`QueueApi`).
- Anchors: the three metered surfaces (runner minutes, shadow spend, storage);
  cap/degrade with pre-exhaustion warning; the billing fixture proving zero
  overage.
- Conventions: failing acceptance suite committed first; per-surface exhaustion
  fixtures + a billing fixture for ②.

## Implementation notes (every fork PRE-DECIDED)
- **Three metered surfaces, each tested (①):** runner minutes, shadow spend, and
  storage — drive each to exhaustion independently; the system caps or degrades
  (pauses or falls back) and emits a pre-exhaustion warning before the wall.
- **Zero overage (②):** flat means flat — exhaustion never generates an overage
  charge; the billing fixture asserts $0 overage. (Whitepaper §11: never
  usage-billing whiplash; never meter the customer's own compute.)
- Runner-minute exhaustion rides the C2 runtime; shadow-spend rides C8's budget
  decrement; storage rides the CAS accounting — but all three are metered through
  C7's per-tenant budget engine, which C10 drives. C10 owns the guard + billing
  assertion, not the underlying meters.
- Degrade/fallback honors the §9 lock-5 degradation invariant: cap/pause is an
  honest surfaced state, never a silent drop or a false green.

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims.

## Completeness
All owned items green · zero writes outside `crates/hugit-queue/budget/no_shock/` ·
evidence bundle (per-surface exhaustion → cap/degrade + warning traces,
zero-overage billing-fixture assertion) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.
