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
    /// Memoisation key (64-char lowercase hex).
    ///
    /// # FROZEN FORMULA (BYTE-EXACT, single-sourced)
    ///
    /// Computed only by `hugit_refstore::compute_memo_key` — import and call it,
    /// never re-transcribe.
    ///
    /// ```text
    /// memo_key = lower_hex( SHA-256(
    ///     LP(tree_hash) ‖ LP(def_digest) ‖ LP(toolchain_digest)
    /// ) )
    /// ```
    ///
    /// where `LP(s)` = `u32_be(byte_len(s)) ‖ utf8_bytes(s)` (4-byte big-endian
    /// `u32` length prefix then the raw UTF-8 bytes). Each input is the
    /// **lowercase-hex** UTF-8 string of the respective digest. The three axes
    /// appear in struct field order: `tree_hash`, then `def_digest`, then
    /// `toolchain_digest`. Output is 64-char lowercase hex.
    pub memo_key: String,

    /// Merkle tree root hash of the workspace snapshot used (lowercase hex).
    /// First memo axis (see `memo_key`).
    pub tree_hash: String,

    /// SHA-256 hex digest of the CheckDef body (lowercase hex). Second memo axis.
    pub def_digest: String,

    /// Content-addressed digest of the toolchain used (lowercase hex). Third
    /// memo axis.
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
