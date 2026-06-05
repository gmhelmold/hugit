# WP-B3 — affected-targets v0
squad B · M · sonnet · route sonnet · budget 70k · branch: wp/B3

## Charter
Build affected-target computation in `hugit-checks/affected`: per-package build
graphs for cargo / pnpm / turbo, golden affected-sets, full-set on root edits,
and fail-open (full set) on unknown ecosystems. This scopes which memoized
checks must run for a given change — the delta-novelty input to the landing
queue and to B2's glob sensitivity.

## Owned acceptance
B3 owns all 3 items of B3 (no split). VERBATIM from decomposition v2.0 §2:

① golden sets cargo/pnpm/turbo · ② root edit→full set · ③ unknown ecosystem→
full set fail-open

## Contract deps
Consumes from `hugit-contracts` (frozen): **CheckDef** (the check set whose
affected subset is computed) and **QueueApi** (the affected-set shape consumed
by landing). No contract type authored or changed here.

## Claims
`crates/hugit-checks/affected/` (the affected-target engine, per-ecosystem
graph adapters, fail-open policy). Does NOT touch `crates/hugit-checks/src/client/`
(B2a), `src/runner/` (B2b), or `regen/` (C4).

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B3.md`).
- `hugit-contracts` (CheckDef, QueueApi).
- whitepaper §5.1 ("affected-target computation + memoized checks: testing
  A+B+C costs only the delta novelty"), §6.2 (`affected(Δtree) = build-graph
  reachable check set`).
- warp-10-days §Squad B (B3 deliverable: cargo metadata / pnpm workspaces /
  turbo.json).
- The failing acceptance suite at `tests/acceptance/wp-B3/`.
Estimated packet size: ~46k tokens (inside 70k).

## Implementation notes
Every fork pre-decided:
- **affected(Δtree) = build-graph reachable check set** (whitepaper §6.2): given
  a tree delta, compute the reachable check targets over the package graph.
- **Per-ecosystem adapters (①):** cargo via `cargo metadata`; pnpm via
  workspace package graph; turbo via `turbo.json` task graph. Each yields a
  golden affected-set on a committed fixture.
- **Root edit → full set (②):** an edit to a root manifest (workspace
  `Cargo.toml`, `pnpm-workspace.yaml`, root `turbo.json`) invalidates the whole
  graph → full check set.
- **Unknown ecosystem → fail-OPEN (③):** if the ecosystem is unrecognized,
  return the FULL set (run everything) — never silently skip checks. Fail-open
  here means "over-run", the safe direction for an affected-set (correctness
  over speed); contrast with the fail-CLOSED security/gate paths elsewhere.
- **Output shape:** the affected-set is emitted in the `QueueApi` affected
  shape so B4a's union batching and B2a's glob sensitivity consume it directly.

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①②③ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
Items ①②③ green; zero writes outside `crates/hugit-checks/affected/`; evidence
bundle (golden sets per ecosystem, root-edit full-set proof, fail-open proof)
attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, three golden sets, fail-open
trace), deviations = none | waiver-ref.
