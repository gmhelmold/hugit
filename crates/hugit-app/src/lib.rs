//! hugit-app — GitHub App skeleton (WP-B1).
//!
//! Provides: webhook auth + ingest (X-Hub-Signature-256 HMAC verification),
//! PR-event persistence adapter, Checks-API write-back client, least-privilege
//! App manifest, and lifecycle handling (install/uninstall).
//!
//! Modules reserved for other WPs (DO NOT author here):
//!   - `sidecar/` — B6
//!   - `ui/`      — B7

pub mod checks;
pub mod manifest;
pub mod persistence;
pub mod revocation;
pub mod webhook;

pub use checks::{
    ChecksClient, ChecksClientError, ChecksTransport, UreqChecksTransport,
    conclusion_to_status_state,
};
pub use manifest::APP_MANIFEST;
pub use persistence::{PersistenceAdapter, PersistenceError};
pub use revocation::{RevocationError, RevocationLedger};
pub use webhook::{RevokeOutcome, WebhookError, WebhookProcessor};
