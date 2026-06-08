//! VerdictObject — frozen by decomposition §1, item 8.
//!
//! Verdict produced by a review lens: outcome, claims checked, and evidence refs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The verdict outcome of a review or gate check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Verdict {
    /// The PR/change is approved for landing.
    Approve,
    /// The PR/change requires fixes before landing.
    FixFirst,
    /// The PR/change is rejected.
    Reject,
}

/// Verdict object from a review lens (decomposition §1, item 8).
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerdictObject {
    /// The intent_id of the PR / change being reviewed.
    pub intent: String,

    /// Merkle tree hash of the workspace snapshot reviewed.
    pub tree_hash: String,

    /// Identifier of the review lens / policy applied.
    pub lens: String,

    /// Model identifier used to produce this verdict.
    pub model: String,

    /// SHA-256 hex digest of the prompt used to produce this verdict.
    pub prompt_digest: String,

    /// The verdict outcome.
    pub verdict: Verdict,

    /// List of claims checked during the review.
    pub claims_checked: Vec<String>,

    /// Content-addressed refs to evidence blobs supporting the verdict.
    pub evidence_refs: Vec<String>,
}
