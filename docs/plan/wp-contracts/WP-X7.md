# WP-X7 — erasure cascade (right-to-erasure)
squad X · L→M · opus · 80k · branch: wp/X7 · scheduled: **sprint 2**

## Charter
Prove a data subject's personal data is provably erased across EVERY store —
CAS, provenance/ledger, context, the GitHub mirror, and the experiment corpus —
with no orphaned provenance refs, attestation chains that re-seal or fail
CLOSED, and the erasure×seal precedence resolved (lawful corpus erasure is
permitted despite the seal and invalidates the gate fail-closed).

## Owned acceptance
① a data subject's personal data provably erased across CAS + provenance/ledger + context store + the GitHub mirror **+ the experiment corpus (R10)**
② no orphaned provenance refs survive erasure
③ attestation chains re-seal or fail CLOSED after erasure (never silently broken)
④(R10) erasure × seal precedence: lawful erasure of a corpus datapoint is PERMITTED despite the seal, and the gate verdict invalidates FAIL-CLOSED (claims/regen re-pin until re-evaluation) — never blocked by the seal, never a silently broken seal

## Contract deps
- `AttestationChain {tree, def, runner, model, principal, sig}` (frozen — item ③ re-seals or fail-closes it; never modified).
- `RegenGate {optin_scope, repass, indep_verdict}` (frozen — item ④'s gate verdict that invalidates fail-closed on corpus erasure).
- `EventRecord` (frozen — ledger/provenance refs item ② scans for orphans).
- `ExportSchema` (frozen — the mirror-erasure leg ties into the export/exit proof; X12 owns the residual-risk disclosure).
- Surfaces consumed as-built: CAS/AC (B2), context store (D11), mirror (E1), experiment corpus (D8). Tenant boundary = HMAC-derived prefixes (CoreLink model).

## Claims
- `crates/hugit-invariants/x7/` (test crate + red-team fixtures for X7 only).
- No production crate paths; consumes the five stores' surfaces, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X7 row) + §7 (DAG: X7⇠{D1,X3,E1}) + the v1.10 note (X7④ erasure×seal precedence; the X7/D8⑤ structural contradiction resolved) · `docs/whitepaper/hugit-v1.md` §9 + §13.2 · frozen types above.
- Anchors: items ①–④ each a test module under `crates/hugit-invariants/x7/`; ① seeds a subject's data into all five stores then erases and scans each for absence; ④ erases a SEALED corpus datapoint and asserts the gate invalidates fail-closed.
- Conventions: failing suite first; every erasure assertion verifies ABSENCE across each store (scan, not flag); ③/④ assert re-seal OR fail-closed (never silently broken).

## Implementation notes (every fork PRE-DECIDED)
- **Item ① (five-store cascade):** seed a data subject's personal data into CAS, provenance/ledger, context store, the GitHub mirror, AND the experiment corpus (the R10 addition); trigger erasure; scan each store and assert absence. X3 owns the context-store purge proof in isolation; X7 owns the CROSS-store cascade including the corpus + mirror legs (DAG: X7⇠{D1,X3,E1}).
- **Item ② (no orphans):** after erasure, scan `EventRecord`/ledger provenance refs and assert zero dangling refs to the erased object — every ref resolves to a live target or a tamper-evident tombstone (the tombstone resolution itself is X12①/X13②/X14; X7② asserts no ORPHANS survive).
- **Item ③ (attestation re-seal/fail-closed):** after erasure, the `AttestationChain` either re-seals over the tombstone OR fails CLOSED — never a silently broken seal. This is the attestation leg X12① also relies on; X7③ owns the post-erasure re-seal-or-fail proof.
- **Item ④ (erasure × seal precedence — the R10 contradiction resolution):** D8 SEALS the experiment corpus before evaluation (D8④); a lawful erasure of a corpus datapoint is nonetheless PERMITTED despite that seal. The test erases a sealed datapoint and asserts: (a) erasure succeeds (not blocked by the seal), (b) the gate verdict INVALIDATES fail-closed — claims/regen re-pin to advisory/OFF/blocked until re-evaluation, (c) the seal is NOT silently broken (the invalidation is recorded + audited). This resolves the X7/D8⑤ structural contradiction: erasure wins, the gate fails closed, nothing is silent.
- **attestation = `AttestationChain` from hugit-contracts**; tenant boundary = HMAC-derived prefixes. Consumes D1/X3/E1/D8 surfaces as-built; modifies none.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–④ red→green · cold-verify pass by a non-author · zero writes outside claims · security review at the sprint-2 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x7/` · evidence bundle (five-store absence scans, orphan-ref scan, attestation re-seal/fail-closed proof, the erasure×seal precedence proof with the audited invalidation) attached to the sprint-2 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + per-store absence-scan refs), deviations = none | waiver-ref.
