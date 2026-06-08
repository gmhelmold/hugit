//! RunnerLease — frozen by decomposition §1, item 5.
//!
//! A scoped filesystem-access lease granted to a runner, with expiry and
//! lifecycle state tracking.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Lifecycle state of a runner lease.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum RunnerState {
    /// Lease is currently held and active.
    Held,
    /// Lease expired without explicit release.
    Expired,
    /// Runner crashed while holding the lease.
    Crashed,
    /// Lease was explicitly released.
    Released,
}

/// A runner lease granting scoped filesystem access to a runner
/// (decomposition §1, item 5).
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunnerLease {
    /// Unique lease identifier.
    pub lease_id: String,

    /// Ordered chain of principals (agent ids / user ids) that own this
    /// lease.
    pub principal_chain: Vec<String>,

    /// Set of filesystem paths this lease grants access to.
    pub path_set: Vec<String>,

    /// Unix epoch milliseconds at which this lease expires.
    pub expiry: u64,

    /// Network policy name / ref governing this runner's outbound access.
    pub net_policy: String,

    /// Temporary root directory allocated to this runner.
    pub tmp_root: String,

    /// Current lifecycle state of the lease.
    pub state: RunnerState,
}
