# WP-X5 — namespace laws
squad X · S · sonnet · 40k · branch: wp/X5 · scheduled: **sprint 2**

## Charter
Mechanize the two namespace laws: no hugit CLI verb shadows a git verb
(checked against `git help -a`), and managed refs (`refs/hugit/…`) never
collide with arbitrary user branches/tags (property test). The "git is never
shadowed" guarantee, made standing and falsifiable.

## Owned acceptance
① no hugit CLI verb shadows a git verb (mechanized check against `git help -a`)
② managed refs (`refs/hugit/…`) never collide with arbitrary user branches/tags (property test)

## Contract deps
- The `hugit` CLI verb surface (hugit-cli) — consumed read-only; X5 reads the verb table, never adds verbs.
- The managed-ref namespace (`refs/hugit/…`) from D1/D4 refstore — consumed as-built.
- No frozen `hugit-contracts` type is modified here.

## Claims
- `crates/hugit-invariants/x5/` (test crate + red-team fixtures for X5 only).
- No production crate paths; consumes the CLI verb table + ref namespace, never modifies them.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §6 (X5 row) · `docs/product/command-catalog.md` ("Unchanged forever: every git command byte-for-byte" + the three namespace laws) · `docs/whitepaper/hugit-v1.md` §9 (degradation invariant context) · the hugit CLI verb table.
- Anchors: item ① a mechanized test diffing the hugit verb set against `git help -a`; item ② a property test over arbitrary user branch/tag names asserting no collision with `refs/hugit/…`.
- Conventions: failing suite first; item ① runs `git help -a` as the oracle (not a hand-maintained list); item ② is a property test with a generated namespace corpus.

## Implementation notes (every fork PRE-DECIDED)
- **Item ① (no shadowing):** the test enumerates hugit's CLI verbs and asserts the intersection with `git help -a`'s verb set is empty. The git verb list is generated from `git help -a` at test time — never a hand-copied list (which would rot). A new shadowing verb anywhere in hugit-cli turns this red.
- **Item ② (ref collision):** a property test generates arbitrary user branch/tag names and asserts none can collide with the `refs/hugit/…` managed namespace, AND that managed-ref creation never lands a ref in user space. The `refs/hugit/…` prefix is the invariant; the test treats it as reserved.
- X5 consumes the CLI verb table and the refstore namespace as-built; it modifies neither. It adds NO verbs and reserves NO new prefixes — it only proves the existing laws hold.
- This is the namespace-law standing fixture; it is sonnet-routed because the contract makes it a deterministic build (no adversarial judgment, just the two mechanized checks).

## DoD
Global bar: fmt + clippy + test + audit green · owned items ①–② red→green · cold-verify pass by a non-author · zero writes outside claims · security review at the sprint-2 SEAL.

## Completeness
All owned items green · zero writes outside `crates/hugit-invariants/x5/` · evidence bundle (the `git help -a` shadow-check output, the ref-collision property-test report) attached to the sprint-2 SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (test module paths + shadow-check + property-test refs), deviations = none | waiver-ref.
