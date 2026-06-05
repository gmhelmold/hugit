//! CheckResult — frozen by decomposition §1, item 2.
//!
//! Memo of `check(tree_hash, def_digest, toolchain_digest)`. Bit-identical
//! across phases (X9①).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A (path, content-digest) pair in the artifacts list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    /// Relative output path of the artifact.
    pub path: String,
    /// SHA-256 hex digest of the artifact content.
    pub digest: String,
}

/// Memoised result of `check(tree_root, def_digest, toolchain_digest)`
/// (decomposition §1, item 2; X9①).
///
/// Canonical serialization is deterministic (sorted maps, fixed field order)
/// so the phase-B memo equals the phase-D evidence byte-for-byte.
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    /// Memoisation key.
    ///
    /// FROZEN FORMULA (doc-only, implementation in B2a/D1a):
    /// `memo_key = SHA-256(len(tree_root) ‖ tree_root ‖ len(def_digest) ‖
    /// def_digest ‖ len(toolchain_digest) ‖ toolchain_digest)` where each
    /// length prefix is a 4-byte big-endian u32 and the inputs are UTF-8
    /// bytes of the respective hex strings.
    pub memo_key: String,

    /// Merkle tree root hash of the workspace snapshot used (hex).
    pub tree_hash: String,

    /// SHA-256 hex digest of the CheckDef body.
    pub def_digest: String,

    /// Content-addressed digest of the toolchain used.
    pub toolchain_digest: String,

    /// Process exit code (0 = success).
    pub exit: i32,

    /// Output artifacts: (path, digest) pairs.
    pub artifacts: Vec<Artifact>,

    /// Content-addressed ref to captured stdout blob.
    pub stdout_ref: String,

    /// Content-addressed ref to captured stderr blob.
    pub stderr_ref: String,

    /// Wall-clock duration of the check in milliseconds.
    pub duration_ms: u64,

    /// Reference to the runner that executed this check.
    pub runner_ref: String,

    /// Unix epoch milliseconds when this result was produced.
    pub produced_at: u64,
}
