//! CheckDef — frozen by decomposition §1, item 1.
//!
//! A check-as-code definition. The `def_digest` is the canonical digest over
//! the definition body (the second axis of the memo key).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A check-as-code definition (decomposition §1, item 1).
///
/// Frozen by WP-00. The `def_digest` is the canonical digest over the
/// definition body; it forms the second axis of the CheckResult memo key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckDef {
    /// SHA-256 hex digest of the definition body (canonical, second axis of
    /// memo key).
    pub def_digest: String,

    /// The command to execute (argv[0] + args as a single shell string or
    /// structured invocation).
    pub command: String,

    /// Declared input paths / globs that affect the check.
    pub inputs: Vec<String>,

    /// Reference to the toolchain (e.g. a content-addressed toolchain digest
    /// or version string).
    pub toolchain_ref: String,

    /// Reference to an environment manifest (content-addressed blob ref).
    pub env_manifest: String,

    /// File glob patterns that scope materialization for this check.
    pub glob_set: Vec<String>,
}
