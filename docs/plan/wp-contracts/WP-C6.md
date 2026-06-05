# WP-C6 — flake-stats collector
squad C · S · sonnet · 50k · branch: wp/C6

## Charter
Statistical flake detection: every check execution feeds per-test statistics; a
planted 20% flake is detected in <30 runs; the quarantine list is a policy
artifact only — no auto-act (any reorder/skip/block/gate is prohibited in v0,
annotation-only allowed); and a deterministically-failing (non-flaky) test is
classified a REAL failure, never quarantined. Depends on B2 executions.

## Owned acceptance
① every execution feeds stats · ② planted 20% flake detected <30 runs · ③ **🔧 quarantine list = policy artifact; "auto-act" defined: any reorder/skip/block/annotation-that-gates = prohibited in v0 (annotation-only allowed)** · **④(R3) false-positive guard: a deterministically-failing (non-flaky) test is classified as REAL failure and NEVER quarantined within the same volume window**

## Contract deps
- `CheckResult` (frozen — every execution's result feeds the stats; consumed,
  never modified).
- The B2 check-execution stream (consumed as the feed; B2 owns the executor,
  C6 owns the statistics over its outputs).

## Claims
- `crates/hugit-diag/flake/` — the flake-stats collector + detector + the
  policy-artifact quarantine list (annotation-only). Disjoint from the rest of
  `hugit-diag` (B5 bisect) and `hugit-diag/experiment` (D8).

## Dispatch packet
- Files received: this contract · decomposition §3 (C6 row) · warp-10-days
  Squad C (C6: "every check execution feeds per-test statistics … quarantine
  policy consumes later") · command-catalog Phase C ("Flake intelligence:
  statistical flake detection + quarantine by policy") · `hugit-contracts`
  (`CheckResult`).
- Anchors: per-test running statistics; the 20%-flake detection threshold
  (<30 runs); the v0 auto-act prohibition (annotation-only).
- Conventions: failing acceptance suite committed first; planted-flake fixture +
  deterministic-failure fixture.

## Implementation notes (every fork PRE-DECIDED)
- **Stats source:** every `CheckResult` from the B2 executor feeds per-test
  statistics (the fabric's execution volume is the point — hence phase C).
- **Detection:** a planted 20% flake is detected within <30 runs of that test;
  the detector emits to the quarantine list.
- **v0 auto-act prohibition (③):** the quarantine list is a POLICY ARTIFACT.
  Any reorder, skip, block, or annotation-that-gates is prohibited in v0;
  annotation-only (non-gating) is the sole permitted surface. C6 must not wire
  itself into any gate.
- **False-positive guard (④):** a deterministically-failing (non-flaky) test is
  classified a REAL failure and is NEVER quarantined within the same volume
  window — the detector distinguishes intermittent from deterministic.
- Stats may be backed by the D1 event stream where available, but C6's claim is
  the collector/detector, not the store.

## DoD
Global: fmt + clippy + test + audit green · owned items red→green · cold-verify
pass by a non-author. Zero writes outside claims.

## Completeness
All owned items green · zero writes outside `crates/hugit-diag/flake/` ·
evidence bundle (stats-feed proof, 20%-flake-in-<30-runs detection, auto-act
prohibition assertion, deterministic-failure-not-quarantined proof) attached
to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs, deviations = none | waiver-ref.
