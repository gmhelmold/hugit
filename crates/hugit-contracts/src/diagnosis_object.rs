//! DiagnosisObject — frozen by decomposition §1, item 3.
//!
//! Bounded structured diagnosis (never a raw log dump) (B5④).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bounded structured diagnosis object (decomposition §1, item 3; B5④).
///
/// Never a raw log dump — size_bytes is asserted bounded at the application
/// layer.
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisObject {
    /// Content-addressed ref to the culprit artefact or commit.
    pub culprit_ref: String,

    /// Content-addressed ref to the diff between this state and the last
    /// known-green baseline.
    pub diff_vs_green_ref: String,

    /// Build/test targets suspected of being the root cause.
    pub suspect_targets: Vec<String>,

    /// Ordered sequence of tree refs representing the bisect path from the
    /// last known-green tree to the culprit.
    pub bisect_path: Vec<String>,

    /// Serialised byte size of this diagnosis object (asserted bounded by
    /// the application layer).
    pub size_bytes: u64,
}
