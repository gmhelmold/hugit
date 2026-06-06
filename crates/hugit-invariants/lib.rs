//! hugit-invariants — Squad-X cross-cutting invariant test crate.
//!
//! Root library entry point. Each WP owns a subtree; this file wires them
//! together additively. No subtree is modified when a new WP is added.
//!
//! # WP layout
//! - `x4/` — supply-chain invariants: pinned images, verified deps, fail-closed (WP-X4).
//! - `x5/` — namespace-law invariants: no git-verb shadow, ref-namespace non-collision (WP-X5).

// ── WP-X4: supply-chain invariants ───────────────────────────────────────────
// Re-export x4's public surface so existing consumers (acceptance_x4.rs) keep
// their `hugit_invariants::pin::…` import paths intact.
#[path = "x4/lib.rs"]
mod x4_root;

pub use x4_root::pin;

// ── WP-X5: namespace-law invariants ──────────────────────────────────────────
#[path = "x5/lib.rs"]
pub mod x5;
