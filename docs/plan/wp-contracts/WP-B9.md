# WP-B9 — exit telemetry + the money gate
squad B · S · sonnet · route sonnet · budget 50k · branch: wp/B9

## Charter
Build exit telemetry + the money gate in `hugit-app` (exit-metric module): per-
install activity → week-3 retention vs ≥40%, unprompted-vs-prompted feedback
capture with the ≥3-unprompted gate enforced as pass/fail, an auditable
generated exit report with cohort/window guards, and THE MONEY GATE — billing
is structurally blocked until the report = PASS (a control, not a dashboard;
symmetric to D8⑥). Depends on B8's report.

## Owned acceptance
B9 owns all 6 items of B9 (no split). VERBATIM from decomposition v2.0 §2:

① per-install activity → week-3 retention computable vs the ≥40% threshold
(privacy-documented) · ② **🔧 feedback capture distinguishes UNPROMPTED ("I'd
pay") statements from prompted responses — only unprompted count toward the ≥3
gate** · ③ exit-metric report generated from data, auditable · **④ 🔧
cohort/window guards: n=10 external teams, ≥3 weeks real use, evaluation window
ANCHORED to the first-10-paying-customers event and inside 90 days — otherwise
"insufficient/out-of-window", never a pass** · **⑤(R4) the ≥3 gate is ENFORCED
as pass/fail: 2 correctly-counted unprompted signals → FAIL even with ≥40%
retention; exactly 3 → PASS** · **⑥ 🔧 THE MONEY GATE BINDS: charging money for
hugit is structurally blocked until the exit report = PASS; a FAIL/insufficient/
out-of-window report CANNOT enable billing; a DEGRADED gate-evaluator state =
"insufficient" → fails CLOSED, cannot enable; the enable-billing event is itself
audited (control, not dashboard — symmetric to D8⑥)**

## Contract deps
Consumes from `hugit-contracts` (frozen): **EventRecord** (the audited enable-
billing event + activity events), **ExportSchema**-adjacent report typing for
the auditable exit report. Depends on B8's versioned report (the data source).
No contract type authored or changed here.

## Claims
`crates/hugit-app/exit/` (the exit-telemetry collector, retention computer,
unprompted/prompted classifier, report generator, the money-gate enforcement
point). Disjoint from B1/B6/B7 module paths within `hugit-app`.

## Dispatch packet
- This contract file (`docs/plan/wp-contracts/WP-B9.md`).
- `hugit-contracts` (EventRecord) + B8's versioned report.
- warp-10-days §front-matter (the phase-B exit metric: 10 teams, 3 weeks,
  ≥40% retention, ≥3 unprompted "I'd pay"; gates charging money, not building;
  clock anchored to first-10-paying-customers).
- command-catalog (the phase-B exit metric, binding form).
- decomposition v1.8 note (B9⑥ THE MONEY GATE BINDS — symmetric to D8⑥
  "control, not dashboard").
- The failing acceptance suite at `tests/acceptance/wp-B9/`.
Estimated packet size: ~36k tokens (inside 50k).

## Implementation notes
Every fork pre-decided:
- **Retention (①):** per-install activity events → week-3 retention rate,
  comparable against the ≥40% threshold; the privacy model (what is collected,
  how anonymized) is documented in the artifact.
- **Unprompted classifier (②/⑤):** feedback capture tags each "I'd pay"
  statement as UNPROMPTED vs PROMPTED; ONLY unprompted count toward the gate.
  The ≥3 gate is hard pass/fail: 2 correctly-counted unprompted → FAIL even at
  ≥40% retention; exactly 3 → PASS. Both must hold.
- **Generated report (③):** the exit report is generated FROM data (never
  hand-written) and is auditable (traces to the underlying events).
- **Cohort/window guards (④):** require n=10 external teams, ≥3 weeks real use,
  evaluation window ANCHORED to the first-10-paying-customers event and inside
  90 days; outside any guard → status "insufficient/out-of-window", NEVER a
  pass.
- **THE MONEY GATE (⑥):** billing is STRUCTURALLY blocked until the report =
  PASS. A FAIL / insufficient / out-of-window report CANNOT enable billing; a
  DEGRADED gate-evaluator state = "insufficient" → fails CLOSED (cannot
  enable). The enable-billing transition is itself an audited `EventRecord`.
  This is a control gating billing, not a dashboard — symmetric to D8⑥.
- **CoreLink consumed as CLIENT only** — zero server changes; billing path is
  Stripe (the audited path), gated here.

## DoD
Global bar: `cargo fmt` + `clippy -D warnings` + `cargo test` + `cargo audit`
green · owned items ①–⑥ red→green via the suite · cold verification by a
non-author agent · zero writes outside Claims.

## Completeness
All 6 owned items green; zero writes outside `crates/hugit-app/exit/`; evidence
bundle (retention computation, unprompted-classifier + ≥3 pass/fail cases,
generated report, cohort/window guard cases, money-gate block + audited enable
event) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (suite log, ≥3-gate pass/fail proofs,
guard cases, money-gate fail-closed + audit proof), deviations = none |
waiver-ref.
