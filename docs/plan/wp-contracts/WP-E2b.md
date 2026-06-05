# WP-E2b — import: PR/issue → proposed intents + fidelity contract

squad E · M · sonnet · 70k · branch: wp/E2b

## Charter
Import GitHub **PR/issue metadata** into hugit as **proposed, non-
authoritative** intents with per-element provenance, against a stated fidelity
contract (body, comment/review threads, state, labels, cross-refs) that
explicitly enumerates what is and is not imported. Git history import (byte-
identity, LFS, resume, idempotency) is E2a.

## Owned acceptance
*(VERBATIM from decomposition v2.0, E2; this WP owns items ② ⑥.)*

- **②** PRs/issues→proposed intents w/ provenance
- **⑥(R3)** PR/issue fidelity contract: stated set (body, comment/review
  threads, state, labels, cross-refs) preserved with per-element provenance;
  non-imported elements explicitly enumerated; verified on a fixture containing
  each element

## Contract deps
*(frozen types/APIs consumed from `hugit-contracts`; never modified here)*

- `IntentSidecar` / native intent shape — PR/issue imports become **proposed,
  non-authoritative** intents of this type (state-flagged `proposed`).
- **D4** intent projection + `intent_id` (`hugit-refstore/intent`): proposed
  intents are minted with the native lifecycle/id. Consumed, not modified.
- **E2a** import auth + history boundary (`hugit-mirror/import`): E2b reuses the
  installation-auth client and runs ALONGSIDE history import; the boundary law
  (no intent from a bare commit, ⑤) is E2a's — E2b only mints intents from
  PR/issue **metadata**, flagged proposed.

## Claims
*(paths this WP owns — disjoint by construction; writes outside = leak)*

- `crates/hugit-mirror/src/import/prissue/` — PR/issue fetch, proposed-intent
  projection, per-element provenance attachment, fidelity-contract enforcement
  + the explicit non-imported-element enumeration.
- `tests/import/prissue_*.rs`, `tests/import/fidelity_*.rs`.

## Dispatch packet
- This contract file.
- Frozen `IntentSidecar`/intent shape + D4 `intent_id` anchors.
- E2a installation-auth client signature.
- Anchor: `crates/hugit-mirror/lib.rs` barrel exports `import::prissue`.
- Conventions: every imported PR/issue intent carries state = **proposed /
  non-authoritative**; every preserved element carries per-element provenance;
  the non-imported set is an explicit, enumerated, published list.

## Implementation notes
*(every fork PRE-DECIDED — the zero-decision guarantee)*

- **Proposed, non-authoritative (②)**: PR/issue → intent, **flagged proposed** —
  it never gates/blocks/lands anything on import (consistent with B6④
  sidecar-non-authoritative + E2a⑤ boundary). Provenance: source PR/issue URL +
  element-level origin recorded.
- **Fidelity contract (⑥)** — the **stated set** preserved with per-element
  provenance: **body, comment/review threads, state, labels, cross-refs**. Each
  preserved element is verified on a fixture **containing each element**. The
  **non-imported elements are explicitly enumerated** (a published list — e.g.
  reactions, social-graph signals — riding the mirror, not stormed; cf.
  whitepaper §10). No silent drop: anything not imported is named.
- **Cross-refs** resolve to the imported objects where both ends are imported;
  dangling cross-refs are recorded as explicit residual, never fabricated.

## DoD
*(global: fmt+clippy+test+audit green · owned items red→green ·
cold-verify pass by non-author)*

- `cargo fmt --check` · `cargo clippy -D warnings` · `cargo test` ·
  `cargo audit` green on `wp/E2b`.
- Owned items ② ⑥ red→green; failing suites committed first.
- Cold verification by non-author; security review at SEAL.
- DCO + CHANGELOG `[Unreleased]` entry.

## Completeness
- All owned items (② ⑥) green.
- Zero writes outside Claims.
- Evidence bundle (proposed-intent provenance proof, per-element fidelity
  fixture proof, enumerated non-imported-element list) attached to SEAL.

## Return shape
SEAL ≤20 lines: status · evidence refs · items ②⑥ red→green ·
deviations = none | waiver-ref.
