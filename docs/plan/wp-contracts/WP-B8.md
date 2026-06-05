# WP-B8 — dogfood harness
squad B · M · sonnet · route sonnet · budget 70k · branch: wp/B8

## Charter
Build the dogfood harness in `tests/dogfood`: run a real 5-PR agent-fleet wave
end-to-end through the Phase-B App, measure against a defined baseline
(memoization off) in a versioned report with formulas, and prove a 48h soak
with zero wrong-merge / lost-PR (event-audited). Targets are `hugit` +
`corelink-workspaces` + 2 synthetic fleet repos — NEVER `corelink-server`
(non-interference / focus gate).

## Owned acceptance
B8 owns all 3 items of B8 (no split). VERBATIM from decomposition v2.0 §2:

① real 5-PR wave e2e · ② **🔧 vs defined baseline (same wave, memoization off),
versioned report with formulas** · ③ 48h soak: 0 wrong-merge/lost-PR (event-
audited)

## Contract deps
Consumes from `hugit-contracts` (frozen): **CheckResult**, **QueueApi**,
**EventRecord** (the audit trail proving 0 wrong-merge/lost-PR). Integrates the
full Phase-B stack (B1–B7) as a CLIENT — B8 is the cross-claim integration WP
and runs at the barrier (decomposition §7: "cross-claim writers remain B8 +
final SEAL, at barriers"). No contract type authored or changed here.

## Claims
`tests/dogfood/` (the harness, fixtures for the 4 target repos, the baseline
runner, the versioned report generator, the 48h soak driver). Does NOT write
into any `crates/` source (it consumes them); does NOT enroll `corelink-server`.

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B8.md`).
- `hugit-contracts` (CheckResult, QueueApi, EventRecord) + the B1–B7 stack as
  built (consumed, not modified).
- whitepaper §5.2 (baseline = GitHub-style re-run-everything vs memoized),
  §6.4 (the union-queue wave it drives end-to-end).
- warp-10-days §Squad B (B8 deliverable + the D5 mid-SEAL exit criterion: "a
  real agent-fleet PR wave on `hugit` lands through the union queue with
  memoized checks and a regenerated lockfile"; targets EXCLUDE corelink-server).
- CLAUDE.md + command-catalog focus gate (zero corelink-server enrollment).
- The failing acceptance suite at `tests/acceptance/wp-B8/`.
Estimated packet size: ~50k tokens (inside 70k).

## Implementation notes
Every fork pre-decided:
- **Targets (fixed):** `hugit`, `corelink-workspaces`, and 2 synthetic fleet
  repos. `corelink-server` is STRUCTURALLY excluded — enrollment of it must
  FAIL the harness (the focus gate, X10②; non-interference governance law §8).
- **5-PR wave e2e (①):** drive a real 5-PR agent-fleet wave through the full
  Phase-B path (App ingest → memoized checks → affected-set → union queue →
  diagnosis → surface), landing through the union queue.
- **Baseline report (②):** run the SAME wave with memoization OFF (every check
  re-executes) as the defined baseline; emit a VERSIONED report with the
  explicit formulas (minutes saved = baseline_exec_minutes − memoized_exec_
  minutes; the report states the cost model version from B7). Reproducible.
- **48h soak (③):** run continuously for 48h; assert ZERO wrong-merge and ZERO
  lost-PR, PROVEN via the `EventRecord` audit log (every land/hold/exclude is an
  event; the audit reconstructs that no PR was merged wrongly or dropped).
- **Integration WP:** B8 consumes B1–B7; it does not modify them. It runs at the
  D5 mid-SEAL barrier.
- **CoreLink consumed as CLIENT only** — zero server changes.

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①②③ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
Items ①②③ green; zero writes outside `tests/dogfood/`; corelink-server
provably excluded; evidence bundle (5-PR wave trace, versioned baseline report
+ formulas, 48h soak event-audit) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (wave e2e log, baseline report ref, 48h
soak audit, corelink-server exclusion proof), deviations = none | waiver-ref.
