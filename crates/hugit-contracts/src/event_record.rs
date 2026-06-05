//! EventRecord — frozen by decomposition §1, item 7.
//!
//! One append-only hash-chained event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One append-only hash-chained event record (decomposition §1, item 7).
///
/// Hash-chain formula (FROZEN, doc-only — implementation in B2a/D1a):
/// `this_hash = H(prev_hash ‖ kind ‖ principal_chain ‖ payload ‖ seq)`
/// where H = SHA-256, ‖ = concatenation of length-prefixed UTF-8 fields
/// (4-byte big-endian u32 length prefix per field), seq is encoded as
/// 8-byte big-endian u64, and the genesis `prev_hash` = 64 hex zeros.
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventRecord {
    /// Monotonically increasing sequence number within the event log.
    pub seq: u64,

    /// SHA-256 hex digest of the preceding event (64 hex zeros for genesis).
    pub prev_hash: String,

    /// SHA-256 hex digest of this event.
    ///
    /// FROZEN FORMULA: `H(prev_hash ‖ kind ‖ principal_chain ‖ payload ‖ seq)`.
    pub this_hash: String,

    /// Event kind / type discriminator string.
    pub kind: String,

    /// Ordered chain of principals that produced this event.
    pub principal_chain: Vec<String>,

    /// Opaque JSON payload for this event (serialised as a string to keep
    /// the envelope schema stable across payload evolution).
    pub payload: String,

    /// Unix epoch milliseconds when this event was recorded.
    pub recorded_at: u64,
}
