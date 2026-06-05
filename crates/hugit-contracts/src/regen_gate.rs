//! RegenGate — frozen by decomposition §1, item 15 (+).
//!
//! Regenerative-rebase promotion gate (decomposition §1, D12).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Regenerative-rebase promotion gate
/// (decomposition §1, item 15 (+); D12).
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegenGate {
    /// Opt-in scope for regenerative rebase (e.g. repo slug, org slug, or
    /// "*" for all).
    pub optin_scope: String,

    /// Whether this gate has been re-passed (promotion criterion met).
    pub repass: bool,

    /// Content-addressed ref to an independent verdict that confirmed the
    /// rebase is safe to promote.
    pub indep_verdict: String,
}
