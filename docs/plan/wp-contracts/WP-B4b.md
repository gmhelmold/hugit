# WP-B4b — GitHub integration (merge API, branch protection, force-push, kill-tests)
squad B · M · opus · route opus · budget 90k · branch: wp/B4b

## Charter
Build the GitHub API surface of the union-testing queue in `hugit-queue`: drive
ordered atomic merges through the merge API, recompute the union on force-push,
hold (never force-merge) protected / required-review PRs while honoring the
merge method, and prove crash-idempotency via a kill-test. This rides B4a's
pure engine; it is the API-surface half.

## Owned acceptance
**This half owns items ③④⑥ of B4** (GitHub integration: force-push recompute,
crash-idempotent kill-test, branch protection + merge method). The partition is
exhaustive and disjoint: B4a = ①②⑤, B4b = ③④⑥, union = B4's six items. (Item
④ spans the state machine built in B4a and its kill-test surface here, so it
lands in this later/integration half per the split rule.) VERBATIM from
decomposition v2.0 §2:

③ force-push recompute · ④ crash idempotent (kill-test) · **⑥(+)
protected/required-review PR is HELD+reported, never force-merged; merge method
honored**

## Contract deps
Consumes from `hugit-contracts` (frozen): **QueueApi** (the engine surface from
B4a), **AppWebhooks** (force-push events, merge-API write-back), **EventRecord**
(audit of holds and recomputes). Depends on B4a's frozen state machine + union
fold (build against B4a's stub) and B1's App skeleton for the GitHub
credential/Checks surface. No contract type authored or changed here.

## Claims
`crates/hugit-queue/src/github/` (merge-API driver, force-push recompute
trigger, branch-protection guard, merge-method honoring, crash-recovery
harness). Does NOT touch `crates/hugit-queue/src/core/` (B4a) or
`crates/hugit-queue/budget/` (C7).

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B4b.md`).
- `hugit-contracts` (QueueApi, AppWebhooks, EventRecord) + B4a's engine stub +
  B1's App skeleton (installation auth, Checks API).
- whitepaper §6.4 (landing — atomic ref move, batch proceeds on partial
  failure), §6.5 (event log — idempotency / "nothing is ever rewritten").
- warp-10-days §Squad B (B4 deliverable: ordered atomic merge via API).
- The failing acceptance suite at `tests/acceptance/wp-B4b/`.
- Conventions: CoreLink consumed as CLIENT; GitHub merge API via the B1 App
  installation token (write-only secret model).
Estimated packet size: ~68k tokens (inside 90k).

## Implementation notes
Every fork pre-decided:
- **Atomic merge via API (engine→GitHub):** when B4a's engine yields a green
  ordered batch, drive the GitHub merge API to land it; on success the engine
  moves to LANDED. Main stays green by construction (only green unions land).
- **Force-push recompute (③):** on a force-push webhook to a batched PR's head,
  recompute the union (re-fold via B4a) — the prior union result is invalidated;
  no stale union may land. Record the recompute as an `EventRecord`.
- **Crash idempotency (④, kill-test):** kill the worker mid-land; on restart,
  the state machine (B4a) replays from its durable state and the operation is
  idempotent — no double-merge, no lost batch, no false green. This is the
  kill-test surface for B4a's idempotent transitions; prove it end-to-end here.
- **Branch protection (⑥):** a PR under branch protection / required review is
  HELD and reported (an `EventRecord` + surfaced status) — NEVER force-merged.
  The configured merge method (merge / squash / rebase) is honored on land.
- **CoreLink consumed as CLIENT; GitHub via B1's installation token** — zero
  server changes; credentials never logged (write-only model).

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ③④⑥ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
Items ③④⑥ green; zero writes outside `crates/hugit-queue/src/github/`; evidence
bundle (force-push recompute trace, kill-test idempotency proof, branch-
protection hold + merge-method honoring proof) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, recompute trace, kill-test
log, protection-hold proof), partition note (owns ③④⑥ of B4), deviations =
none | waiver-ref.
