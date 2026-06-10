//! The structured porcelain error for the `intent` verb (WP-PC2).
//!
//! Design law: errors are a single JSON object on stdout carrying an honest
//! `kind`, a human `message`, AND a `suggested_fix` an agent can act on — never
//! a bare string, never a fake success. Shape (stable wire contract):
//!
//! ```json
//! {"error":{"kind":"…","message":"…","suggested_fix":"…"}}
//! ```
//!
//! This lives in the `intent` module (PC2 owns it) rather than the shared
//! `porcelain` helper, which carries only the PC0 NOT-IMPLEMENTED stub shape;
//! the richer fix-carrying envelope is PC2's surface.

use super::store::StoreError;

/// A structured, fix-carrying porcelain error emitted as one JSON object.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PorcelainError {
    /// Machine-stable error class (e.g. `"invalid_argument"`, `"not_found"`).
    pub kind: String,
    /// Human-readable description of what went wrong.
    pub message: String,
    /// The concrete fix the caller (an agent) should apply.
    pub suggested_fix: String,
}

impl PorcelainError {
    /// Build a structured error from its three honest parts.
    pub fn new(
        kind: impl Into<String>,
        message: impl Into<String>,
        suggested_fix: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            suggested_fix: suggested_fix.into(),
        }
    }

    /// Lift a store-layer fault into a structured porcelain error. An I/O or
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

    /// Serialise to the stable single-object JSON wire shape.
    pub fn to_json(&self) -> String {
        serde_json::json!({ "error": self }).to_string()
    }
}
