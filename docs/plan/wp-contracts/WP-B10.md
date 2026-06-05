# WP-B10 — negative scope
squad B · S · sonnet · route sonnet · budget 40k · branch: wp/B10

## Charter
Prove the Phase-B negative-scope invariants in `hugit-queue` (negative-scope
test module): no claim/lease is acquired at dispatch — conflict discovery
happens ONLY at landing/union (the mechanism is absent) — and rebase in phase B
is textual-fallback only, with the regenerative path absent/disabled. These are
ABSENCE assertions: the demoted features must be provably not present in Phase B.

## Owned acceptance
B10 owns all 2 items of B10 (no split). VERBATIM from decomposition v2.0 §2:

① no claim/lease acquired at dispatch — conflict discovery happens ONLY at
landing/union (assert mechanism absent) · ② rebase in phase B is textual-
fallback only — regenerative path absent/disabled (assert)

## Contract deps
Consumes from `hugit-contracts` (frozen): **QueueApi** (to assert conflict
discovery is landing-only), **RunnerLease** (to assert no lease is acquired at
dispatch). Asserts the ABSENCE of any dispatch-time claim mechanism and of the
regenerative rebase path in Phase B. No contract type authored or changed here.

## Claims
`crates/hugit-queue/tests/negative_scope/` (the absence-assertion test
fixtures + mechanism-absence checks). Does NOT touch `crates/hugit-queue/src/core/`
(B4a), `src/github/` (B4b), or `budget/` (C7) source — it asserts ABOUT them.

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B10.md`).
- `hugit-contracts` (QueueApi, RunnerLease) + the B4a/B4b queue as built.
- whitepaper §6.4 (landing = discovery by speculation, not prediction), §6.3
  (regenerative rebase — the path that is DISABLED in phase B; textual fast-path
  is the fallback used here).
- command-catalog (claims-at-dispatch 🔬 GATED — "demoted from phase B
  entirely. Conflicts are discovered at landing"; Regenerative rebase ⛔ CUT
  from B — "textual fallback only in phase B").
- decomposition v1.5 adjudication ("no-fake-intents at runtime" mapping).
- The failing acceptance suite at `tests/acceptance/wp-B10/`.
Estimated packet size: ~28k tokens (inside 40k).

## Implementation notes
Every fork pre-decided:
- **No claim/lease at dispatch (①):** assert that the Phase-B dispatch path
  acquires NO claim and NO `RunnerLease` for conflict purposes — conflict
  discovery happens ONLY at landing/union (B4's union test is the conflict
  oracle, whitepaper §6.4). The assertion is that the dispatch-time
  claim-acquisition MECHANISM is absent (not merely unused) — there is no code
  path to acquire one.
- **Textual-fallback rebase only (②):** assert that Phase-B rebase uses the
  textual fast-path ONLY (whitepaper §6.3: `claims(I) ∩ Δ = ∅ → textual
  fast-path`); the regenerative (re-execution) path is ABSENT/DISABLED in
  phase B (command-catalog: regen rebase ⛔ CUT from B). Assert the regen
  codepath is unreachable in the Phase-B build.
- **These are negative/absence proofs** — the WP adds no feature; it pins that
  two demoted features are provably not present, guarding against scope creep.
- **CoreLink consumed as CLIENT only** — zero server changes.

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①② red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
Items ①② green; zero writes outside `crates/hugit-queue/tests/negative_scope/`;
evidence bundle (dispatch-time claim/lease-absence proof, regen-path-disabled
proof) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, mechanism-absent assertions),
deviations = none | waiver-ref.
