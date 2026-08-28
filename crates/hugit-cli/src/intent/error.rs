//! The structured porcelain error for the `intent` verb (WP-PC2).
//!
//! **Error law (WB0 canonical, converged 2026-06-11; flat-context uniformity
//! WF).**  All intent errors — and every other porcelain verb — share ONE
//! shape:
//!
//! ```json
//! {"error":{"kind":"…","message":"…","fix":"…", …context}}
//! ```
//!
//! The fix-carrying field is `"fix"` (not `"suggested_fix"`) — matching the
//! canonical shape [`crate::porcelain::PorcelainError`] and the sibling
//! [`crate::campaign::output::CampaignError`].  Extra structured context is
//! folded **FLAT** into the `error` object (never nested under a `detail`
//! sub-object — WF error-shape uniformity), so ONE parser (`error.<key>`)
//! works across every verb.  The envelope is always `{"error":{…}}` — a single
//! object on stdout, never a bare string, never a fake success.
//!
//! [`PorcelainError::to_json`] is the single rendering point; all callers go
//! through it.

use super::store::StoreError;

/// A structured, fix-carrying porcelain error emitted as one JSON object — the
/// SAME canonical shape as [`crate::porcelain::PorcelainError`] (context folded
/// flat, never under `detail`). PR-5: `internal: bool` distinguishes a
/// bug-class internal fault (exit 1) from a user/domain error (exit 2). Hand-
/// rolled `PartialEq` ignores it (the wire `to_json` is identical for both;
/// the flag is a routing decision).
#[derive(Debug, Clone)]
pub struct PorcelainError {
    /// Machine-stable error class (e.g. `"invalid_argument"`, `"not_found"`).
    pub kind: String,
    /// Human-readable description of what went wrong.
    pub message: String,
    /// The concrete fix the caller (an agent) should apply.
    pub fix: String,
    /// Extra structured context, folded FLAT into the `error` object on render
    /// — matching the canonical [`crate::porcelain::PorcelainError`]. Insertion
    /// order preserved.
    pub context: Vec<(&'static str, serde_json::Value)>,
    /// PR-5: bug-class internal fault (exit 1) flag. NOT in `PartialEq`.
    pub internal: bool,
}

impl PartialEq for PorcelainError {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.message == other.message
            && self.fix == other.fix
            && self.context == other.context
    }
}

impl PorcelainError {
    /// Build a structured error from its three honest parts (no context).
    pub fn new(
        kind: impl Into<String>,
        message: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            fix: fix.into(),
            context: Vec::new(),
            internal: false,
        }
    }

    /// An internal fault (a bug, not bad input). `kind = "internal"`, exit 1.
    /// Matches the porcelain signature (single `message` arg) — `fix` is set
    /// to the canonical internal-fault remediation.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: "internal".into(),
            message: message.into(),
            fix: "this is an internal hugit bug; report it with the command + inputs".into(),
            context: Vec::new(),
            internal: true,
        }
    }

    /// True iff this is a bug-class internal fault (exit 1).
    pub fn is_internal(&self) -> bool {
        self.internal
    }

    /// Fold a flat structured context key into the `error` object — the ONE
    /// canonical placement (flat, never under `detail`), matching
    /// [`crate::porcelain::PorcelainError::with_context`].
    pub fn with_context(mut self, key: &'static str, value: serde_json::Value) -> Self {
        self.context.push((key, value));
        self
    }

    /// Lift a store-layer fault into a structured porcelain error.  An I/O or
    /// parse fault on the store is an operator problem (bad `--store` path or a
    /// corrupt/tampered file); a chain-broken store fails closed; a MISSING
    /// store on a read-only query is `store_not_found`/exit-2 (matching the
    /// sibling `--log` `log_not_found` contract — never a silent empty store).
    pub fn from_store(e: StoreError) -> Self {
        let fix = match &e {
            StoreError::NotFound { .. } => {
                "create the store first (`hugit intent new --store <path> …` \
                 bootstraps it) or point --store at an existing intent store"
            }
            StoreError::Read { .. } | StoreError::Write { .. } => {
                "check the --store path is writable and on an existing directory"
            }
            StoreError::Parse { .. } | StoreError::Rehydrate(_) | StoreError::ChainBroken(_) => {
                "the --store file is corrupt or tampered; point --store at a clean store"
            }
            StoreError::Serialize(_) => "internal: the store could not be serialised",
            // The advisory lock is held by another live `hugit` verb (WP-WC1) —
            // retry-able, never a clobber.
            StoreError::Busy { .. } => {
                "another `hugit` process holds the store lock; retry once it \
                 releases (a stale lock is auto-reclaimed after a short window)"
            }
        };
        // Distinct machine-matchable kinds: a missing store, a busy lock (a
        // transient, retry-able condition), and a generic store fault.
        let kind = match &e {
            StoreError::NotFound { .. } => "store_not_found",
            StoreError::Busy { .. } => "store_busy",
            _ => "store_error",
        };
        // A missing store carries its path as FLAT context (the canonical shape,
        // matching `log_not_found`); other faults carry the path in the message.
        let mut err = Self::new(kind, e.to_string(), fix);
        if let StoreError::NotFound { path } = &e {
            err = err.with_context("path", serde_json::json!(path));
        }
        err
    }

    /// Serialise to the canonical single-object JSON wire shape:
    /// `{"error":{"kind":…,"message":…,"fix":…, …context}}` — `kind` FIRST
    /// (agents stream-match on it, PS-15 F-2), context folded FLAT, identical
    /// key order to every sibling porcelain verb (the shared
    /// [`crate::porcelain::ordered_error_object`] builder).
    pub fn to_json(&self) -> String {
        let inner = crate::porcelain::ordered_error_object(
            &self.kind,
            &self.message,
            &self.fix,
            &self.context,
        );
        format!(r#"{{"error":{inner}}}"#)
    }

    /// PR-5: exit code under the one exit-code law. `internal:true` (a bug-class
    /// fault) returns `INTERNAL_FAULT_EXIT` (1); domain errors return
    /// `PORCELAIN_ERROR_EXIT` (2).
    pub fn exit_code(&self) -> std::process::ExitCode {
        if self.internal {
            std::process::ExitCode::from(crate::porcelain::INTERNAL_FAULT_EXIT)
        } else {
            std::process::ExitCode::from(crate::porcelain::PORCELAIN_ERROR_EXIT)
        }
    }
}
