//! hugit-queue — the union-testing landing queue.
//!
//! [`core`] is the pure engine (WP-B4a): batching, union-tree fold, minimal-
//! failing-pair bisection, ordered idempotent landing, and the deterministic
//! state machine. The GitHub API surface (B4b) and budget (C7) are layered on
//! top in their own modules and do not exist yet.

pub mod core;
