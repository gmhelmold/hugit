//! Budget-bounded fan-out for `hugit tournament -n N` (WP-D13 ④).
//!
//! N is POLICY-CAPPED: an N-way tournament respects per-tenant caps and C7
//! fairness (consumed read-only from `hugit_queue::budget`). Under a flat plan
//! the cost amplifier generates ZERO overage — the fan-out is refused/capped
//! before it dispatches, never after.
//!
//! ## Budget contract (consumed, not owned)
//!
//! D13 consumes the C7 `BudgetManager` surface through its published API only:
//! - [`hugit_queue::budget::BudgetManager::try_dispatch`] — attempt to consume
//!   N units (one per candidate) before dispatching.
//! - [`hugit_queue::budget::BudgetManager::budget`] — read remaining budget.
//! - [`hugit_queue::budget::BudgetStatus`] — discriminate available vs. exhausted.
//! - [`hugit_queue::budget::P95_WAIT_BOUND_MS`], [`hugit_queue::budget::THROUGHPUT_FLOOR`] — stated fairness constants.
//!
//! ## Zero-overage guarantee
//!
//! If `n > policy_cap` → the request is rejected with `BudgetError::NExceedsCap`
//! before any candidate is produced. If `n > remaining_budget` → rejected with
//! `BudgetError::BudgetInsufficient`. In both cases, ZERO candidates are produced
//! and the overage charge is zero (nothing ran).

use hugit_queue::budget::{
    BudgetEvent, BudgetManager, BudgetStatus, P95_WAIT_BOUND_MS, THROUGHPUT_FLOOR,
};

// Re-export the fairness constants so acceptance tests can reference them
// without importing hugit-queue directly.
pub use hugit_queue::budget::{
    P95_WAIT_BOUND_MS as FAIRNESS_P95_WAIT_BOUND_MS, THROUGHPUT_FLOOR as FAIRNESS_THROUGHPUT_FLOOR,
};

/// Maximum N allowed by policy (the global policy cap).
///
/// This is the hard ceiling on the fan-out multiplier. An N-way tournament with
/// N > `MAX_N_POLICY` is always refused — even if a tenant has budget — because
/// the per-request amplification would violate the flat-plan cost contract.
pub const MAX_N_POLICY: usize = 16;

/// The budget cost of one candidate (one unit per candidate).
pub const UNITS_PER_CANDIDATE: u64 = 1;

/// The outcome of a budget-bounded fan-out attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FanOutOutcome {
    /// Fan-out approved: N candidates may be produced. Remaining budget after
    /// this approval is included for transparency.
    Approved { n: usize, remaining_after: u64 },
    /// Fan-out refused: N exceeded the policy cap. Zero candidates produced,
    /// zero charge.
    NExceedsCap { requested: usize, cap: usize },
    /// Fan-out refused: N exceeded available budget. Zero candidates produced,
    /// zero charge.
    BudgetInsufficient { requested: u64, remaining: u64 },
}

impl FanOutOutcome {
    /// True if the fan-out was approved.
    pub fn is_approved(&self) -> bool {
        matches!(self, FanOutOutcome::Approved { .. })
    }

    /// True if refused due to policy cap.
    pub fn is_cap_refused(&self) -> bool {
        matches!(self, FanOutOutcome::NExceedsCap { .. })
    }

    /// True if refused due to budget exhaustion.
    pub fn is_budget_refused(&self) -> bool {
        matches!(self, FanOutOutcome::BudgetInsufficient { .. })
    }
}

