# WP-D13 — tournament
squad D · S · sonnet · ctx 50k · branch: wp/D13

## Charter
Build `hugit tournament -n N`: produce N independent candidate implementations of
one intent, let a judge panel select per documented criteria, keep losers
addressable as evidence. N is policy-capped and budget-bounded — the fan-out
respects per-tenant caps + fairness and generates ZERO overage under a flat plan
(the one cost amplifier, now budgeted).

## Owned acceptance (VERBATIM — decomposition v2.0 D13①–④)
① `-n N` produces N independent candidates
② judge panel selects per documented criteria (fixture w/ known-best)
③ losers remain addressable as evidence
④ **(R7) budget-bounded fan-out: N is policy-capped; an N-way tournament respects per-tenant caps + fairness (C7) and generates ZERO overage under a flat plan (the one cost amplifier was unbudgeted — now it isn't)**

## Contract deps (frozen — consumed from hugit-contracts, never modified here)
- `VerdictObject` — the judge panel's selection output (②); the panel is the D7 mechanism, consumed here.
- `IntentSidecar` / native intent id — the one intent the N candidates implement.
- Per-tenant budget/fairness surface (C7) — the cap the fan-out respects (④); consumed read-only.
- `EventRecord` — candidates, selection, and losers-as-evidence are recorded on the stream.

## Claims (paths this WP owns — disjoint by construction)
- `crates/hugit-cli/tournament/` (the `-n N` fan-out, the judge-panel selection
  per documented criteria, loser-as-evidence addressing, the policy cap +
  budget-bounded enforcement).
- `crates/hugit-cli/tournament/tests/`.
Writes outside `hugit-cli/tournament/` = leak. (D7 owns `verdict/`, D9 owns
`attention/`, D10 owns `why/`+`impact/` — D13 is a disjoint module under the
shared `hugit-cli` crate; it CONSUMES the D7 panel + C7 budget surfaces.)

## Dispatch packet (exactly what the executing agent receives)
- Files: this contract; `docs/plan/decomposition.md` §4 (D13 row) + §3 (C7
  budgets/fairness — the cap seam) + §8; `docs/whitepaper/hugit-v1.md` §7 (`hugit
  tournament` verb) + §11 (flat pricing, never meter the customer's compute);
  `docs/product/command-catalog.md` (Tournament intents CORE; `hugit tournament -n
  N` row); frozen `VerdictObject`/`EventRecord` + the C7 budget surface.
- Anchors: tournament = orchestration over existing primitives; N is POLICY-CAPPED;
  the judge panel uses DOCUMENTED criteria with a known-best fixture. Fixture:
  N candidates with one known-best + a cap-overrun attempt.
- Conventions: house stack (Rust); fmt+clippy+test+audit; failing acceptance suite
  committed BEFORE implementation; SEAL.

## Implementation notes (every fork PRE-DECIDED — zero live decisions)
- **N independent candidates (①):** `-n N` produces N INDEPENDENT candidate
  implementations of the one intent — assert independence (no shared mutation).
- **Documented-criteria selection (②):** the judge panel selects by WRITTEN
  criteria; on a fixture with a KNOWN-BEST candidate, assert the panel picks it.
  The panel is the D7 verdict mechanism (independent reviewers, distinct prompts +
  ≥2 models, served ground truth) — D13 consumes it, does not re-implement it.
- **Losers as evidence (③):** losing candidates remain ADDRESSABLE as evidence
  objects (not discarded) — assert resolvability after selection.
- **Budget-bounded fan-out (④):** N is POLICY-CAPPED; the N-way tournament respects
  per-tenant caps + C7 fairness and generates ZERO overage under a flat plan. Drive
  a cap-overrun attempt and assert it is bounded (capped/refused) with zero overage
  charge — the cost amplifier is now budgeted, not unbudgeted.
- **Consumes, never owns:** D13 owns orchestration; the panel (D7) and the budget
  surface (C7) are consumed seams, modified by neither.

## DoD (global bar — identical for every WP)
fmt + clippy + test + audit green · all owned items red→green · cold-verify
pass by a non-author agent · zero writes outside Claims.

## Completeness
All four owned items green · zero writes outside `crates/hugit-cli/tournament/` ·
evidence bundle (N-independent-candidates proof, known-best selection, loser
addressability, policy-cap + zero-overage assertion) attached to SEAL.

## Return shape (SEAL ≤20 lines)
status (per item ①–④: green/red) · evidence refs (test ids + fixture paths +
cap/overage assertion) · claims-respected: yes · deviations: none | waiver-ref.
