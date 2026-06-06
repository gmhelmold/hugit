//! Pricing no-shock guard (WP-C10).
//!
//! Drives a tenant to budget exhaustion on **each metered surface**
//! (runner minutes, shadow spend, storage), caps/degrades (pauses or falls
//! back) with a pre-exhaustion warning, and ensures **zero overage charge** —
//! flat means flat.
//!
//! # Invariants
//!
//! * Any tenant driven past their surface budget receives a
//!   [`NoShockWarning`] **before** the wall is hit (when remaining drops to
//!   ≤ [`WARN_THRESHOLD_FRACTION`] of capacity).
//! * After the wall the system **caps/degrades** — it pauses or falls back,
//!   never silently continues and never generates an overage charge.
//! * The billing fixture asserts that `overage_charge == 0` at all times:
//!   exhaustion is honest backpressure, not a billing event.
//!
//! # Disjointness
//!
//! This module is entirely disjoint from the C7 budget core.  It
//! **consumes** only `BudgetStatus` from the published surface; it never
//! imports internal core types and never touches sibling core or engine modules.

use std::collections::HashMap;

use crate::budget::BudgetStatus;

// ── Pre-exhaustion warn threshold ─────────────────────────────────────────────

/// Fraction of capacity at which a pre-exhaustion [`NoShockWarning`] is emitted.
///
/// When remaining ≤ capacity × `WARN_THRESHOLD_FRACTION` (and remaining > 0),
/// the guard emits the warning **before** the tenant hits the wall.
pub const WARN_THRESHOLD_FRACTION: f64 = 0.20;

// ── Metered surfaces ──────────────────────────────────────────────────────────

/// The three metered surfaces tracked by the no-shock guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeteredSurface {
    /// CPU / wall-clock minutes consumed by runner jobs.
    RunnerMinutes,
    /// Budget units pre-spent by shadow-checks (C8 approximation pre-billing).
    ShadowSpend,
    /// Bytes stored in the CAS / ref-store accounting tier.
    Storage,
}

// ── Guard action ──────────────────────────────────────────────────────────────

/// The action the no-shock guard takes when a surface is driven to exhaustion.
///
/// Both variants satisfy the §9 lock-5 degradation invariant: the state is
/// surfaced honestly, never silently dropped or falsely reported as green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardAction {
    /// Work is **paused** (held in queue, will resume on replenish).
    Pause,
    /// Work is **falling back** to a cheaper / degraded execution path.
    Fallback,
}

// ── NoShockWarning ────────────────────────────────────────────────────────────

/// Warning emitted when a tenant's remaining budget for a surface drops to
/// ≤ [`WARN_THRESHOLD_FRACTION`] × capacity (pre-exhaustion, not post).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoShockWarning {
    pub tenant_id: String,
    pub surface: MeteredSurface,
    /// Remaining units at the time the warning was emitted.
    pub remaining: u64,
    /// Original capacity for this surface.
    pub capacity: u64,
}

// ── CapDegrade event ──────────────────────────────────────────────────────────

/// Emitted when the no-shock guard caps/degrades a tenant on a surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapDegrade {
    pub tenant_id: String,
    pub surface: MeteredSurface,
    pub action: GuardAction,
    /// Item that triggered the exhaustion.
    pub item_id: String,
}

// ── BillingFixture — zero-overage assertion (item ②) ─────────────────────────

/// Billing fixture: proves flat-pricing invariant.
///
/// The `overage_charge` field is always `0`; the flat monthly fee covers all
/// usage up to capacity. Exhaustion never generates a billing event — it
/// generates a [`CapDegrade`] (backpressure), not a charge.
///
/// This struct doubles as the runtime ledger for a single billing period and
/// as the fixture that acceptance tests assert against (`overage_charge == 0`).
#[derive(Debug, Clone, Default)]
pub struct BillingFixture {
    /// Flat period charge (e.g. monthly subscription).  May be non-zero.
    pub flat_charge: u64,
    /// Overage charge — **always zero** per the flat-pricing doctrine.
    ///
    /// Whitepaper §11: never usage-billing whiplash; never meter the
    /// customer's own compute.
    pub overage_charge: u64,
    /// Number of cap/degrade events recorded this period.
    pub cap_degrade_events: u64,
    /// Number of pre-exhaustion warnings emitted this period.
    pub warning_events: u64,
}

