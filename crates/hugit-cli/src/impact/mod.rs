//! `hugit impact` — blast-radius query + ground-truth export (WP-D10 ②③).
//!
//! item ②: `impact <path|change>` returns the GOLDEN affected set on a KNOWN
//! build graph — asserted via set-equality against the golden, not merely
//! non-empty.
//!
//! item ③: `impact` EXPORTS the affected-set as the ground truth consumed by
//! D7 verdict panels (one-directional seam: D10 exports, D7 consumes).

pub mod blast_radius;
pub mod ground_truth_export;

pub use blast_radius::{ImpactError, ImpactQuery, ImpactResult, compute_impact};
pub use ground_truth_export::{GroundTruthRecord, export_ground_truth};
