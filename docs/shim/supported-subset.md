# Actions-YAML Shim — Published Supported-Subset Contract

**Version:** v0  
**WP:** E4  
**Status:** PUBLISHED — every feature listed here is proven-to-execute by a
passing fixture in `crates/hugit-runner/tests/acceptance_e4.rs`.  
**Machine-readable source of truth:** `crates/hugit-runner/src/shim/subset.rs`

---

## Contract guarantee

"Supported" means **proven-to-execute**, not merely documented. Every feature
in this list:

1. Is parsed correctly by the shim's YAML parser (`shim/parser.rs`).
2. Executes through the shim's executor (`shim/executor.rs`) and produces
   observable outcomes equivalent to real GitHub Actions for a deterministic
   fixture workflow.
3. Is covered by a named fixture in `tests/acceptance_e4.rs` (item ①).

Any workflow construct **not** in this list produces an explicit, actionable
`OutOfContractReport` (item ②) — never a silent skip.

---

## Falsifiable boundary

The boundary is falsifiable: if a feature is listed here but fails to execute
correctly on the shim, item ① of the acceptance suite turns RED. If a feature
is used in a workflow but is not listed here, item ② asserts that an
`OutOfContractReport` is produced naming the unsupported construct.

---

## Supported features (v0)

### Trigger events (`on:`)

| Feature | Shim enum | Proven by |
|---|---|---|
| `on: push` | `SubsetFeature::OnPush` | `item_1_supported_subset_proven_to_execute` |
| `on: pull_request` | `SubsetFeature::OnPullRequest` | `item_1_supported_subset_proven_to_execute` |
| `on: workflow_dispatch` (no inputs) | `SubsetFeature::OnWorkflowDispatch` | `item_1_supported_subset_proven_to_execute` |

### Job-level structure

| Feature | Shim enum | Proven by |
|---|---|---|
| Single job, `runs-on: ubuntu-latest` | `SubsetFeature::SingleJobUbuntu` | `item_1_supported_subset_proven_to_execute` |
| Job-level `env:` (static string values only) | `SubsetFeature::JobEnvStatic` | `item_1_supported_subset_proven_to_execute` |
| Job `if: ${{ always() }}` | `SubsetFeature::JobIfAlways` | `item_1_supported_subset_proven_to_execute` |

Supported `runs-on` values: `ubuntu-latest`, `ubuntu-22.04`, `ubuntu-20.04`.

### Step-level structure

| Feature | Shim enum | Proven by |
|---|---|---|
| `run:` step, single-line | `SubsetFeature::RunStepSingleLine` | `item_1_supported_subset_proven_to_execute` |
| `run:` step, multi-line (`\|`) | `SubsetFeature::RunStepMultiLine` | `item_1_supported_subset_proven_to_execute` |
| Step-level `env:` (static strings only) | `SubsetFeature::StepEnvStatic` | `item_1_supported_subset_proven_to_execute` |
| Step `name:` field | `SubsetFeature::StepName` | `item_1_supported_subset_proven_to_execute` |
| Step `if: ${{ always() }}` | `SubsetFeature::StepIfAlways` | `item_1_supported_subset_proven_to_execute` |
| Step `continue-on-error: true` | `SubsetFeature::StepContinueOnError` | `item_1_supported_subset_proven_to_execute` |

### Secret resolution

| Feature | Shim enum | Proven by |
|---|---|---|
| `${{ secrets.NAME }}` resolved via C5 broker | `SubsetFeature::SecretExpression` | `item_3_secrets_fail_closed_named` |

Secret resolution contract (item ③):
- Secrets are resolved **only** via the C5 secrets broker (`shim/broker.rs`).
- Missing or denied secrets cause a **fail-CLOSED** error naming the secret.
- Raw secret material **never** appears in logs, environment variables, or
  any returned `String`. The red-team assertion is in
  `item_3_secrets_not_in_logs_env`.

### Actions (uses:)

| Feature | Shim enum | Proven by |
|---|---|---|
| `actions/upload-artifact@v3` | `SubsetFeature::UploadArtifactV3` | `item_2_out_of_contract_actionable_report` |
| `actions/download-artifact@v3` | `SubsetFeature::DownloadArtifactV3` | `item_2_out_of_contract_actionable_report` |

---

## Out-of-contract constructs (v0)

The following constructs are **explicitly not supported** in v0. Using them in
a workflow produces an `OutOfContractReport` — never a silent skip.

- `runs-on: windows-latest` or `macos-*` (non-ubuntu runners)
- `strategy:` / `matrix:` (multi-job matrix builds)
- `services:` (service containers)
- `container:` (container jobs)
- `timeout-minutes:` (job or step level)
- Any `uses:` action other than `actions/upload-artifact@v3` and
  `actions/download-artifact@v3`
- Multiple concurrent jobs (parallel job execution)
- `${{ github.* }}` context expressions in static `env:` values
- Floating action refs (`@latest`, `@main`, `@master`)

---

## Equivalence precondition (item ④)

A workflow is eligible for the shim↔GitHub-Actions equivalence comparison only
if it satisfies the **determinism precondition**:

1. **Pinned toolchain** — no `@latest`, `@main`, or `@master` in `uses:` refs.
2. **No wall-clock nondeterminism** — no `date`, `$RANDOM`, or wall-clock
   reads in `run:` steps.
3. **No net nondeterminism** — no live HTTP calls in `run:` steps; only the GH
   API itself (via the harness) is permitted.
4. **Single-job, sequential steps** — no parallel job execution.
5. **Static env values** — no `${{ github.run_number }}`, `${{ github.sha }}`,
   etc. in `env:` maps.

Workflows that fail the determinism precondition return
`EquivalenceOutcome::Partial` — never a fake GREEN.

---

## Change log

| Version | Change |
|---|---|
| v0 (WP-E4) | Initial published contract — 15 features, proven-to-execute by acceptance_e4 |


> **Note (2026-06-10):** The runner crate (`hugit-runner`) was transferred to `../corelink-runners` per CLAUDE.md. References in this doc are historical.