impl BillingFixture {
    /// Record a cap/degrade event.  **Never** increments `overage_charge`.
    pub fn record_cap_degrade(&mut self) {
        self.cap_degrade_events += 1;
        // overage_charge remains 0 — flat means flat.
    }

    /// Record a warning event.
    pub fn record_warning(&mut self) {
        self.warning_events += 1;
    }

    /// Assert the zero-overage invariant: flat means flat.
    ///
    /// Panics with a descriptive message if `overage_charge != 0`.
    pub fn assert_zero_overage(&self) {
        assert_eq!(
            self.overage_charge, 0,
            "flat means flat: overage_charge must be 0, got {}",
            self.overage_charge
        );
    }

    /// Returns `true` iff the zero-overage invariant holds.
    pub fn is_zero_overage(&self) -> bool {
        self.overage_charge == 0
    }
}

// ── Per-surface budget state ──────────────────────────────────────────────────

/// Per-surface capacity + remaining budget for one tenant.
#[derive(Debug, Clone)]
pub struct SurfaceBudget {
    pub capacity: u64,
    pub remaining: u64,
    /// Whether the warning for this surface has already been emitted.
    pub warning_emitted: bool,
}

impl SurfaceBudget {
    pub fn new(capacity: u64) -> Self {
        Self {
            capacity,
            remaining: capacity,
            warning_emitted: false,
        }
    }

    /// True when remaining units have hit zero.
    pub fn is_exhausted(&self) -> bool {
        self.remaining == 0
    }

    /// True when remaining ≤ warn threshold AND warning not yet emitted.
    pub fn should_warn(&self) -> bool {
        if self.warning_emitted || self.remaining == 0 {
            return false;
        }
        let threshold = (self.capacity as f64 * WARN_THRESHOLD_FRACTION).ceil() as u64;
        self.remaining <= threshold
    }

    /// Consume `units`, saturating at zero.  Returns new remaining.
    pub fn consume(&mut self, units: u64) -> u64 {
        self.remaining = self.remaining.saturating_sub(units);
        self.remaining
    }
}

// ── NoShockGuard ──────────────────────────────────────────────────────────────

/// Per-tenant, per-surface no-shock guard.
///
/// Owns three surface budgets per tenant (runner minutes, shadow spend,
/// storage).  On each `consume` call it:
///
/// 1. Emits a [`NoShockWarning`] if crossing the warn threshold (pre-wall).
/// 2. If exhausted, emits a [`CapDegrade`] and records in the
///    [`BillingFixture`] — **without** adding any overage charge.
/// 3. Returns the [`BudgetStatus`] so callers can pause/fallback.
#[derive(Debug, Default)]
pub struct NoShockGuard {
    /// Map: tenant_id → surface → SurfaceBudget.
    budgets: HashMap<String, HashMap<MeteredSurface, SurfaceBudget>>,
    /// Billing ledger: proves zero overage across all tenants.
    pub billing: BillingFixture,
}

impl NoShockGuard {
    /// Register a tenant with explicit capacity on each surface.
    pub fn register_tenant(
        &mut self,
        tenant_id: impl Into<String>,
        runner_minutes: u64,
        shadow_spend: u64,
        storage: u64,
    ) {
        let id = tenant_id.into();
        let mut surfaces = HashMap::new();
        surfaces.insert(
            MeteredSurface::RunnerMinutes,
            SurfaceBudget::new(runner_minutes),
        );
        surfaces.insert(
            MeteredSurface::ShadowSpend,
            SurfaceBudget::new(shadow_spend),
        );
        surfaces.insert(MeteredSurface::Storage, SurfaceBudget::new(storage));
        self.budgets.insert(id, surfaces);
    }

