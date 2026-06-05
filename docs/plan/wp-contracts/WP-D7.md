# WP-D7 — verdict panels + review Q&A
squad D · M · opus · ctx 80k · branch: wp/D7

## Charter
Build adversarial verdict panels: `verdict request --lens` fans out to
INDEPENDENT reviewer agents — distinct prompts AND ≥2 distinct models — each
judging against SERVED ground truth (build-graph impact, contracts, evidence).
The change NEVER defends itself: no author-controlled text reaches a reviewer.
Plus grounded human review Q&A — answers are citations to real evidence objects,
or an explicit refusal.

## Owned acceptance (VERBATIM — decomposition v2.0 D7①–⑥)
① lenses isolated (prompt audit)
② valid VerdictObject[] + evidence refs
③ **🔧 planted bug of a NON-author-visible class (semantic/logic, demonstrably uncovered by any author test) caught by ≥1 lens**
④ **(R2) human review Q&A: answers = citations to real evidence objects; no grounding → explicit refusal, never fabricated**
⑤ **(R3) DIVERSITY enforced: homogeneous (same prompt+model) panel rejected/flagged; real panel dispatches distinct prompts and ≥2 distinct models**
⑥ **(R3) no-self-defense negative: planted persuasive false self-justification in author-controlled fields is unreachable by reviewers; verdict identical with vs without it (no persuasion channel exists)**

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `VerdictObject` `{intent, tree_hash, lens, verdict ∈ {APPROVE,FIX-FIRST,REJECT}, claims_checked[], evidence[]}` — the panel's output type (②).
- `IntentSidecar` — the intent under review (B6 dependency); its AUTHOR-CONTROLLED
  fields are the channel that must be PROVEN unreachable (⑥).
- `CheckResult` / impact ground truth (D10) — the served evidence reviewers judge against.
- `EventRecord` — verdicts/Q&A are emitted/attributable on the stream.

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-cli/verdict/` (the entire module: lens fan-out, reviewer-agent
  dispatch, ground-truth server, diversity enforcer, prompt-isolation auditor,
  Q&A grounded-retrieval engine, refusal path).
- `crates/hugit-cli/verdict/tests/` (lens-isolation audit fixtures, planted-bug
  fixtures, persuasion-channel negative fixtures).
Writes outside `hugit-cli/verdict/` = leak.

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D7 row) + §8;
  `docs/whitepaper/hugit-v1.md` §4 (Verdict object) + §8 item 5 (review =
  interrogation, grounded) + Inversion 4; `docs/product/command-catalog.md`
  (Adversarial verdict panels + Assisted human review rows, both HARDENED);
  frozen `VerdictObject`/`IntentSidecar`/`CheckResult`/`EventRecord` schemas.
- Anchors: reviewers see ONLY served ground truth; B6 supplies the intent corpus.
  Fixtures include a semantic/logic planted bug and a planted false self-justification.
- Conventions: house stack (Rust); fmt+clippy+test+audit; failing acceptance
  suite committed BEFORE implementation; opus route (adversarial judgment); SEAL.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **Panels = independent reviewer agents (①⑤):** each lens is an INDEPENDENT
  reviewer; the panel MUST dispatch DISTINCT prompts AND ≥2 DISTINCT models. A
  homogeneous panel (same prompt+model) is REJECTED/flagged by construction —
  diversity is enforced at dispatch, audited by the prompt-isolation auditor (①).
- **Served ground truth ONLY — the change never defends itself (⑥):** reviewers
  receive served evidence (impact, contracts, CheckResults) — NEVER
  author-controlled text. Author-controlled IntentSidecar fields are structurally
  outside the reviewer input set. The negative proof: plant a persuasive false
  self-justification in those fields → the verdict is IDENTICAL with vs without
  it (byte-identical), proving no persuasion channel exists. This is the
  no-self-defense invariant, asserted, not asserted-by-policy.
- **VerdictObject[] (②):** every lens emits a valid `VerdictObject` with real
  `evidence[]` refs resolving to actual evidence objects; schema-validate the array.
- **Non-author-visible bug class (③):** the planted bug is semantic/logic and
  DEMONSTRABLY uncovered by any author test (show the author suite passing);
  ≥1 lens still catches it — proving the panel adds coverage authors cannot.
- **Q&A grounding (④):** human-review answers are CITATIONS to real evidence
  objects; when no grounding exists the engine REFUSES explicitly — it never
  fabricates. Grounded retrieval, not generative persuasion. Assert both the
  cited-answer path and the refusal path.

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All six owned items green · zero writes outside `crates/hugit-cli/verdict/` ·
evidence bundle (prompt-isolation audit, model-diversity assertion, planted-bug
catch, Q&A citation+refusal transcripts, persuasion-channel identical-verdict
proof) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–⑥: green/red) · evidence refs (test ids + fixture paths +
diversity/isolation audit + persuasion-negative proof) · claims-respected: yes ·
deviations: none | waiver-ref.
