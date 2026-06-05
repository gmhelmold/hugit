# WP-D8 — experiment harness + gate binding
squad D · M · opus · ctx 70k · branch: wp/D8

## Charter
Build the experiment harness that auto-collects claim-disjointness + regen-honesty
datapoints from the fleet's real waves, and BINDS the experiment gate: claims-as-
oracle stays advisory/OFF and regen promotion stays blocked until the report shows
PASS. The gate is a fail-CLOSED CONTROL, not a dashboard — a degraded evaluator =
"insufficient" = cannot promote. The corpus is pre-sealed, focus-gate-eligible at
ingestion, and the gate report is itself an attested tamper-evident object.

## Owned acceptance (VERBATIM — decomposition v2.0 D8①–⑨)
① every wave auto-contributes datapoints
② dashboard: disjointness %, regen agree/disagree, n
③ gate report generated, never hand-written
④ **(R2) anti-gaming: promotion corpus pre-registered and SEALED before evaluation; sample selection auditable**
⑤ **(R2) post-hoc removal detected → invalidates the verdict**
⑥ **🔧 THE GATE BINDS: claims-as-oracle stays structurally pinned advisory/OFF and regen promotion stays structurally blocked until the report shows PASS; a FAIL/insufficient-n report CANNOT flip either feature; a DEGRADED gate-evaluator state = "insufficient" → fails CLOSED, cannot promote; the promotion event itself is audited**
⑦ **(R6) degradation honesty: waves executed during a degraded window are excluded from the corpus or explicitly marked — never silently contribute biased data**
⑧ **(R9) source-eligibility WIRED to the focus gate: the experiment harness rejects focus-gate-ineligible changes (corelink-server) at INGESTION — the corpus eligibility predicate IS the X10② exclusion, fail-closed + audited**
⑨ **(R10) the gate REPORT is itself an attested, tamper-evident object (X2-class): a forged/swapped PASS report cannot enable promotion or billing through any path other than a genuine evaluation**

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `EventRecord` — waves emit datapoints onto the stream; promotion + ingestion-reject events are audited here.
- `RegenGate {optin_scope, repass, indep_verdict}` — the feature D8 gates (consumed; D12 IMPLEMENTS the regen gate, D8 BINDS its promotion).
- `AttestationChain {tree, def, runner, model, principal, sig}` — the X2-class attestation the gate report itself carries (⑨).
- Focus-gate eligibility predicate (X10②) — the ingestion filter (⑧); consumed, not redefined.

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-diag/experiment/` (the entire module: datapoint collector,
  disjointness/regen-agreement dashboard, report generator, corpus seal,
  post-hoc-removal detector, the BINDING control surface, ingestion eligibility
  filter, degradation-window marker, report attestation).
- `crates/hugit-diag/experiment/tests/`.
Writes outside `hugit-diag/experiment/` = leak. (D12 owns regen-gate mechanics;
X10 owns the focus-gate definition — D8 CONSUMES both seams, modifies neither.)

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D8 row) + §2 (B9⑥ money
  gate, the symmetric control) + §6 (X10②/X2) + §8; `docs/whitepaper/hugit-v1.md`
  §6.3 (regen rebase) + §6.2; `docs/product/command-catalog.md` (the experiment
  gate, the two standing gates); frozen `EventRecord`/`RegenGate`/`AttestationChain`
  schemas; the focus-gate eligibility predicate from X10.
- Anchors: the gate is a CONTROL — promotion is structurally impossible until
  report=PASS. Fixtures: pre-sealed corpus, a post-hoc removal, a degraded window,
  a corelink-server ingestion attempt, a forged PASS report.
- Conventions: house stack (Rust); fmt+clippy+test+audit; failing acceptance suite
  committed BEFORE implementation; opus route (security/control); SEAL.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **The gate is a fail-CLOSED CONTROL, not a dashboard (⑥):** claims-as-oracle is
  PINNED advisory/OFF and regen promotion is structurally BLOCKED until the report
  = PASS. A FAIL or insufficient-n report CANNOT flip either feature. A DEGRADED
  gate-evaluator state = "insufficient" → fails CLOSED (cannot promote). The
  promotion event is itself audited. Symmetric to B9⑥ (the money gate).
- **Pre-seal before evaluation (④⑤):** the promotion corpus is pre-registered and
  SEALED before any evaluation; sample selection is auditable. A post-hoc removal
  from the sealed corpus is DETECTED and INVALIDATES the verdict (re-pin fail-closed).
- **Source-eligibility = the focus-gate exclusion (⑧):** the ingestion filter's
  eligibility predicate IS the X10② exclusion — a focus-gate-ineligible change
  (corelink-server) is REJECTED at INGESTION, fail-closed + audited. A
  corelink-server change can never become a datapoint that flips claims/regen.
  D8 CONSUMES the X10 predicate; it does not redefine the focus gate.
- **Degradation honesty (⑦):** waves executed in a degraded window are EXCLUDED
  from the corpus or EXPLICITLY MARKED — never silently biasing the data.
- **Report = attested tamper-evident object (⑨):** the gate report carries an
  X2-class `AttestationChain`; a forged/swapped PASS report cannot enable promotion
  or billing through any path other than a genuine evaluation — assert the forged
  report is rejected at the binding surface.
- **Generated, never hand-written (③):** the report is produced from collected
  data only; assert no hand-authoring path exists.

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All nine owned items green · zero writes outside `crates/hugit-diag/experiment/` ·
evidence bundle (auto-contribution proof, dashboard fields, generated-report
assertion, seal + post-hoc-removal invalidation, FAIL/insufficient/degraded
cannot-promote transcripts, corelink-server ingestion reject, forged-report
reject) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–⑨: green/red) · evidence refs (test ids + fixture paths +
fail-closed transcripts + attestation proof) · claims-respected: yes ·
deviations: none | waiver-ref.
