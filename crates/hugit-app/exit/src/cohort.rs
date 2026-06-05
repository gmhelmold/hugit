//! ④ cohort/window guards.
//!
//! Guards: n=10 external teams, ≥3 weeks real use, evaluation window ANCHORED
//! to the first-10-paying-customers event and inside 90 days.
//! Outside any guard → `InsufficiendOrOutOfWindow`, NEVER a pass.

use serde::{Deserialize, Serialize};

/// Required minimum number of external teams in the cohort.
pub const MIN_TEAMS: usize = 10;
/// Required minimum weeks of real use per team.
pub const MIN_WEEKS: u32 = 3;
/// Maximum days from the first-10-paying-customers event anchor.
pub const MAX_WINDOW_DAYS: u64 = 90;

/// Input state describing the current cohort and evaluation window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohortState {
    /// Number of external teams in the cohort.
    pub team_count: usize,
    /// Minimum weeks of real use across all teams.
    pub min_weeks_real_use: u32,
    /// Days elapsed since the first-10-paying-customers anchor event.
    /// `None` means the anchor event has not occurred yet.
    pub days_since_anchor: Option<u64>,
}

/// Result of evaluating cohort guards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CohortGuardResult {
    /// All guards satisfied; cohort is valid for evaluation.
    Satisfied,
    /// One or more guards failed; reason encoded in the variant.
    InsufficientOrOutOfWindow(String),
}

impl CohortGuardResult {
    /// Returns `true` iff all guards are satisfied.
    pub fn is_satisfied(&self) -> bool {
        matches!(self, CohortGuardResult::Satisfied)
    }
}

/// Evaluate all cohort guards against the provided state.
///
/// Returns `Satisfied` only when ALL of the following hold:
/// 1. `team_count` ≥ 10
/// 2. `min_weeks_real_use` ≥ 3
/// 3. `days_since_anchor` is `Some(d)` where d ≤ 90
///
/// Any failure → `InsufficientOrOutOfWindow` — NEVER a pass.
pub fn evaluate_cohort_guards(state: &CohortState) -> CohortGuardResult {
    if state.team_count < MIN_TEAMS {
        return CohortGuardResult::InsufficientOrOutOfWindow(format!(
            "insufficient: only {} teams (need ≥{})",
            state.team_count, MIN_TEAMS
        ));
    }
    if state.min_weeks_real_use < MIN_WEEKS {
        return CohortGuardResult::InsufficientOrOutOfWindow(format!(
            "insufficient: only {} weeks real use (need ≥{})",
            state.min_weeks_real_use, MIN_WEEKS
        ));
    }
    match state.days_since_anchor {
        None => CohortGuardResult::InsufficientOrOutOfWindow(
            "out-of-window: first-10-paying-customers anchor event has not occurred".to_string(),
        ),
        Some(days) if days > MAX_WINDOW_DAYS => {
            CohortGuardResult::InsufficientOrOutOfWindow(format!(
                "out-of-window: {} days since anchor exceeds 90-day window",
                days
            ))
        }
        Some(_) => CohortGuardResult::Satisfied,
    }
}
