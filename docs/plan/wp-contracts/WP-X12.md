# WP-X12 — erasure × provenance × mirror
squad X · M · opus · 70k · branch: wp/X12 · scheduled: **sprint 2**

## Charter
Prove the three-way composition of erasure, provenance, and the mirror: after an
erasure request the attestation chain remains INDEPENDENTLY verifiable with the
erased object as a tamper-evident TOMBSTONE (never silently re-linked), and the
mirror-side erasure obligation (data already replicated to GitHub) is discharged
OR explicitly surfaced as residual risk — that disclosure being part of the
export/exit proof.

## Owned acceptance
① after an erasure request the attestation chain remains independently verifiable with the erased object as a tamper-evident TOMBSTONE (never silently re-linked)
② the mirror-side erasure obligation (data already replicated to GitHub) is discharged or explicitly surfaced as residual risk — and that disclosure is part of the export/exit proof

## Contract deps
- `AttestationChain` (frozen — item ①'s chain that stays independently verifiable over the tombstone; never modified).
- `ExportSchema` (frozen — item ②'s residual-risk disclosure is a stated element of the export/exit proof).
- `EventRecord` (frozen — the provenance links that must terminate in a tombstone, not a re-link).
- Surfaces consumed as-built: X7 (the erasure cascade), E1 (the mirror), E5 (export/exit proof). Tenant boundary = HMAC-derived prefixes (CoreLink model).

## Claims
- `crates/hugit-invariants/x12/` (test crate + red-team fixtures for X12 only).
- No production crate paths; consumes the X7/E1/E5 surfaces, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X12 row) + the v1.x erasure notes (X7④ precedence; X7 cascade) · `docs/whitepaper/hugit-v1.md` §9 (Provenance; the boundary) + §13.2 · `docs/product/command-catalog.md` (one-way mirror; export anti-lock-in) · frozen types above.
- Anchors: items ①–② each a test module under `crates/hugit-invariants/x12/`; ① erases an object then asserts the chain still verifies with a tamper-evident tombstone (no silent re-link); ② asserts the mirror obligation is discharged OR the residual risk is disclosed IN the export/exit proof.
- Conventions: failing suite first; ① asserts the tombstone is tamper-EVIDENT (verifiable as a deliberate erasure marker, not a broken link); ② asserts the disclosure is present in the `ExportSchema`-validated export.

## Implementation notes (every fork PRE-DECIDED)
- **Item ① (attestation over tombstone):** erase an object (via the X7 cascade) and assert the `AttestationChain` remains INDEPENDENTLY verifiable — the erased object resolves to a tamper-evident TOMBSTONE, NEVER silently re-linked to some substitute. X7③ owns the re-seal-or-fail-closed proof at erasure time; X12① owns that the chain stays verifiable AFTERWARD with the tombstone in place. This consumes X7's cascade surface, never modifies it.
- **Item ② (mirror obligation × exit proof):** data already replicated to the GitHub mirror creates an erasure obligation that physical control over GitHub cannot fully guarantee. Assert the obligation is EITHER discharged (mirror-side erasure executed + verified) OR explicitly surfaced as RESIDUAL RISK — and that the residual-risk disclosure is a stated element of the export/exit proof (validated against `ExportSchema`, cf. E5⑥/E5⑦). The honest disclosure IS the deliverable when full discharge is not provable.
- **attestation = `AttestationChain` from hugit-contracts; tenant boundary = HMAC-derived prefixes.** Consumes X7/E1/E5 as-built; modifies none. X12 is the intersection WP — it does not re-prove the cascade (X7), the mirror (E1), or the export (E5); it proves they COMPOSE.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–② red→green · cold-verify pass by a non-author · zero writes outside claims · security review at the sprint-2 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x12/` · evidence bundle (the post-erasure independent chain-verification with tamper-evident tombstone + no-silent-re-link proof, the mirror-obligation discharge-or-disclosure proof tied into the `ExportSchema` exit proof) attached to the sprint-2 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + tombstone-verification + exit-proof-disclosure refs), deviations = none | waiver-ref.
