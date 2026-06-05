# WP-X14 — deep-link referential integrity (lifecycle)
squad X · M→S · sonnet · 50k · branch: wp/X14 · scheduled: **sprint 2**

## Charter
Prove deep-link referential integrity across the FULL object lifecycle — after
compaction/cold-tier to R2, after mirror round-trip, after tombstoning: every
ledger/intent deep link resolves to its target or to a tamper-evident tombstone,
with ZERO dangling links, ever — a continuous integrity check standing as a
fixture.

## Owned acceptance
① property test across the full object lifecycle — after compaction/cold-tier to R2, after mirror round-trip, after tombstoning: every ledger/intent deep link resolves to its target or to a tamper-evident tombstone
② ZERO dangling links, ever (continuous integrity check as a standing fixture)

## Contract deps
- `EventRecord` (frozen — the ledger/intent deep links under the property test; never modified).
- Surfaces consumed as-built: D1 (compaction/cold-tier to R2), E1 (mirror round-trip), D5 (ledger/intent deep links), X7 (tombstoning). 
- Tenant boundary = HMAC-derived prefixes (CoreLink model).

## Claims
- `crates/hugit-invariants/x14/` (test crate + red-team fixtures + the standing continuous-integrity fixture for X14 only).
- No production crate paths; consumes the D1/E1/D5/X7 surfaces, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X14 row) + the v1.5 note (deep-link referential integrity across the full object lifecycle) · `docs/whitepaper/hugit-v1.md` §9 (event-sourced everything; the boundary) · `docs/product/command-catalog.md` (deep links resolve to golden targets; the human always follows) · frozen `EventRecord`.
- Anchors: item ① a property test that runs deep links through compaction→cold-tier→mirror-round-trip→tombstoning and asserts resolve-to-target-or-tombstone; item ② the standing continuous-integrity fixture asserting ZERO dangling links.
- Conventions: failing suite first; ① is a PROPERTY test over the lifecycle transitions (not a single fixture); ② is a STANDING fixture (continuous), distinguishing resolution (X14) from identity (X9③).

## Implementation notes (every fork PRE-DECIDED)
- **Item ① (lifecycle property test):** generate ledger/intent deep links and drive them through the FULL lifecycle — after compaction/cold-tier to R2 (D1), after mirror round-trip (E1), after tombstoning (X7); assert each link resolves to its TARGET or to a tamper-evident TOMBSTONE. The property holds across every lifecycle transition, not just at rest.
- **Item ② (zero dangling, standing fixture):** the continuous integrity check is a STANDING fixture — ZERO dangling links, ever. It runs as an ongoing invariant, not a one-shot.
- **Resolution ≠ identity:** X14 covers deep-link RESOLUTION (the link lands on target-or-tombstone); X9③ covers IDENTITY (the id is the same id, non-colliding). X14 consumes the same EventRecord links but proves resolution, not identity. X13② is the human-following reading; X14 is the mechanized property/standing-fixture reading; X7② is the no-orphans-survive-erasure reading. They are distinct legs of the same integrity guarantee.
- **tenant boundary = HMAC-derived prefixes; attestation = `AttestationChain` from hugit-contracts** (provenance deep links may carry attestation refs). Consumes D1/E1/D5/X7 as-built; modifies none.
- Sonnet-routed: the contract makes it a deterministic property test + standing fixture (no adversarial judgment).

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–② red→green · cold-verify pass by a non-author · zero writes outside claims · security review at the sprint-2 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x14/` · evidence bundle (the full-lifecycle property-test report across compaction/cold-tier/mirror/tombstone, the standing zero-dangling continuous-integrity fixture) attached to the sprint-2 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + lifecycle property-test + standing-fixture refs), deviations = none | waiver-ref.