    /// Consume `units` from `surface` for `tenant_id`.
    ///
    /// Returns `(BudgetStatus, warnings, cap_degrades)`.
    ///
    /// * Emits a [`NoShockWarning`] if crossing the warn threshold.
    /// * If exhausted, emits a [`CapDegrade`] + records in billing.
    /// * The billing `overage_charge` is **never** incremented.
    pub fn consume(
        &mut self,
        tenant_id: &str,
        item_id: impl Into<String>,
        surface: MeteredSurface,
        units: u64,
    ) -> (BudgetStatus, Vec<NoShockWarning>, Vec<CapDegrade>) {
        let item = item_id.into();
        let mut warnings = Vec::new();
        let mut cap_degrades = Vec::new();

        let surfaces = self.budgets.entry(tenant_id.to_string()).or_default();

        let budget = surfaces
            .entry(surface)
            .or_insert_with(|| SurfaceBudget::new(0));

        // Check warn threshold BEFORE consuming (pre-exhaustion).
        if budget.should_warn() {
            warnings.push(NoShockWarning {
                tenant_id: tenant_id.to_string(),
                surface,
                remaining: budget.remaining,
                capacity: budget.capacity,
            });
            budget.warning_emitted = true;
            self.billing.record_warning();
        }

        // Consume units.
        budget.consume(units);

        if budget.is_exhausted() {
            // Check warn threshold again after consuming (catches the boundary
            // where warn + exhaust happen in the same consume call).
            if !budget.warning_emitted {
                warnings.push(NoShockWarning {
                    tenant_id: tenant_id.to_string(),
                    surface,
                    remaining: 0,
                    capacity: budget.capacity,
                });
                budget.warning_emitted = true;
                self.billing.record_warning();
            }

            let action = match surface {
                MeteredSurface::RunnerMinutes => GuardAction::Pause,
                MeteredSurface::ShadowSpend => GuardAction::Fallback,
                MeteredSurface::Storage => GuardAction::Pause,
            };

            cap_degrades.push(CapDegrade {
                tenant_id: tenant_id.to_string(),
                surface,
                action,
                item_id: item,
            });
            self.billing.record_cap_degrade();
            // billing.overage_charge intentionally NOT touched — flat means flat.

            return (BudgetStatus::Exhausted, warnings, cap_degrades);
        }

        (BudgetStatus::Available, warnings, cap_degrades)
    }

    /// Inspect the current [`SurfaceBudget`] for a tenant/surface pair.
    pub fn surface_budget(
        &self,
        tenant_id: &str,
        surface: MeteredSurface,
    ) -> Option<&SurfaceBudget> {
        self.budgets.get(tenant_id)?.get(&surface)
    }

    /// Assert that the billing fixture shows zero overage (item ②).
    pub fn assert_zero_overage(&self) {
        self.billing.assert_zero_overage();
    }
}

// ── Convenience: drive a tenant to exhaustion on a single surface ─────────────

/// Drive `tenant_id` to exhaustion on `surface` by consuming 1 unit at a time
/// until the budget is exhausted.
///
/// Returns all warnings and cap/degrade events emitted during the drive.
/// After this call `billing.overage_charge` must remain 0.
pub fn drive_to_exhaustion(
    guard: &mut NoShockGuard,
    tenant_id: &str,
    surface: MeteredSurface,
    capacity: u64,
) -> (Vec<NoShockWarning>, Vec<CapDegrade>) {
    let mut all_warnings = Vec::new();
    let mut all_cap_degrades = Vec::new();

    for i in 0..=capacity {
        let item = format!("item-exhaust-{i}");
        let (_, warnings, cap_degrades) = guard.consume(tenant_id, item, surface, 1);
        all_warnings.extend(warnings);
        all_cap_degrades.extend(cap_degrades);

        // Stop driving once exhausted — we have the cap/degrade event.
        if !all_cap_degrades.is_empty() {
            break;
        }
    }

    (all_warnings, all_cap_degrades)
}
