# WP-C1 — runner inventory + reuse verdict
squad C · S · opus · 40k · branch: wp/C1

## Charter
Inventory the existing ephemeral-runner work in the CoreLink ecosystem
(campaign #1) and emit a written reuse verdict per item: reuse-verbatim,
adapt, or build-new. Read-only over adjacent prod repos — zero changes there.
This grounds C2a/C2b/C3/C5a/C5b so they reuse the fabric rather than reinvent it.

## Owned acceptance
① written inventory + reuse verdict/item. **(R2: "zero changes to adjacent prod repos" reclassified → governance law §8, not a feature acceptance)**

## Contract deps
- `RunnerLease` (consumed read-only — informs which lease fields the inventoried
  runner work already supplies; never modified here).
- No other frozen types touched; this WP produces documentation, not code paths
  against `hugit-contracts`.

## Claims
- `docs/inventory/` (the inventory document + per-item reuse verdict table).
- No source crate paths claimed — C1 writes documentation only.

## Dispatch packet
- Files received: this contract · `docs/plan/decomposition.md` §3 (C-squad) ·
  `docs/plan/warp-10-days.md` (Squad C table, C1 row) ·
  `docs/whitepaper/hugit-v1.md` §5 (L3 CoreLink Runners status: "in flight,
  campaign #1") · the CoreLink-side runner/campaign-#1 sources (READ-ONLY).
- Anchors: the inventory enumerates each existing runner capability and binds a
  verdict ∈ {reuse-verbatim, adapt, build-new} with a one-line rationale.
- Conventions: Markdown table under `docs/inventory/`; cite source paths.

## Implementation notes (every fork PRE-DECIDED)
- The target runtime is **container-per-job on the single Hetzner-class box**;
  the Firecracker upgrade path is **documented, not built** — the inventory
  records which campaign-#1 assets map onto container-per-job now and which are
  Firecracker-only (deferred).
- `clw` is the reference snapshot/hydrate/run client; the inventory verdicts
  treat `clw` capabilities as reuse-verbatim substrate, never re-implemented.
- The broker follows the CoreLink **write-only secret model generalized**
  (credentials never on the runner); the inventory flags any existing secret
  handling against that model.
- **Read-only law:** adjacent prod repos (corelink-server et al.) are inventoried
  by reading only — zero edits, per governance law §8. No git operations.

## DoD
Global bar: fmt + clippy + test + audit green (vacuous for a docs-only WP, but
the CI gates must still pass) · owned item red→green · cold-verify pass by a
non-author · zero writes outside claims.

## Completeness
All owned items green · zero writes outside `docs/inventory/` · evidence bundle
(the inventory doc + reuse verdicts + cited source refs) attached to SEAL.

## Return shape
SEAL ≤20 lines: status, evidence refs (inventory path + verdict count),
deviations = none | waiver-ref.
