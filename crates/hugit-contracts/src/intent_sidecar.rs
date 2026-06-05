//! IntentSidecar — frozen by decomposition §1, item 4.
//!
//! PR-attached intent metadata. Non-authoritative; never gates landing (B6④).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// PR-attached intent metadata sidecar (decomposition §1, item 4; B6④).
///
/// Non-authoritative (`authoritative` is always `false`); never gates
/// landing.
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntentSidecar {
    /// Stable unique identifier for this intent (UUID or content hash).
    pub intent_id: String,

    /// Human-readable charter / description of what the PR intends to do.
    pub charter: String,

    /// Acceptance criteria — a list of acceptance item descriptions.
    pub acceptance: Vec<String>,

    /// Content-addressed ref to the full context blob (may include diff,
    /// prompt, rationale).
    pub context_ref: String,

    /// Whether this sidecar is authoritative. MUST always be `false`;
    /// non-authoritative by design.
    pub authoritative: bool,
}
