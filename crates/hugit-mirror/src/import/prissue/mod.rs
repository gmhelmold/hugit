//! PR/issue import — proposed, non-authoritative intent projection.
//!
//! Imports GitHub PR and issue metadata into hugit as **proposed,
//! non-authoritative** intents with per-element provenance.
//!
//! # Fidelity contract
//!
//! **Preserved (with per-element provenance)**:
//! - body
//! - comment/review threads
//! - state
//! - labels
//! - cross-refs (resolved when both ends imported; dangling → explicit residual)
//!
//! **Not imported (explicitly enumerated, no silent drop)**: see
//! [`model::NON_IMPORTED`].
//!
//! # Proposed / non-authoritative
//!
//! Every imported PR/issue intent carries `authoritative = false` and is
//! state-flagged proposed: it never gates, blocks, or lands anything on import
//! (consistent with B6④ sidecar-non-authoritative and E2a⑤ boundary).
//!
//! # Idempotency
//!
//! The `intent_id` is derived from the source URL: re-importing the same
//! PR/issue with unchanged metadata is a no-op; changed metadata triggers an
//! incremental re-sync under the same `intent_id`; no duplicates are produced.
//!
//! # Boundary (E2a⑤)
//!
//! This module only mints intents from PR/issue **metadata**. Bare commits
//! MUST NOT produce intents — that law is E2a's and is never crossed here.

pub mod import;
pub mod model;

pub use import::{PrIssueImportError, import_prissue, import_prissue_batch};
pub use model::{
    CrossRef, ElementProvenance, ImportedComment, ImportedPrIssue, NON_IMPORTED, PrIssueState,
    ProposedIntent, ProposedIntentProvenance, SourceKind,
};
