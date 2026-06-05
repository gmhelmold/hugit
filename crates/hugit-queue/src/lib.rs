//! hugit-queue — the union-testing landing queue.
//!
//! [`core`] is the pure engine (WP-B4a): batching, union-tree fold, minimal-
//! failing-pair bisection, ordered idempotent landing, and the deterministic
//! state machine. The GitHub API surface (B4b) and budget (C7) are layered on
//! top in their own modules. [`github`] is B4b's API surface (merge driver,
//! force-push recompute, branch-protection holds, crash-recovery); `budget` is
//! C7.

#[path = "../budget/mod.rs"]
pub mod budget;
pub mod core;
pub mod github;
