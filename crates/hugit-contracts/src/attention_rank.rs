//! AttentionRank — frozen by decomposition §1, item 12 (+).
//!
//! Composite ordering inputs for the attention queue.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Composite ordering inputs for the attention queue
/// (decomposition §1, item 12 (+)).
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttentionRank {
    /// Policy identifier / ref that produced this rank.
    pub policy: String,

    /// Estimated blast-radius score (higher = more files / systems affected).
    pub blast_radius: u64,

    /// Model confidence in the blast-radius estimate (0.0–1.0, stored as a
    /// fixed-point integer in basis points: 10000 = 1.0).
    pub confidence: u64,
}
