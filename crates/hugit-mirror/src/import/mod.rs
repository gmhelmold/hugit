//! hugit-mirror import — git history import (WP-E2a) + PR/issue import (WP-E2b).
//!
//! Modules:
//! - `history/` — commit/tree/blob import, byte-identity, change-event projection (E2a)
//! - `lfs/`     — LFS object materialization (E2a)
//! - `resume/`  — resumable import state + idempotency engine (E2a)
//! - `auth`     — GitHub App installation-auth client, private repos (E2a)
//! - `prissue/` — PR/issue → proposed intents, fidelity contract (E2b)

pub mod auth;
pub mod history;
pub mod lfs;
pub mod prissue;
pub mod resume;
