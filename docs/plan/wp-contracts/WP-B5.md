# WP-B5 — bisect + diagnosis
squad B · M · sonnet · route sonnet · budget 70k · branch: wp/B5

## Charter
Build auto-bisect over memoized checks + the DiagnosisObject in `hugit-diag`:
find the culprit in ≤log₂ executions, return diff-vs-green + suspect targets as
bounded schema data (never a raw log dump), within a 2-minute fixture budget,
triggered automatically on any red. Memoization makes bisect ≈ free, so it runs
always.

## Owned acceptance
B5 owns all 5 items of B5 (no split). VERBATIM from decomposition v2.0 §2:

① culprit ≤log₂ execs · ② diff-vs-green + suspects · ③ <2min fixture ·
**④(+) diagnosis is bounded schema data, never raw log dump (size assert)** ·
**⑤(R2) bisect triggers automatically on any red — no manual invocation**

## Contract deps
Consumes from `hugit-contracts` (frozen): **DiagnosisObject** (the bounded
output schema with `size_bytes` assertion), **CheckResult** (the memoized
checks bisected over), **QueueApi** (the red signal from union landing that
triggers bisect). No contract type authored or changed here.

## Claims
`crates/hugit-diag/src/bisect/` and `crates/hugit-diag/src/diagnosis/` (the
bisect engine, culprit finder, diff-vs-green + suspect-target assembler, the
auto-trigger). Does NOT touch `crates/hugit-diag/flake/` (C6) or
`crates/hugit-diag/experiment/` (D8).

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B5.md`).
- `hugit-contracts` (DiagnosisObject, CheckResult, QueueApi).
- whitepaper §5.1 ("bisect over memoized checks ≈ free; culprit-finding becomes
  a default"), §6.4 (`red → bisect batch over memoized checks → minimal failing
  pair → structured UNION-FAIL`).
- command-catalog (structured diagnoses: culprit + diff-vs-last-green + suspect
  targets — data, not 4,000-line logs; auto-culprit on regression).
- warp-10-days §Squad B (B5 deliverable).
- The failing acceptance suite at `tests/acceptance/wp-B5/`.
Estimated packet size: ~48k tokens (inside 70k).

## Implementation notes
Every fork pre-decided:
- **Bisect over memoized checks (①):** binary search over the batch/history
  using AC-memoized check results — each probe is an AC lookup (≈ free), so
  the culprit is found in ≤ ⌈log₂ n⌉ executions.
- **Diagnosis assembly (②):** emit a `DiagnosisObject` carrying `culprit_ref`,
  `diff_vs_green_ref`, and `suspect_targets[]` (the build-graph targets reachable
  from the culprit's change).
- **Bounded schema, size assert (④):** the `DiagnosisObject` is bounded
  structured data — it carries CAS refs to logs (`stdout_ref`/`stderr_ref` on
  CheckResult), NEVER inlined raw logs. The suite asserts `size_bytes ≤` the
  documented bound; a raw-log-dump diagnosis FAILS the assertion.
- **2-minute fixture (③):** the end-to-end bisect+diagnose on the committed
  fixture completes in <2min (mostly AC hits).
- **Auto-trigger (⑤):** bisect fires automatically on ANY red signal from the
  landing queue (`QueueApi` UNION-FAIL / red) — there is NO manual-invocation
  path; the auto-trigger is the only entry point (assert the manual path is
  absent).

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①–⑤ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
All 5 owned items green; zero writes outside the bisect/diagnosis module paths;
evidence bundle (≤log₂ execution count, DiagnosisObject sample + size assert,
<2min timing, auto-trigger proof) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, bisect-depth proof, bounded-
diagnosis size assert, auto-trigger trace), deviations = none | waiver-ref.
