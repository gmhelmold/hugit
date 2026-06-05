//! Core engine of the union-testing landing queue (WP-B4a).
//!
//! Pure, GitHub-free, testable in isolation. Owns items ①②⑤ of B4:
//! * ① minimal failing pair — bisect a red union, exclude + name the pair
//!   ([`union::evaluate_union`]).
//! * ② disjoint greens land with 0 re-runs — disjointness over affected-sets
//!   and AC-hit accounting ([`union::disjoint_lanes`], [`union::CheckSource`]).
//! * ⑤ ordered idempotent landing — the state machine has no out-of-order
//!   landing edge ([`state::transition`], [`order::land_in_order`]).
//!
//! The GitHub merge API, branch protection, and crash-idempotency kill-test
//! (item ④) are B4b, built on top of these pure transitions.

pub mod affected;
pub mod batch;
pub mod order;
pub mod state;
pub mod union;

pub use affected::AffectedSet;
pub use batch::{Batch, BatchEntry};
pub use order::{LandingError, LandingStep, land_in_order, landed_in_order};
pub use state::{EntryState, TransitionError, UnionOutcome, transition};
pub use union::{
    CheckSource, MemoCheck, UnionEvaluation, UnionVerdict, disjoint_lanes, evaluate_union,
};
