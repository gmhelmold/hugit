# WP-D9 — attention queue
squad D · M · opus · ctx 60k · branch: wp/D9

## Charter
Build the human's inbox: the attention queue that ranks what needs a human by a
DOCUMENTED composite of policy × blast-radius × verdict-confidence. Policy-mandatory
items can never be ranked out of view; fast-approve is blocked for high-risk/
policy-mandatory items; and under degradation the queue never goes silently dark —
mandatory items still surface with an honest "ranking degraded" state.

## Owned acceptance (VERBATIM — decomposition v2.0 D9①–⑤)
① fixture with known policy/blast/confidence → documented composite ordering reproduced
② perturbing one input moves entry to expected position
③ policy-mandatory items can never be ranked out of the human's view
④ **(R2) fast-approve (90s) affordance is BLOCKED for high-risk/policy-mandatory items — forced through full review; permitted for policy-low-risk**
⑤ **(R7) up-zoom under degradation: with ranking inputs (blast/confidence) unavailable, policy-mandatory items STILL surface with an honest "ranking degraded" state — the queue never goes silently dark on items that must reach a human**

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `AttentionRank {policy, blast_radius, confidence}` — the three composite inputs (the rank is defined by this frozen type; D9 implements its documented composition).
- `VerdictObject` — supplies the `confidence` input (from D7 panels).
- Blast-radius input — from D10 `impact` (the DAG edge D10 → D9); consumed read-only.
- Policy descriptor — supplies `policy`/mandatory classification (from D6); consumed read-only.

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-cli/attention/` (the entire module: the documented composite
  ranker, the policy-mandatory floor, the fast-approve affordance gate, the
  degraded-ranking up-zoom path).
- `crates/hugit-cli/attention/tests/`.
Writes outside `hugit-cli/attention/` = leak. (D7 owns `hugit-cli/verdict/`; D9
CONSUMES VerdictObject + D10 impact — disjoint modules under the same crate.)

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D9 row) + §7 (D10→D9
  blast-radius edge, D7→D9) + §8; `docs/whitepaper/hugit-v1.md` §8 item 4 (the
  attention queue is the inbox); `docs/product/command-catalog.md` (attention
  queue + approve-in-90s rows); frozen `AttentionRank`/`VerdictObject` schemas;
  the D10 impact ground truth + D6 policy classification as input seams.
- Anchors: ranking is a DOCUMENTED composite of policy × blast × confidence;
  fixtures carry known input triples and expected orderings. Degradation fixture
  removes blast/confidence inputs.
- Conventions: house stack (Rust); fmt+clippy+test+audit; failing acceptance suite
  committed BEFORE implementation; opus route; SEAL.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **Rank = documented composite (①②):** attention rank is the DOCUMENTED
  composition of policy × blast-radius × verdict-confidence over `AttentionRank`.
  The composition function is written down; a known-input fixture reproduces the
  exact ordering, and perturbing ONE input moves an entry to its expected position.
- **Policy-mandatory floor (③):** policy-mandatory items can NEVER be ranked out
  of the human's view — they have a structural floor independent of blast/confidence.
- **Fast-approve gate (④):** the 90s fast-approve affordance is BLOCKED for
  high-risk/policy-mandatory items (forced through full review) and PERMITTED only
  for policy-low-risk. Assert both the block and the permit paths.
- **Up-zoom under degradation (⑤):** when blast/confidence inputs are unavailable,
  policy-mandatory items STILL surface, carrying an honest "ranking degraded"
  state — the queue NEVER goes silently dark on items that must reach a human.
  Drive a degraded fixture and assert the mandatory items still appear, labeled.
- **Consumes, never owns inputs:** D9 reads D10 impact (blast) + D7 VerdictObject
  (confidence) + D6 policy — it modifies none of them; the seams are frozen.

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All five owned items green · zero writes outside `crates/hugit-cli/attention/` ·
evidence bundle (documented-composite ordering reproduction, perturbation test,
mandatory-floor proof, fast-approve block/permit transcripts, degraded up-zoom
labeled-surface proof) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–⑤: green/red) · evidence refs (test ids + fixture paths +
composite-doc ref + degraded transcript) · claims-respected: yes · deviations:
none | waiver-ref.
