//! hugit-mirror — git history import subsystem (WP-E2a).
//!
//! Modules owned by WP-E2a:
//! - `history/` — commit/tree/blob import, byte-identity, change-event projection
//! - `lfs/`     — LFS object materialization
//! - `resume/`  — resumable import state + idempotency engine
//! - `auth`     — GitHub App installation-auth client (private repos)

pub mod auth;
pub mod history;
pub mod lfs;
pub mod resume;
