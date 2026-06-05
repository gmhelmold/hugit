# WP-E4 — Actions-YAML compatibility shim

squad E · M · sonnet · 70k · branch: wp/E4

## Charter
A migration-lubricant shim that runs a SUPPORTED subset of existing GitHub
Actions workflows on hugit's runners. The supported subset is a published,
proven-to-execute contract with a falsifiable boundary; secrets fail CLOSED;
and a deterministic fixture workflow runs EQUIVALENTLY on real GitHub Actions
and on the shim.

## Owned acceptance
*(VERBATIM from decomposition v2.0, E4; this WP owns items ① ② ③ ④.)*

- **① 🔧** the SUPPORTED subset is a published contract; "supported" =
  proven-to-execute, not merely documented
- **② 🔧** outside the contract → explicit actionable report — falsifiable
  boundary, no silent skip
- **③(+)** missing/denied secret → fail CLOSED w/ named secret; material never
  in logs/env (red-team)
- **④ 🔧** execution EQUIVALENCE: a DETERMINISTIC fixture workflow (determinism
  precondition stated) runs on real GitHub Actions AND on the shim → equivalent
  observable outcomes (steps, env, artifacts, exit states)

## Contract deps
*(frozen types/APIs consumed from `hugit-contracts`; never modified here)*

- `RunnerLease` — the shim executes workflow steps inside a leased runner.
- `FenceManifest` + secrets-broker surface (from **C5**, `hugit-fence`): the
  shim resolves secrets VIA the broker; raw material never enters the runner.
  Consumed as a frozen producer; not modified.
- **C2** ephemeral runner (`hugit-runner`): the execution substrate. Consumed;
  the shim is a new submodule under it.

## Claims
*(paths this WP owns — disjoint by construction; writes outside = leak)*

- `crates/hugit-runner/src/shim/` — Actions-YAML parser, supported-subset
  contract, step executor, out-of-contract reporter, secrets-via-broker
  resolution, equivalence harness driver.
- `tests/shim/supported_contract_*.rs`, `tests/shim/boundary_*.rs`,
  `tests/shim/secrets_failclosed_*.rs`, `tests/shim/equivalence_*.rs`.
- `docs/shim/supported-subset.md` — the published supported-subset contract.

## Dispatch packet
- This contract file.
- Frozen `RunnerLease`, `FenceManifest`, C5 secrets-broker signature, C2 runner
  signature.
- Anchor: `crates/hugit-runner/lib.rs` barrel exports `shim`.
- Conventions: every workflow feature is either IN the published supported
  contract (proven-to-execute) or produces an **explicit actionable out-of-
  contract report** — never a silent skip; all secret access through the broker.

## Implementation notes
*(every fork PRE-DECIDED — the zero-decision guarantee)*

- **Supported subset (①)** is a **published contract** (`docs/shim/supported-
  subset.md`) where "supported" = **proven-to-execute** by a passing fixture per
  listed feature — not merely documented.
- **Boundary (②)**: anything outside the contract triggers an **explicit
  actionable report** naming the unsupported construct — a **falsifiable
  boundary**, zero silent skips.
- **Secrets (③)**: secrets resolve **via the C5 broker only**; a missing/denied
  secret **fails CLOSED** naming the secret; raw material **never** appears in
  logs or env (red-team asserted, consistent with C5②/⑤).
- **Equivalence (④)**: a **deterministic** fixture workflow (determinism
  precondition explicitly stated — pinned toolchain/inputs, no wall-clock/net
  nondeterminism) runs on **real GitHub Actions** AND on the shim; assert
  **equivalent observable outcomes** — steps run, env seen, artifacts produced,
  exit states. The determinism precondition is the gate for the comparison (cf.
  v2.0 E4④, the R5 determinism-precondition pin).

## DoD
*(global: fmt+clippy+test+audit green · owned items red→green ·
cold-verify pass by non-author)*

- `cargo fmt --check` · `cargo clippy -D warnings` · `cargo test` ·
  `cargo audit` green on `wp/E4`.
- Owned items ① ② ③ ④ red→green; failing suites committed first.
- Cold verification by non-author; **dedicated secrets red-team** at SEAL
  (rides C5's red-team discipline).
- DCO + CHANGELOG `[Unreleased]` entry.

## Completeness
- All owned items (① ② ③ ④) green.
- Zero writes outside Claims.
- Evidence bundle (published supported-subset contract + proven-to-execute
  fixtures, out-of-contract actionable-report proof, secrets fail-CLOSED red-
  team trace, real-Actions-vs-shim equivalence proof) attached to SEAL.

## Return shape
SEAL ≤20 lines: status · evidence refs · items ①②③④ red→green ·
deviations = none | waiver-ref.
