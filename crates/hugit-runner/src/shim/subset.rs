//! Published supported-subset contract for the Actions-YAML shim.
//!
//! ## Contract guarantee (①)
//! Every feature listed in [`SUPPORTED_SUBSET`] is **proven-to-execute** by a
//! passing fixture in `acceptance_e4.rs`. "Supported" is not merely documented
//! — it means the shim can parse, plan, and execute a workflow using that
//! feature, with equivalent observable outcomes to real GitHub Actions.
//!
//! ## Boundary guarantee (②)
//! Any workflow construct NOT represented in [`SUPPORTED_SUBSET`] is
//! out-of-contract and will produce an explicit [`OutOfContractReport`] rather
//! than a silent skip. See `report.rs`.
//!
//! ## Published location
//! The human-readable version of this contract lives at
//! `docs/shim/supported-subset.md`. That document is generated from this
//! source of truth.

use std::fmt;

/// A single supported feature in the Actions-YAML shim's published contract.
///
/// Every variant here is:
/// - listed in `docs/shim/supported-subset.md`,
/// - covered by a fixture that proves execution, not just parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubsetFeature {
    // ── Workflow-level structure ──────────────────────────────────────────────
    /// `on: push` trigger (single-event, no filter).
    OnPush,
    /// `on: pull_request` trigger (single-event, no filter).
    OnPullRequest,
    /// `on: workflow_dispatch` trigger (no inputs).
    OnWorkflowDispatch,

    // ── Job-level structure ───────────────────────────────────────────────────
    /// A single job with `runs-on: ubuntu-latest`.
    SingleJobUbuntu,
    /// Job-level `env:` map (static string values only).
    JobEnvStatic,
    /// Job `if:` condition using the literal expression `${{ always() }}`.
    JobIfAlways,

    // ── Step-level structure ──────────────────────────────────────────────────
    /// `run:` step with a single-line shell command.
    RunStepSingleLine,
    /// `run:` step with a multi-line shell command (heredoc / `|`).
    RunStepMultiLine,
    /// Step-level `env:` map (static string values only).
    StepEnvStatic,
    /// Step `name:` field (display label).
    StepName,
    /// Step `if:` condition using the literal expression `${{ always() }}`.
    StepIfAlways,
    /// Step `continue-on-error: true`.
    StepContinueOnError,

    // ── Secret resolution (via broker) ────────────────────────────────────────
    /// `${{ secrets.NAME }}` expression resolved via the C5 secrets broker;
    /// raw material never enters environment or logs.
    SecretExpression,

    // ── Artifacts ─────────────────────────────────────────────────────────────
    /// `actions/upload-artifact@v3` with a `name:` and `path:` that refers to
    /// a file written by a prior `run:` step.
    UploadArtifactV3,
    /// `actions/download-artifact@v3` with a matching `name:` to a
    /// same-workflow upload.
    DownloadArtifactV3,
}

impl fmt::Display for SubsetFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::OnPush => "on:push trigger",
            Self::OnPullRequest => "on:pull_request trigger",
            Self::OnWorkflowDispatch => "on:workflow_dispatch trigger",
            Self::SingleJobUbuntu => "single job (runs-on: ubuntu-latest)",
            Self::JobEnvStatic => "job-level env (static strings)",
            Self::JobIfAlways => "job if: always()",
            Self::RunStepSingleLine => "run step (single-line)",
            Self::RunStepMultiLine => "run step (multi-line)",
            Self::StepEnvStatic => "step-level env (static strings)",
            Self::StepName => "step name",
            Self::StepIfAlways => "step if: always()",
            Self::StepContinueOnError => "step continue-on-error: true",
            Self::SecretExpression => "secrets.NAME expression (via broker)",
            Self::UploadArtifactV3 => "actions/upload-artifact@v3",
            Self::DownloadArtifactV3 => "actions/download-artifact@v3",
        };
        f.write_str(s)
    }
}

/// The published supported-subset contract: every feature the shim can
/// execute, proven by a passing fixture.
///
/// This list is the machine-readable source of truth; `docs/shim/supported-
/// subset.md` mirrors it in human-readable form.
pub const SUPPORTED_SUBSET: &[SubsetFeature] = &[
    SubsetFeature::OnPush,
    SubsetFeature::OnPullRequest,
    SubsetFeature::OnWorkflowDispatch,
    SubsetFeature::SingleJobUbuntu,
    SubsetFeature::JobEnvStatic,
    SubsetFeature::JobIfAlways,
    SubsetFeature::RunStepSingleLine,
    SubsetFeature::RunStepMultiLine,
    SubsetFeature::StepEnvStatic,
    SubsetFeature::StepName,
    SubsetFeature::StepIfAlways,
    SubsetFeature::StepContinueOnError,
    SubsetFeature::SecretExpression,
    SubsetFeature::UploadArtifactV3,
    SubsetFeature::DownloadArtifactV3,
];

/// Returns `true` if `feature` is in the published supported subset (and
/// therefore proven-to-execute by a fixture).
pub fn is_supported(feature: SubsetFeature) -> bool {
    SUPPORTED_SUBSET.contains(&feature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_subset_is_non_empty() {
        assert!(!SUPPORTED_SUBSET.is_empty());
    }

    #[test]
    fn is_supported_returns_true_for_all_listed() {
        for &f in SUPPORTED_SUBSET {
            assert!(is_supported(f), "feature {:?} should be supported", f);
        }
    }

    #[test]
    fn subset_features_have_display_strings() {
        for &f in SUPPORTED_SUBSET {
            let s = f.to_string();
            assert!(!s.is_empty(), "display string for {:?} is empty", f);
        }
    }
}
