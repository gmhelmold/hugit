//! ShadowPolicy — frozen by decomposition §1, item 11 (+).
//!
//! Shadow-run policy controlling cadence, compute budget, and opt-in scope.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Shadow-run policy controlling cadence, budget, and opt-in scope
/// (decomposition §1, item 11 (+)).
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShadowPolicy {
    /// Cadence descriptor (e.g. "every_push", "nightly", or a cron
    /// expression string).
    pub cadence: String,

    /// Maximum compute budget for shadow runs (in milliseconds of wall-clock
    /// runner time per policy period).
    pub budget: u64,

    /// Opt-in scope identifier (e.g. repo slug, org slug, or "*" for all).
    pub optin: String,
}
