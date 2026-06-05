# WP-X9 — cross-phase object identity
squad X · S · sonnet · 50k · branch: wp/X9 · scheduled: **sprint 2**

## Charter
Prove object identity holds ACROSS phases: a CheckResult memoized by the phase-B
App is bit-identical to the one served as phase-D verdict evidence for the same
(tree,def,toolchain); mismatch fails CLOSED + alerts; and an intent_id minted by
a phase-B sidecar is identical and non-colliding with the native phase-D intent
— one lifecycle, one id.

## Owned acceptance
① a CheckResult memoized by the phase-B App is bit-identical to the one served as evidence in a phase-D verdict panel for the same (tree,def,toolchain)
② mismatch fails CLOSED + alerts
③(R10) intent_id identity: the id minted by a phase-B sidecar is identical and non-colliding with the native phase-D intent for the same logical intent — one lifecycle, one id; divergence/collision fails CLOSED (link-resolution alone is insufficient — X14 covers resolution, this covers identity)

## Contract deps
- `CheckResult`, `CheckDef` (frozen — item ① asserts bit-identity of the memoized vs evidence-served CheckResult for the same (tree,def,toolchain); never modified).
- `IntentSidecar` (frozen — item ③'s phase-B intent_id source).
- `VerdictObject` (frozen — the phase-D verdict panel that serves the CheckResult as evidence).
- Surfaces consumed as-built: B2 (memoization), D7 (verdict panels), B6 (sidecar), D4 (native intents). DAG: X9⇠{B2,D7}.

## Claims
- `crates/hugit-invariants/x9/` (test crate + red-team fixtures for X9 only).
- No production crate paths; consumes the B2/D7/B6/D4 surfaces, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X9 row) + §7 (DAG: X9⇠{B2,D7}) + the v1.10 note (X9③ intent_id identity across B→D) · `docs/product/command-catalog.md` (memoized checks byte-identical; intents native) · frozen types above.
- Anchors: items ①–③ each a test module under `crates/hugit-invariants/x9/`; ① compares the B-memoized CheckResult bytes to the D-served evidence bytes; ③ compares the B-sidecar intent_id to the D-native intent_id for one logical intent.
- Conventions: failing suite first; bit-identity = byte compare (not result-equal); every mismatch/divergence/collision path asserts fail-closed + alert.

## Implementation notes (every fork PRE-DECIDED)
- **Item ① (CheckResult cross-phase bit-identity):** for the same (tree,def,toolchain), assert the `CheckResult` memoized by phase-B is BYTE-identical to the one phase-D's verdict panel serves as evidence. This is the cross-phase object-identity guarantee; it consumes B2 and D7 surfaces (DAG: X9⇠{B2,D7}).
- **Item ② (mismatch fail-closed):** inject a mismatch (a CheckResult whose bytes diverge across phases) and assert the system fails CLOSED + alerts — never silently serves a divergent object as evidence.
- **Item ③ (intent_id identity — the R10 addition):** a phase-B sidecar mints an intent_id (`IntentSidecar`); the phase-D native intent (D4) for the same LOGICAL intent must carry the IDENTICAL, non-colliding id — one lifecycle, one id. Inject divergence and collision; assert both fail CLOSED. **Identity ≠ resolution:** X14 covers deep-link resolution; X9③ covers that the id is the SAME id (no second id minted, no collision), which link-resolution alone cannot prove.
- X9 is sonnet-routed because the contract makes it deterministic (byte compares + id-equality checks). Consumes B2/D7/B6/D4 as-built; modifies none.

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–③ red→green · cold-verify pass by a non-author · zero writes outside claims · security review at the sprint-2 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x9/` · evidence bundle (the cross-phase byte-compare, the mismatch fail-closed+alert proof, the intent_id identity/non-collision proof with divergence+collision fail-closed) attached to the sprint-2 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + byte-compare refs), deviations = none | waiver-ref.
