//! QueueApi — frozen by decomposition §1, item 9.
//!
//! Landing-queue compound surface. Root struct aggregating sub-structs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A single entry in the landable queue (PR / change ready to land).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LandableEntry {
    /// Unique identifier for this landable item.
    pub item_id: String,
    /// intent_id of the associated PR sidecar.
    pub intent_id: String,
    /// Merkle tree hash of the change's workspace snapshot.
    pub tree_hash: String,
    /// Position in the landing queue.
    pub order_index: u64,
}

/// Union merge result for a batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnionResult {
    /// Batch identifier.
    pub batch_id: String,
    /// Merkle tree hash of the union-merged workspace.
    pub union_tree: String,
    /// Whether the union merge succeeded without conflicts.
    pub conflict_free: bool,
}

/// The minimal failing pair in a batch (two items whose union fails).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MinimalFailingPair {
    /// First item of the failing pair.
    pub item_a: String,
    /// Second item of the failing pair.
    pub item_b: String,
}

/// Seal record for a completed landing batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchSeal {
    /// Batch identifier.
    pub batch_id: String,
    /// Merkle tree hash of the sealed (committed) tree.
    pub union_tree: String,
    /// Queue position / ordering index of this batch.
    pub order_index: u64,
    /// Current state of the batch.
    pub state: String,
    /// The minimal failing pair if the batch is in a failed state (absent
    /// when the batch succeeded).
    pub minimal_failing_pair: Option<MinimalFailingPair>,
}

/// Landing-queue API surface — compound root type aggregating all landing-
/// queue sub-types (decomposition §1, item 9).
///
/// The committed JSON Schema is the schema of this root type.
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueueApi {
    /// Items currently landable (enqueue request body).
    pub landable: Vec<LandableEntry>,
    /// Batch identifier for the current or most recent batch.
    pub batch_id: String,
    /// Union merge result for the current batch.
    pub union_result: UnionResult,
    /// Seal record for the current or most recent batch.
    pub seal: BatchSeal,
}