/// Attempt a budget-bounded fan-out for `n` candidates for `tenant_id`.
///
/// Checks policy cap first (before touching the budget manager), then attempts
/// to consume `n * UNITS_PER_CANDIDATE` from the tenant's C7 budget.
///
/// Returns `FanOutOutcome::Approved` only when both checks pass. In all refusal
/// cases, no units are consumed and no candidates will be produced (zero
/// overage under a flat plan).
///
/// `events` is populated with any `BudgetExhausted` events emitted by the C7
/// manager.
pub fn attempt_fan_out(
    manager: &mut BudgetManager,
    tenant_id: &str,
    n: usize,
    enqueue_tick: u64,
    events: &mut Vec<BudgetEvent>,
) -> FanOutOutcome {
    // ── ① Policy cap check (pre-budget, fail-fast) ────────────────────────────
    if n > MAX_N_POLICY {
        return FanOutOutcome::NExceedsCap {
            requested: n,
            cap: MAX_N_POLICY,
        };
    }

    // ── ② Budget check (consume via C7 surface) ───────────────────────────────
    let units = (n as u64) * UNITS_PER_CANDIDATE;

    // Peek remaining budget before attempting to consume, so we can surface the
    // exact remaining amount in BudgetInsufficient without double-consuming.
    let remaining = manager.budget(tenant_id).map(|b| b.remaining).unwrap_or(0);

    if units > remaining {
        return FanOutOutcome::BudgetInsufficient {
            requested: units,
            remaining,
        };
    }

    // Consume via the C7 API.
    let status = manager.try_dispatch(tenant_id, "tournament-fan-out", units, enqueue_tick, events);

    match status {
        BudgetStatus::Available => {
            let remaining_after = manager.budget(tenant_id).map(|b| b.remaining).unwrap_or(0);
            FanOutOutcome::Approved { n, remaining_after }
        }
        BudgetStatus::Exhausted | BudgetStatus::Queued => {
            // This branch is reachable if the budget was consumed concurrently
            // between the peek and the consume (or the manager has 0-capacity).
            // In either case: zero charge, zero overage.
            let remaining_after = manager.budget(tenant_id).map(|b| b.remaining).unwrap_or(0);
            FanOutOutcome::BudgetInsufficient {
                requested: units,
                remaining: remaining_after,
            }
        }
    }
}

/// Validate that the C7 fairness constants are within acceptable bounds.
///
/// This is a sanity check (called from tests) to confirm that
/// the consumed C7 constants haven't drifted from the stated contract.
pub fn assert_fairness_constants() {
    // C7 states p95 wait ≤ 5 000 ms and throughput floor ≥ 20%.
    const {
        assert!(
            P95_WAIT_BOUND_MS <= 5_000,
            "C7 p95 wait bound must be ≤ 5 000 ms"
        )
    };
    const {
        assert!(
            (THROUGHPUT_FLOOR * 100.0) as u64 >= 20,
            "C7 throughput floor must be ≥ 20%"
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_with(tenant: &str, capacity: u64) -> BudgetManager {
        let mut m = BudgetManager::default();
        m.register_tenant(tenant, capacity);
        m
    }

    #[test]
    fn policy_cap_enforced() {
        let mut mgr = manager_with("t1", 1000);
        let mut events = vec![];
        let result = attempt_fan_out(&mut mgr, "t1", MAX_N_POLICY + 1, 0, &mut events);
        assert!(result.is_cap_refused(), "N > cap must be refused");
        // Zero units consumed.
        assert_eq!(mgr.budget("t1").unwrap().remaining, 1000);
    }

    #[test]
    fn budget_insufficient_refused() {
        let mut mgr = manager_with("t1", 2);
        let mut events = vec![];
        let result = attempt_fan_out(&mut mgr, "t1", 5, 0, &mut events);
        assert!(
            result.is_budget_refused(),
            "N > remaining budget must be refused"
        );
        // Zero units consumed.
        assert_eq!(mgr.budget("t1").unwrap().remaining, 2);
    }

    #[test]
    fn approved_within_cap_and_budget() {
        let mut mgr = manager_with("t1", 10);
        let mut events = vec![];
        let result = attempt_fan_out(&mut mgr, "t1", 3, 0, &mut events);
        assert!(
            result.is_approved(),
            "N=3 within cap and budget must be approved"
        );
        assert_eq!(mgr.budget("t1").unwrap().remaining, 7, "3 units consumed");
    }

    #[test]
    fn fairness_constants_valid() {
        assert_fairness_constants();
    }
}
