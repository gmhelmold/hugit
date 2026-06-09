# WP-E6 — bidirectional mirror (SUPERSEDED + BUILT 2026-06-08)

squad E · L · opus · 60k · branch: wp/E6

> **SUPERSEDED.** The gate-bound write-back design was superseded by the
> forge-arbitrated seamless bidirectional-sync design built 2026-06-08.
> See `docs/design/2026-06-08-seamless-bidirectional-sync.md` for the
> implemented design. The acceptance items below remain as the safety contract.

## Charter
The bidirectional mirror: bounded, forge-authoritative write-back from GitHub
into hugit, gated behind months of clean E1 one-way soak. It is NEVER naive
symmetric — GitHub is authoritative on conflict, write-back is bounded, webhook
sync is idempotent. Built only after the soak gate + panel + catalog decision
open it.

## Owned acceptance
*(VERBATIM from decomposition v2.0, E6 — PRE-REGISTERED for gate-open.)*

- **①** write-back is BOUNDED (rate/scope limits enforced)
- **②** webhook sync idempotent (duplicate/out-of-order deliveries converge)
- **③** forge-authoritative conflict handling (GitHub-side concurrent edit →
  forge wins, divergence incident)
- **④** property test: no state reachable where the two sides sync symmetrically
  without forge authority ("never naive symmetric")

## Contract deps
*(frozen types/APIs consumed from `hugit-contracts`; never modified here)*

- `EventRecord`, `AppWebhooks` / GitHub App auth — write-back lands events; the
  webhook channel feeds idempotent sync.
- **E1** one-way mirror (`hugit-mirror/outbound`, `divergence`): E6 is the
  inverse direction layered ON the soaked one-way mirror; it reuses E1's
  divergence/forge-authoritative machinery. The one-way enforcement (E1⑦) is
  the invariant E6 is permitted to relax ONLY under the gate.
- The gate decision artifact (soak-report PASS + panel + catalog decision) — a
  hard precondition; E6 consumes its existence as the dispatch authorization.

## Claims
*(paths this WP owns — disjoint by construction; writes outside = leak)*

- `crates/hugit-mirror/src/sync/` — forge-arbitrated bidirectional sync engine, idempotent
  convergence, forge-authoritative conflict resolver.
- `tests/mirror/writeback_bounded_*.rs`, `tests/mirror/webhook_idempotent_*.rs`,
  `tests/mirror/forge_authoritative_*.rs`, `tests/mirror/never_symmetric_*.rs`
  (the property test).

## Dispatch packet
*(delivered ONLY on gate-open; until then this section is dormant.)*
- This contract file.
- The recorded gate decision (soak PASS + panel + catalog) — without it,
  dispatch is forbidden.
- Frozen `EventRecord`, `AppWebhooks`/auth anchors; E1 divergence/forge-
  authoritative seam signatures.
- Anchor: `crates/hugit-mirror/lib.rs` barrel export `writeback`.
- Conventions: GitHub authoritative on conflict; write-back bounded by
  enforced rate/scope; webhook sync idempotent; symmetric-without-authority is
  an unreachable state (property-tested).

## Implementation notes
*(every fork PRE-DECIDED — the zero-decision guarantee)*

- **DISPATCH PRECONDITION**: months of E1 one-way soak must run clean and the
  **panel + catalog decision** must explicitly open the gate. No agent begins
  E6 before that record exists. (Symmetric to the money gate / experiment gate
  doctrine: a control, not a dashboard.)
- **Bounded write-back (①)**: rate AND scope limits are **enforced** (not
  advisory) — a bound on how much/which refs can flow GitHub→hugit; overflow is
  refused, not absorbed.
- **Idempotent webhook sync (②)**: duplicate and out-of-order webhook deliveries
  **converge** to the same state (dedup by delivery id + ordering reconciliation
  against the event log).
- **Forge-authoritative conflict (③)**: on a GitHub-side concurrent edit,
  **forge wins** and a **divergence incident** is raised — hugit yields to
  GitHub for write-back conflicts (the inverse of E1's hugit-authoritative
  one-way repair, by deliberate design of this direction).
- **Never naive symmetric (④)**: a **property test** proves NO reachable state
  exists where the two sides sync symmetrically **without forge authority** —
  symmetry without an authority arbiter is structurally impossible.

## DoD
*(global: fmt+clippy+test+audit green · owned items red→green ·
cold-verify pass by non-author)*

- *(Applies on gate-open.)* `cargo fmt --check` · `cargo clippy -D warnings` ·
  `cargo test` · `cargo audit` green on `wp/E6`.
- Owned items ① ② ③ ④ red→green; failing suites (incl. the never-symmetric
  property test) committed first.
- Cold verification by non-author; security review at SEAL (write-back is a new
  trust-boundary direction).
- DCO + CHANGELOG `[Unreleased]` entry.

## Completeness
- *(On gate-open.)* All owned items (① ② ③ ④) green.
- Zero writes outside Claims.
- Evidence bundle (bounded-write-back enforcement proof, idempotent-convergence
  proof, forge-wins conflict proof, never-naive-symmetric property-test proof,
  AND the recorded gate-open decision) attached to SEAL.
- **Until gate-open: completeness = N/A; the contract sits frozen in the
  register, undispatched.**

## Return shape
SEAL ≤20 lines: status · evidence refs · items ①②③④ red→green · gate-decision
ref · deviations = none | waiver-ref.
