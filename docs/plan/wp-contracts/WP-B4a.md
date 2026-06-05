# WP-B4a — union queue core (batching, union tree, ordering, state machine)
squad B · M · opus · route opus · budget 90k · branch: wp/B4a

## Charter
Build the pure engine of the union-testing landing queue in `hugit-queue`:
batch landable PRs, fold them into a union tree, run affected memoized checks
on the union, exclude+name the minimal failing pair, land disjoint greens in
order, and prevent out-of-order landing structurally. This is the conflict
oracle's engine; B4b adds the GitHub API surface on top of it.

## Owned acceptance
**This half owns items ①②⑤ of B4** (the pure engine: union exclusion, ordered
disjoint landing, structural ordering). The partition is exhaustive and
disjoint: B4a = ①②⑤, B4b = ③④⑥, union = B4's six items. (Item ④ "crash
idempotent (kill-test)" spans the state machine and its kill-test surface and
so lands in the later/integration half B4b, per the split rule.) VERBATIM from
decomposition v2.0 §2:

① A+B-red pair excluded+named · ② 5 disjoint greens land, 0 re-runs ·
**⑤(+) lands in queue order; out-of-order structurally prevented**

## Contract deps
Consumes from `hugit-contracts` (frozen): **QueueApi** (enqueue/state/union-
result/seal + `minimal_failing_pair`), **CheckResult** (the memoized results
folded over the union), and the affected-set shape from B3. No contract type
authored or changed here.

## Claims
`crates/hugit-queue/src/core/` (batching, union-tree fold, ordering invariant,
the idempotent state machine's pure transitions) and the `core` module wiring
in `crates/hugit-queue/src/lib.rs`. Does NOT touch `crates/hugit-queue/src/github/`
(B4b) or `crates/hugit-queue/budget/` (C7).

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B4a.md`).
- `hugit-contracts` (QueueApi, CheckResult) + B3's affected-set shape.
- whitepaper §6.4 (landing / union testing at fleet scale — the algorithm:
  `U = fold(regen-rebase, head, B)`, run affected checks, green→land atomic,
  red→bisect→minimal failing pair, disjoint-claims land in parallel lanes),
  §4.1 (LANDABLE→LANDED state machine).
- warp-10-days §Squad B (B4 deliverable: union tree, ordered atomic merge,
  minimal-failing-pair report).
- The failing acceptance suite at `tests/acceptance/wp-B4a/`.
Estimated packet size: ~66k tokens (inside 90k).

## Implementation notes
Every fork pre-decided:
- **Union fold (whitepaper §6.4):** `U = fold(regen-rebase, head, batch)`; run
  affected memoized checks on `U` (mostly AC hits — only novelty executes,
  consuming B2a's client + B3's affected-set).
- **Minimal failing pair (①):** on a red union, bisect the batch over memoized
  checks (≈ free) to the minimal failing pair (e.g. A+B-red); EXCLUDE it from
  the batch and NAME both members in the `QueueApi.minimal_failing_pair`; the
  rest of the batch proceeds.
- **Disjoint greens land, 0 re-runs (②):** 5 disjoint-claims greens land in
  parallel lanes with zero check re-execution (all hits). Disjointness =
  affected-sets non-overlapping.
- **Ordering invariant (⑤):** the queue is DAG-ordered; landing applies in
  queue order and out-of-order landing is STRUCTURALLY prevented (the state
  machine has no transition that lands an entry ahead of an unlanded
  predecessor) — not a runtime check that could be bypassed, an absent edge.
- **State machine:** model `LANDABLE → (union-test) → LANDED | UNION-FAIL`
  (whitepaper §4.1) as a pure, deterministic transition function in
  `src/core/`. The idempotency PROOF under crash (kill-test) is B4b's item ④;
  B4a builds the pure transitions, B4b proves crash-idempotency end-to-end.
- **No GitHub API here** — the merge API, branch protection, and force-push
  recompute are B4b. B4a is engine-pure and testable in isolation.

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①②⑤ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
Items ①②⑤ green; zero writes outside `crates/hugit-queue/src/core/`; evidence
bundle (minimal-failing-pair trace, 5-disjoint-greens 0-rerun proof,
out-of-order structural-prevention proof) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, union-fold trace,
ordering-invariant proof), partition note (owns ①②⑤ of B4), deviations =
none | waiver-ref.
