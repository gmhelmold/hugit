//! FenceManifest — frozen by decomposition §1, item 6.
//!
//! Claim-filtered sparse materialization manifest (whitepaper §9.1).
//! Sparse materialization IS the fence.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A (path, digest) pair for a materialised file entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaterializedEntry {
    /// Relative path of the materialized file.
    pub path: String,
    /// SHA-256 hex digest of the file content.
    pub digest: String,
}

/// Claim-filtered sparse materialization manifest (decomposition §1, item 6;
/// whitepaper §9.1).
///
/// Sparse materialization IS the fence. `deny_default = true` means paths not
/// listed are denied by default.
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FenceManifest {
    /// Allowed path set (explicit allowlist of paths the claim may access).
    pub path_set: Vec<String>,

    /// Whether unlisted paths are denied by default (MUST be `true` in all
    /// production manifests).
    pub deny_default: bool,

    /// Actually materialised (path, digest) pairs at runtime.
    pub materialized: Vec<MaterializedEntry>,
}
