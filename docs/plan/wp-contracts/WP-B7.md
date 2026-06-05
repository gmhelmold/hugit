# WP-B7 — surface v0
squad B · S · sonnet · route sonnet · budget 50k · branch: wp/B7

## Charter
Build the Phase-B human surface in `hugit-app/ui`: a live status page and
exactly-one edited PR comment, where every saved-minutes number links to its
CheckResult set (auditable) and the "$ saved" figure derives from minutes via a
versioned, auditable cost model. No new human CLI in Phase B — the App
dashboard + PR comment ARE the surface.

## Owned acceptance
B7 owns all 4 items of B7 (no split). VERBATIM from decomposition v2.0 §2:

① live status page · ② exactly one edited comment/PR · ③ **🔧 every saved-
minutes number links to its CheckResult set (auditable)** · **④(R3) the "$
saved" figure derived from minutes via a versioned, auditable cost model (rates
stated), reconcilable against the minutes count**

## Contract deps
Consumes from `hugit-contracts` (frozen): **CheckResult** (the memoized results
the saved-minutes link to), **AppWebhooks** (the single editable PR comment
write-back). Depends on B1's App skeleton. No contract type authored or changed
here.

## Claims
`crates/hugit-app/ui/` (status page, the single-comment renderer/upserter, the
saved-minutes→CheckResult linker, the versioned cost model). Does NOT touch the
rest of `crates/hugit-app/` (B1) or `crates/hugit-app/sidecar/` (B6).

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B7.md`).
- `hugit-contracts` (CheckResult, AppWebhooks) + B1's App skeleton.
- whitepaper §5.2 (the economic physics — minutes/$ saved is the one number),
  §8 (the human experience — file tree unchanged, minimal surface).
- command-catalog (phase-B human surface = GitHub App dashboard + PR comments,
  NO new CLI; "one number: CI minutes/$ saved").
- warp-10-days §Squad B (B7 deliverable; depends B1).
- The failing acceptance suite at `tests/acceptance/wp-B7/`.
Estimated packet size: ~34k tokens (inside 50k).

## Implementation notes
Every fork pre-decided:
- **Live status page (①):** a status surface served by the App reflecting
  current install/PR check state in real time.
- **Exactly one comment/PR (②):** the App maintains a SINGLE PR comment and
  edits it in place (upsert by a stable marker), never posting a second — assert
  comment count == 1 per PR.
- **Saved-minutes audit link (③):** every saved-minutes figure renders with a
  deep link to the exact CheckResult set it was computed from (the AC hits that
  avoided execution) — auditable, not a bare number.
- **Cost model (④):** the "$ saved" figure = `minutes × rate` via a VERSIONED
  cost model whose rates are stated in the artifact; the figure is reconcilable
  against the minutes count (same minutes × stated rate = stated $). The model
  version is recorded so a past figure is reproducible. NO unversioned or
  hidden-rate dollar claims.
- **No new human CLI** — the surface is the App dashboard + the one PR comment
  (command-catalog: phase-B human surface).
- **CoreLink consumed as CLIENT only** — zero server changes.

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①–④ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
All 4 owned items green; zero writes outside `crates/hugit-app/ui/`; evidence
bundle (status-page render, single-comment assert, saved-minutes→CheckResult
link, cost-model version + reconciliation) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, single-comment proof, audit-
link sample, cost-model reconciliation), deviations = none | waiver-ref.
