# WP-D10 — why + impact
squad D · M · sonnet · ctx 70k · branch: wp/D10

## Charter
Build the provenance + blast-radius query verbs: `hugit why <line|symbol>` →
originating intent + charter/author/model/cost (matching the event log), and
`hugit impact <path|change>` → the golden affected-set over the build graph.
`impact` feeds verdict-panel ground truth (the cross-check). `why` on
regenerated/derived bytes resolves HONESTLY to the regen event — never fabricates
a human author.

## Owned acceptance (VERBATIM — decomposition v2.0 D10①–④)
① `hugit why <line|symbol>` → originating intent + charter/author/model/cost, matching event log
② `hugit impact <path|change>` → golden affected-set on known build graph
③ impact feeds verdict-panel ground truth (cross-check)
④ **(R6) `why` on regenerated/derived bytes resolves honestly to the regen/derivation event — never fabricates or mis-attributes a human author**

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `EventRecord` — `why` resolves against the event log; answers MUST match it (①).
- `AttestationChain {tree, def, runner, model, principal, sig}` — supplies the charter/author/model/cost provenance `why` reports (①④).
- `IntentSidecar` / native intent id — the originating intent `why` resolves to.
- Build-graph / affected-target representation (B3 heritage) — the input to `impact` (②); consumed read-only.

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-cli/why/` (the `why` provenance resolver, incl. the derived-bytes
  honest-resolution path).
- `crates/hugit-cli/impact/` (the `impact` blast-radius query + ground-truth
  export for verdict panels).
- `crates/hugit-cli/why/tests/`, `crates/hugit-cli/impact/tests/`.
Writes outside these two modules = leak. (D7 owns `verdict/`, D9 owns
`attention/` — D10 EXPORTS impact ground truth to both; disjoint modules.)

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D10 row) + §7 (D7→D9
  uses D10 blast-radius; D10 feeds D7 ground truth) + §8; `docs/whitepaper/
  hugit-v1.md` §8 item 5 (deep links to the byte) + §9 (provenance: model/principal
  chain) + §4 (object model); `docs/product/command-catalog.md` (`hugit why` +
  build-graph impact rows); frozen `EventRecord`/`AttestationChain`/`IntentSidecar`.
- Anchors: `impact` runs on a KNOWN build graph with a GOLDEN affected-set; `why`
  is asserted to MATCH the event log. Fixtures include a regenerated/derived file.
- Conventions: house stack (Rust); fmt+clippy+test+audit; failing acceptance suite
  committed BEFORE implementation; SEAL.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **`why` matches the event log (①):** `why <line|symbol>` resolves to the
  originating intent and reports charter/author/model/cost drawn from the
  `AttestationChain` — and the answer is asserted to MATCH the event log (no
  divergent second source).
- **`impact` is golden (②):** `impact <path|change>` returns the GOLDEN affected
  set on a KNOWN build graph — assert set-equality against the golden, not merely
  non-empty/non-error.
- **impact → verdict ground truth (③):** `impact` EXPORTS the affected-set as the
  served ground truth consumed by D7 verdict panels (the cross-check). D10 owns the
  export module; D7 consumes it — the seam is one-directional.
- **Derived-bytes honesty (④):** `why` on regenerated/derived bytes resolves to
  the REGEN/DERIVATION event — it NEVER fabricates or mis-attributes a human
  author. Drive a derived-file fixture and assert the answer names the regen event,
  not a human. (Pairs with D12 regen provenance.)

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All four owned items green · zero writes outside `crates/hugit-cli/why/` +
`crates/hugit-cli/impact/` · evidence bundle (`why`↔event-log match, golden
affected-set equality, impact→verdict ground-truth handoff, derived-bytes honest
resolution) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–④: green/red) · evidence refs (test ids + fixture paths +
golden-set diff + derived-bytes transcript) · claims-respected: yes · deviations:
none | waiver-ref.
