# WP-X13 — legibility × degradation/erasure
squad X · M · opus · 60k · branch: wp/X13 · scheduled: **sprint 2**

## Charter
Prove the human can ALWAYS follow, in every substrate state: with the
intelligence layer DEGRADED, the down-zoom (raw-commit view, deep links, `why`)
still resolves via plain git OR fails HONESTLY ("layer unavailable") — never a
silent 404/blank; and after an erasure cascade, following any chain reaches an
honest tombstone, never a broken link. Legibility composed with degradation and
erasure.

## Owned acceptance
① with the intelligence layer DEGRADED: the human's down-zoom (raw-commit view, deep links, `why`) still resolves via plain git OR fails HONESTLY (explicit "layer unavailable"), never a silent 404/blank
② after an erasure cascade: following any chain reaches an honest tombstone, never a broken link — the human can ALWAYS follow, in every substrate state

## Contract deps
- `EventRecord` (frozen — the chain/deep-links item ② follows to a tombstone; never modified).
- Surfaces consumed as-built: D5 (ledger/watch deep-links), D10 (`why`), D2 (raw-commit view via plain git), X7 (the erasure cascade producing tombstones).
- Tenant boundary = HMAC-derived prefixes (CoreLink model). Lock 5 (degradation invariant): a valid git repo keeps serving.

## Claims
- `crates/hugit-invariants/x13/` (test crate + red-team fixtures for X13 only).
- No production crate paths; consumes the D5/D10/D2/X7 surfaces, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X13 row) + the v1.5 note (legibility×degradation/erasure) · `docs/whitepaper/hugit-v1.md` §9 (lock 5 degradation invariant) · `docs/product/command-catalog.md` (`hugit why`; the Ledger two-zoom; "the human always follows"; degradation invariant) · frozen `EventRecord`.
- Anchors: items ①–② each a test module under `crates/hugit-invariants/x13/`; ① degrades the intelligence layer and asserts the down-zoom resolves via plain git OR fails honestly; ② erases a cascade and follows every chain to an honest tombstone.
- Conventions: failing suite first; the FAIL state is asserted to be EXPLICIT ("layer unavailable"), never a silent 404/blank; every chain terminates in a resolvable target OR a tamper-evident tombstone.

## Implementation notes (every fork PRE-DECIDED)
- **Item ① (legibility under degradation):** with the intelligence layer DEGRADED, the human's down-zoom — raw-commit view (D2 plain git), deep links (D5), `why` (D10) — either resolves via plain git OR fails HONESTLY with an explicit "layer unavailable" state. The test asserts NEVER a silent 404/blank: degradation is honest, the down-zoom always lands somewhere truthful. This is the human-side reading of lock 5 (degradation invariant).
- **Item ② (legibility under erasure):** after an erasure cascade (X7), following ANY chain reaches an honest TOMBSTONE — never a broken link. X7② owns "no orphaned refs survive"; X14 owns "every deep link resolves to target-or-tombstone" as a lifecycle property; X13② owns the HUMAN-following reading: in every substrate state the human can always follow to a truthful endpoint.
- **The composition:** X13 is the legibility intersection — it does not re-prove degradation (X11), the erasure cascade (X7), or deep-link integrity (X14); it proves the HUMAN can always follow under both degradation AND erasure.
- **tenant boundary = HMAC-derived prefixes; attestation = `AttestationChain` from hugit-contracts** (chains followed in ② are attestation/provenance chains). Consumes D5/D10/D2/X7 as-built; modifies none.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–② red→green · cold-verify pass by a non-author · zero writes outside claims · security review at the sprint-2 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x13/` · evidence bundle (the degraded down-zoom resolve-or-honest-fail proof across raw-view/deep-links/`why`, the post-erasure follow-to-tombstone proof) attached to the sprint-2 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + degraded-resolve + tombstone-follow refs), deviations = none | waiver-ref.
