# WP-D5 — ledger + watch + fleet
squad D · M · sonnet · ctx 70k · branch: wp/D5

## Charter
Build the human-facing read surface over the forge event stream: `hugit ledger`
(asked→done→proven by campaign), `hugit watch` (live TUI), and `hugit fleet`
(machine-readable fleet state). All three are pure projections of the
EventRecord stream — one store, two zooms (intent ⇄ raw-commit), redaction at
the view boundary. Read-only; never a source of truth.

## Owned acceptance (VERBATIM — decomposition v2.0 D5①–⑥)
① asked→done→proven per campaign
② **🔧 watch: EventRecord-to-display p95 <2s (measured per event class)**
③ **🔧 deep-links resolve to golden expected targets (not just non-error)**
④ **(+) planted secret renders REDACTED in ledger/verdict views**
⑤ **(R2) two-zoom toggle: intent view ⇄ raw-commit view mutually consistent over the same fixture (one store)**
⑥ **(R2) `hugit fleet` emits documented machine-readable schema reflecting true ws/agent state vs fixture**

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `EventRecord` — the append-only stream this WP projects (the ONLY input).
- `VerdictObject` — surfaced in ledger/verdict views (redaction applies, ④).
- `IntentSidecar` / native intent id — the intent altitude of the two-zoom toggle (⑤).
- Redaction policy descriptor (capture-time + view-time) — consumed read-only for ④.

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-ledger/` EXCEPT `journal/` (D11's claim): ledger projection,
  `watch` TUI, `fleet` schema emitter, deep-link resolver, view-side redaction
  filter.
- `crates/hugit-ledger/tests/` (its acceptance + golden fixtures).
Writes anywhere else (refstore, contracts, policy, cli/verdict, AND
`hugit-ledger/journal/`) = leak.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D5 row) + §8;
  `docs/whitepaper/hugit-v1.md` §8 (human experience — ledger/watch/attention)
  + §4 (projection rule, two zooms); `docs/product/command-catalog.md`
  (`hugit ledger`/`watch`/`fleet` rows); the frozen `EventRecord`/`VerdictObject`
  schemas from `crates/hugit-contracts/`.
- Anchors: project ONLY the EventRecord stream; D1 owns the stream, D5 owns the
  read views. Fixtures are deterministic event logs + golden expected targets.
- Conventions: workspace house stack (Rust); fmt+clippy+test+audit; failing
  acceptance suite committed BEFORE implementation; SEAL with evidence bundle.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **Read-only law:** ledger/watch/fleet CONSUME the EventRecord stream and emit
  nothing back into it. No write path exists from this crate to refstore/contracts.
- **Two zooms, one store (⑤):** the intent view and the raw-commit view are BOTH
  derived from the same event log (one is a projection of the other per the §4
  projection rule); the toggle re-projects, it never reads a second store —
  mutual consistency is structural, asserted on a shared fixture.
- **Watch latency (②):** measure EventRecord-arrival → on-screen render p95
  PER event class (landing / verdict / policy-change / ws-state), report the
  table; <2s is the bar, stated per class, not an aggregate.
- **Deep links (③):** resolution is GOLDEN — each link is asserted to reach a
  specific expected target object, not merely a non-error response.
- **Redaction (④):** redaction is applied at the VIEW boundary (in addition to
  capture-time); a planted secret in the underlying objects renders as REDACTED
  in every ledger/verdict view — assert on the rendered output bytes.
- **`fleet` schema (⑥):** documented, versioned, machine-readable; it reflects
  TRUE ws/agent state derived from events vs a known fixture — schema-validate
  the emission and diff against the fixture's true state.

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All six owned items green · zero writes outside `crates/hugit-ledger/` · evidence
bundle (latency table per event class, golden deep-link diffs, redaction render
assertion, two-zoom consistency proof, `fleet` schema validation) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–⑥: green/red) · evidence refs (test ids + fixture paths +
latency table) · claims-respected: yes · deviations: none | waiver-ref.
