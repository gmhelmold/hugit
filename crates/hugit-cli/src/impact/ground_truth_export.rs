//! Ground-truth export for verdict panels (WP-D10 ③).
//!
//! `impact` EXPORTS the affected-set as the ground truth consumed by D7 verdict
//! panels.  D10 owns this export module; D7 consumes it.  The seam is
//! one-directional (D10 → D7); D10 never imports from D7.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::impact::blast_radius::ImpactResult;

// ---------------------------------------------------------------------------
// Ground-truth record (exported to verdict panels)
// ---------------------------------------------------------------------------

/// The ground-truth record exported to a verdict panel.
///
/// Verdict panels (D7) consume this to cross-check their own verdict against
/// the blast-radius ground truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundTruthRecord {
    /// The event / change identifier this ground truth is for.
    pub change_id: String,
    /// Affected package set (ordered).
    pub affected: BTreeSet<String>,
    /// Whether the result is a full-workspace set.
    pub is_full_set: bool,
    /// Serialized reason the full set was returned, if applicable.
    pub full_set_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// export_ground_truth
// ---------------------------------------------------------------------------

/// Export an [`ImpactResult`] as a [`GroundTruthRecord`] for verdict panels.
///
/// The `change_id` is the stable identifier of the change/event being evaluated
/// (e.g., intent_id, PR number, or a content hash of the diff).
pub fn export_ground_truth(
    change_id: impl Into<String>,
    result: &ImpactResult,
) -> GroundTruthRecord {
    GroundTruthRecord {
        change_id: change_id.into(),
        affected: result.affected.clone(),
        is_full_set: result.is_full_set,
        full_set_reason: result.full_set_reason.clone(),
    }
}
