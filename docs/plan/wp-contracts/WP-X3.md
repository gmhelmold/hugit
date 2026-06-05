# WP-X3 — context privacy
squad X · M · opus · 70k · branch: wp/X3 · scheduled: **sprint 2**

## Charter
Prove context and journals are tenant-scoped (no cross-tenant fetch), redacted
at BOTH capture and export, purged on retention/deletion (verified absent), and
excluded from training/eval with a documented control + audit trail. This is
the privacy invariant behind the context/journal object classes.

## Owned acceptance
① context/journals tenant-scoped (cross-tenant fetch denied)
② redaction at capture AND export
③ retention/deletion purges (verified absent)
④ training/eval exclusion: documented control + audit trail

## Contract deps
- `ExportSchema` (versioned, machine-validatable — frozen; item ②'s export-side redaction is asserted against it).
- Context/journal object surfaces from D11 (journals+resume) and the export path (E5) — consumed read-only as-built.
- Tenant boundary = HMAC-derived prefixes (CoreLink model) — the scope item ① attacks.

## Claims
- `crates/hugit-invariants/x3/` (test crate + red-team fixtures for X3 only).
- No production crate paths; consumes the context/journal + export surfaces, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X3 row) · `docs/whitepaper/hugit-v1.md` §9 (context snapshots tenant-private, policy-redactable, retention-bound, never training data) + §13.2 (context capture sensitivity) · `docs/product/command-catalog.md` (Journals + short-horizon resume) · frozen `ExportSchema`.
- Anchors: items ①–④ each a test module under `crates/hugit-invariants/x3/`; ① is the cross-tenant fetch-deny attack, ③ seeds a datum then asserts post-purge absence, ④ executes the documented exclusion control.
- Conventions: failing suite first; every redaction/purge assertion verifies ABSENCE (grep-class scan of the stored/exported bytes), not mere flag state.

## Implementation notes (every fork PRE-DECIDED)
- **Item ① (tenant scope):** tenant B fetches tenant A's context/journal object; assert deny + audit event. Scope is the HMAC-prefix boundary; X3 consumes D11's surface, never re-implements scoping.
- **Item ② (redaction at capture AND export):** seed a secret into a captured context; assert it is redacted in the at-rest captured object (capture-time) AND in the exported artifact validated against `ExportSchema` (export-time). The export-side assertion is the same redaction E5④/E5⑦ require — X3 owns the capture+export privacy proof; the exit-proof side stays E5.
- **Item ③ (retention/deletion purge):** seed → trigger retention/deletion → re-scan the store and assert the datum is absent (not tombstoned-with-bytes; actually purged). This is the context-store leg of the X7 erasure cascade; X7 owns the cross-store cascade, X3 owns the context-store purge proof.
- **Item ④ (training/eval exclusion):** a DOCUMENTED control (committed under the test crate) marks context/journals never-training-data; the test executes the control and asserts an audit trail entry for any access classified non-training. No model is invoked — the control + audit trail is the deliverable.
- Consumes D11/E5 surfaces as-built; modifies neither.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–④ red→green · cold-verify pass by a non-author · zero writes outside claims · security review at the sprint-2 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x3/` · evidence bundle (cross-tenant deny + audit assertion, capture+export redaction absence scans, post-purge absence scan, the training-exclusion control doc + audit-trail assertion) attached to the sprint-2 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + exclusion-control doc ref), deviations = none | waiver-ref.
