//! Tournament — `hugit tournament -n N` fan-out (WP-D13).
//!
//! Produces N independent candidates for one intent, runs a D7 judge panel to
//! select per documented criteria, retains losers as addressable evidence, and
//! enforces a policy cap + C7 budget bound so the fan-out never generates
//! overage under a flat plan.
//!
//! ## Module layout
//!
//! - [`candidate`] — `Candidate` type, independence invariant, event records.
//! - [`selection`] — judge-panel selection per documented criteria; loser
//!   addressability.
//! - [`budget`] — policy cap + C7 budget-bounded fan-out; zero-overage
//!   enforcement.
//!
//! ## Consumed seams (never modified)
//!
//! - D7 panel dispatch: [`crate::verdict::panel_dispatch::dispatch`].
//! - C7 budget/fairness: [`hugit_queue::budget::BudgetManager`] (pub API only).
//! - Frozen contracts: [`hugit_contracts::IntentSidecar`],
//!   [`hugit_contracts::VerdictObject`], [`hugit_contracts::EventRecord`].

pub mod budget;
pub mod candidate;
pub mod selection;

// ── Convenience re-exports ─────────────────────────────────────────────────────

pub use budget::{
    FanOutOutcome, MAX_N_POLICY, UNITS_PER_CANDIDATE, assert_fairness_constants, attempt_fan_out,
};
pub use candidate::{Candidate, fan_out_event, produce_candidates, selection_event};
pub use selection::{SelectionError, SelectionResult, SelectionScore, resolve_loser, select};
