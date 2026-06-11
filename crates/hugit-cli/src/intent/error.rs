//! The structured porcelain error for the `intent` verb (WP-PC2).
//!
//! **Error law (WB0 canonical, converged 2026-06-11).**  All intent errors —
//! and every other porcelain verb — share ONE shape:
//!
//! ```json
//! {"error":{"kind":"…","message":"…","fix":"…"}}
//! ```
//!
//! The fix-carrying field is `"fix"` (not `"suggested_fix"`) — matching the
//! canonical shape [`crate::campaign::output::CampaignError`] uses and the
//! audit / P2 spec (Tier-2 P2) calls for.  An optional `"detail"` object MAY
//! be folded in for structured extra context.  The envelope is always
//! `{"error":{…}}` — a single object on stdout, never a bare string, never a
//! fake success.
//!
//! [`PorcelainError::to_json`] is the single rendering point; all callers go
//! through it.

use super::store::StoreError;

/// A structured, fix-carrying porcelain error emitted as one JSON object.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PorcelainError {
    /// Machine-stable error class (e.g. `"invalid_argument"`, `"not_found"`).
    pub kind: String,
    /// Human-readable description of what went wrong.
    pub message: String,
    /// The concrete fix the caller (an agent) should apply.
    pub fix: String,
    /// Optional structured detail (folded into the `error` object when present).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl PorcelainError {
    /// Build a structured error from its three honest parts (no detail).
    pub fn new(
        kind: impl Into<String>,
        message: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            fix: fix.into(),
            detail: None,
        }
    }

    /// Attach optional structured detail (folded into the `error` object).
    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Lift a store-layer fault into a structured porcelain error.  An I/O or
    /// parse fault on the store is an operator problem (bad `--store` path or a
    /// corrupt/tampered file); a chain-broken store fails closed.
    pub fn from_store(e: StoreError) -> Self {
        let fix = match &e {
            StoreError::Read { .. } | StoreError::Write { .. } => {
                "check the --store path is writable and on an existing directory"
            }
            StoreError::Parse { .. } | StoreError::Rehydrate(_) | StoreError::ChainBroken(_) => {
                "the --store file is corrupt or tampered; point --store at a clean store"
            }
            StoreError::Serialize(_) => "internal: the store could not be serialised",
        };
        Self::new("store_error", e.to_string(), fix)
    }

    /// Serialise to the stable single-object JSON wire shape:
    /// `{"error":{"kind":…,"message":…,"fix":…}}` (plus optional `"detail"`).
    pub fn to_json(&self) -> String {
        let mut error = serde_json::json!({
            "kind":    self.kind,
            "message": self.message,
            "fix":     self.fix,
        });
        if let (Some(detail), Some(map)) = (&self.detail, error.as_object_mut()) {
            map.insert("detail".to_string(), detail.clone());
        }
        serde_json::json!({ "error": error }).to_string()
    }
}
