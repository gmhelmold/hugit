# WP-X4 — supply chain
squad X · M · opus *(red-team)* · 60k · branch: wp/X4 · scheduled: **sprint 1**

## Charter
Prove the supply chain is pinned and verified end-to-end: runner images are
content-pinned and integrity-verified at spawn, App dependencies are pinned and
verified in CI, and any tampered/unpinned image fails CLOSED before any tenant
work runs. A sprint-1 invariant — it gates the fabric that sprint-2 stands on.

## Owned acceptance
① runner images content-pinned + integrity-verified at spawn
② App dependencies pinned + verified in CI
③ tampered/unpinned image → fail CLOSED before any tenant work

## Contract deps
- `RunnerLease` (frozen — the spawn boundary at which image integrity is verified; consumed read-only).
- The runner-spawn surface (C2) and the App/CI dependency surface (B1 + workspace scaffold) — consumed as-built.
- Tenant boundary = HMAC-derived prefixes (CoreLink model) — relevant only insofar as "before any tenant work" is the fail-closed precondition.

## Claims
- `crates/hugit-invariants/x4/` (test crate + red-team fixtures for X4 only).
- No production crate paths; consumes the spawn + CI surfaces, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X4 row) + §7 (DAG: X8⇠X4; C5⇠X4) · `docs/whitepaper/hugit-v1.md` §9 (the five locks; fail-closed) · `docs/product/command-catalog.md` (ephemeral cache-warm runners) · frozen `RunnerLease`.
- Anchors: items ①–③ each a test module under `crates/hugit-invariants/x4/`; ③ is the tamper/unpinned attack asserting fail-closed BEFORE tenant work.
- Conventions: failing suite first; "content-pinned" = digest-pinned (not tag); every verification asserts the check happens at spawn and BEFORE any tenant byte is processed.

## Implementation notes (every fork PRE-DECIDED)
- **Item ① (image pinning):** runner images are pinned by content digest and integrity-verified at spawn against `RunnerLease`; the test asserts a digest mismatch is detectable. Consumes C2's spawn surface, never modifies it.
- **Item ② (App deps):** App/workspace dependencies are pinned (lockfile/digest) and verified in CI; the test asserts an unpinned/floating dep is rejected by the CI gate. This rides the existing audit gate, not a new one.
- **Item ③ (fail CLOSED, ordered):** present a tampered AND an unpinned image; assert spawn fails CLOSED **before any tenant work** — no lease enters the working state, no tenant byte touched. The ordering assertion (verify-before-work) is the load-bearing part; a post-hoc detection is a FAIL.
- X4 is the provenance floor that X8 (self-release attestation) builds on (DAG: X8⇠X4) and that C5 consumes (DAG: C5⇠X4); X4 modifies neither.
- **Dedicated red-team pass required** (§8): a non-author red-team agent attempts to slip a tampered/unpinned image past spawn beyond the committed scripts; the SEAL records the attempt log.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–③ red→green · cold-verify pass by a non-author · **plus the dedicated red-team pass** (X4 is a named red-team WP, §8) · zero writes outside claims · security review at the sprint-1 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x4/` · evidence bundle (digest-pin verification, CI dep-pin assertion, the ordered fail-closed proof, red-team attempt log) attached to the sprint-1 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + red-team log ref), deviations = none | waiver-ref.
