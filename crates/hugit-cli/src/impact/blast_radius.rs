//! Blast-radius computation for `hugit impact` (WP-D10 ②).
//!
//! Wraps the B3 [`hugit_checks::affected`] engine; callers get the GOLDEN
//! affected set and can assert set-equality against the fixture golden.

use std::collections::BTreeSet;

use hugit_checks::affected::{AffectedSet, BuildGraph};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Query / result shapes
// ---------------------------------------------------------------------------

/// A query to `hugit impact`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactQuery {
    /// Changed paths to evaluate blast-radius for.
    pub changed_paths: Vec<String>,
}

/// The full blast-radius result for an [`ImpactQuery`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactResult {
    /// The computed affected set (ordered).
    pub affected: BTreeSet<String>,
    /// Whether the result represents the full workspace (root edit / fail-open).
    pub is_full_set: bool,
    /// Reason the full set was returned, if any.
    pub full_set_reason: Option<String>,
}

impl ImpactResult {
    /// Build an `ImpactResult` from an [`AffectedSet`].
    pub fn from_affected(set: AffectedSet) -> Self {
        let full_set_reason = set.full_set_reason.as_ref().map(|r| format!("{r:?}"));
        ImpactResult {
            affected: set.packages,
            is_full_set: set.is_full_set,
            full_set_reason,
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from `compute_impact`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImpactError {
    /// The build graph was empty / nil — nothing to compute.
    EmptyGraph,
}

impl std::fmt::Display for ImpactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImpactError::EmptyGraph => write!(f, "build graph is empty"),
        }
    }
}

impl std::error::Error for ImpactError {}

// ---------------------------------------------------------------------------
// compute_impact
// ---------------------------------------------------------------------------

/// Compute the blast-radius of `query.changed_paths` over `graph`.
///
/// Returns the GOLDEN affected set (BFS over the reverse-dep graph, or full
/// set on root-manifest edit / unknown ecosystem).  Callers assert set-equality
/// against the fixture golden — not merely non-empty / non-error.
pub fn compute_impact(
    query: &ImpactQuery,
    graph: &BuildGraph,
) -> Result<ImpactResult, ImpactError> {
    if graph.packages.is_empty() {
        return Err(ImpactError::EmptyGraph);
    }
    let paths: Vec<&str> = query.changed_paths.iter().map(String::as_str).collect();
    let set = graph.affected(&paths);
    Ok(ImpactResult::from_affected(set))
}
