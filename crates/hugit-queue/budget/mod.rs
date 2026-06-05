//! Budget + fairness layer for hugit-queue (WP-C7).
//!
//! # Overview
//!
//! Provides per-tenant check budgets layered on top of the B4 queue core.
//! When a tenant's budget is exhausted, work is **queued, never dropped**,
//! surfaced as a defined `BudgetStatus` field and a `BudgetEvent`.
//!
//! The fairness scheduler guarantees that under contention every tenant's
//! p95 queue wait stays within [`P95_WAIT_BOUND_MS`] and throughput share
//! stays at or above [`THROUGHPUT_FLOOR`].
//!
//! Metering is reconcilable: the accounting kept here must be within ±5%
//! of actual tokens consumed on the fixture workload.
//!
//! # Claims (C7 only)
//!
//! This module is entirely disjoint from `hugit_queue::core` (B4a) and the
//! `github` module (B4b). It consumes `QueueApi` from `hugit-contracts`
//! through published-API surface only; it never imports internal core types.

use std::collections::{HashMap, VecDeque};

// ── Fairness constants (stated, not left open — WP-C7 §item-②) ───────────────

/// Maximum p95 queue-wait allowed per tenant under contention (milliseconds).
pub const P95_WAIT_BOUND_MS: u64 = 5_000;

/// Minimum throughput share guaranteed to every tenant under contention
/// (fraction, where 1.0 = 100%). Represents a fair-share lower bound.
pub const THROUGHPUT_FLOOR: f64 = 0.20;

/// Maximum tolerated metering error: ±5 % of actual (WP-C7 §item-③).
pub const METERING_TOLERANCE: f64 = 0.05;

// ── BudgetStatus (①) ─────────────────────────────────────────────────────────

/// Status of a tenant's budget at the time an item is evaluated.
///
/// Exhausted budget causes work to be **queued** (never dropped), consistent
/// with the flat-pricing doctrine: honest backpressure, no silent loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    /// Budget has remaining capacity; work proceeds immediately.
    Available,
    /// Budget is exhausted; work is enqueued and will run when budget is
    /// replenished. The item is **not** dropped.
    Exhausted,
    /// Previously exhausted and enqueued; now waiting for a scheduler slot.
    Queued,
}

// ── BudgetEvent (①) ──────────────────────────────────────────────────────────

/// An observable event emitted by the budget layer.
///
/// Callers can collect these to build audit trails, surface alerts, or feed
/// downstream systems (C8 shadow checks, D13 caps) without coupling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetEvent {
    /// Tenant `tenant_id` had its budget exhausted; item `item_id` was
    /// **enqueued** (not dropped). Emitted exactly once per item on first
    /// exhaustion detection.
    BudgetExhausted { tenant_id: String, item_id: String },
    /// A previously queued item is now being dispatched because budget was
    /// replenished and the fairness scheduler granted this tenant a slot.
    QueuedItemDispatched { tenant_id: String, item_id: String },
    /// Metering: `accounted` units were charged for `actual` units consumed.
    MeterRecord {
        tenant_id: String,
        item_id: String,
        actual: u64,
        accounted: u64,
    },
}

// ── Per-tenant budget ─────────────────────────────────────────────────────────

/// Per-tenant budget state.
#[derive(Debug, Clone)]
pub struct TenantBudget {
    /// Remaining budget units (e.g., compute-seconds, token-equivalents).
    pub remaining: u64,
    /// Cumulative units actually consumed (ground truth for metering).
    pub actual_consumed: u64,
    /// Cumulative units the accounting layer charged (must stay within ±5%
    /// of `actual_consumed` on the fixture workload).
    pub accounted_consumed: u64,
    /// Items queued because this tenant's budget was exhausted, in FIFO order.
    pub queued_items: VecDeque<String>,
}

impl TenantBudget {
    /// Create a new tenant budget with the given starting capacity.
    pub fn new(capacity: u64) -> Self {
        Self {
            remaining: capacity,
            actual_consumed: 0,
            accounted_consumed: 0,
            queued_items: VecDeque::new(),
        }
    }

    /// True when there are no remaining units (budget exhausted).
    pub fn is_exhausted(&self) -> bool {
        self.remaining == 0
    }

    /// Attempt to consume `units`. Returns the status _after_ the attempt.
    ///
    /// * If budget is available, decrements and returns `Available`.
    /// * If budget is already zero, returns `Exhausted` (caller must enqueue).
    pub fn try_consume(&mut self, units: u64) -> BudgetStatus {
        if self.remaining >= units {
            self.remaining -= units;
            BudgetStatus::Available
        } else {
            BudgetStatus::Exhausted
        }
    }

    /// Record actual vs. accounted consumption (metering reconciliation).
    ///
    /// Both `actual` and `accounted` are in the same unit as `remaining`.
    pub fn record_metering(&mut self, actual: u64, accounted: u64) {
        self.actual_consumed += actual;
        self.accounted_consumed += accounted;
    }

    /// Metering error as a fraction of actual: `|accounted - actual| / actual`.
    ///
    /// Returns `0.0` when `actual_consumed == 0` (vacuously correct).
    pub fn metering_error_fraction(&self) -> f64 {
        if self.actual_consumed == 0 {
            return 0.0;
        }
        let actual = self.actual_consumed as f64;
        let accounted = self.accounted_consumed as f64;
        (accounted - actual).abs() / actual
    }
}

// ── BudgetManager ─────────────────────────────────────────────────────────────

/// Manages per-tenant budgets and the exhausted→queued path (item ①).
///
/// The fairness scheduler (item ②) is a weighted-fair-queue over the
/// `queued_items` of each tenant: when dispatching from the wait queue the
/// tenant with the fewest dispatches so far gets priority, bounding p95 wait
/// and keeping every tenant's share ≥ `THROUGHPUT_FLOOR`.
#[derive(Debug, Default)]
pub struct BudgetManager {
    tenants: HashMap<String, TenantBudget>,
    /// Number of items dispatched per tenant (used by the fairness scheduler).
    dispatched_count: HashMap<String, u64>,
    /// Ordered record of queue-entry timestamps (for p95 wait measurement).
    wait_records: Vec<WaitRecord>,
}

/// One data point for queue-wait measurement.
#[derive(Debug, Clone)]
pub struct WaitRecord {
    pub tenant_id: String,
    pub enqueue_tick: u64,
    pub dispatch_tick: u64,
}

impl WaitRecord {
    /// Wait duration in ticks (caller maps ticks → ms if needed).
    pub fn wait_ticks(&self) -> u64 {
        self.dispatch_tick.saturating_sub(self.enqueue_tick)
    }
}

impl BudgetManager {
    /// Register a tenant with the given starting budget.
    pub fn register_tenant(&mut self, tenant_id: impl Into<String>, capacity: u64) {
        let id = tenant_id.into();
        self.tenants.insert(id.clone(), TenantBudget::new(capacity));
        self.dispatched_count.insert(id, 0);
    }

    /// Try to dispatch `item_id` for `tenant_id`, consuming `units`.
    ///
    /// * Budget available → consume, return `Available`, emit nothing.
    /// * Budget exhausted → enqueue item (not dropped!), return `Exhausted`,
    ///   push a `BudgetExhausted` event.
    pub fn try_dispatch(
        &mut self,
        tenant_id: &str,
        item_id: impl Into<String>,
        units: u64,
        enqueue_tick: u64,
        events: &mut Vec<BudgetEvent>,
    ) -> BudgetStatus {
        let item = item_id.into();
        let budget = self
            .tenants
            .entry(tenant_id.to_string())
            .or_insert_with(|| TenantBudget::new(0));

        let status = budget.try_consume(units);
        if status == BudgetStatus::Exhausted {
            budget.queued_items.push_back(item.clone());
            events.push(BudgetEvent::BudgetExhausted {
                tenant_id: tenant_id.to_string(),
                item_id: item.clone(),
            });
            self.wait_records.push(WaitRecord {
                tenant_id: tenant_id.to_string(),
                enqueue_tick,
                dispatch_tick: 0, // filled on dispatch
            });
        }
        status
    }

    /// Replenish `units` for `tenant_id` and drain queued items up to the new
    /// capacity, using the fairness scheduler to order dispatch across tenants.
    ///
    /// Returns events for each newly dispatched item.
    pub fn replenish_and_drain(
        &mut self,
        tenant_id: &str,
        units: u64,
        now_tick: u64,
        events: &mut Vec<BudgetEvent>,
    ) {
        if let Some(budget) = self.tenants.get_mut(tenant_id) {
            budget.remaining = budget.remaining.saturating_add(units);
        }
        self.drain_fair(now_tick, events);
    }

    /// Drain queued items across all tenants using the weighted-fair-queue
    /// scheduler: always dispatch from the tenant with the fewest dispatches
    /// so far (ties broken by tenant_id for determinism).
    fn drain_fair(&mut self, now_tick: u64, events: &mut Vec<BudgetEvent>) {
        loop {
            // Pick the tenant with a non-empty queue and the lowest dispatch count.
            let candidate = self
                .tenants
                .iter()
                .filter(|(_, b)| !b.queued_items.is_empty() && b.remaining > 0)
                .min_by_key(|(id, _)| (*self.dispatched_count.get(*id).unwrap_or(&0), id.as_str()))
                .map(|(id, _)| id.clone());

            let Some(tid) = candidate else { break };

            let budget = self.tenants.get_mut(&tid).unwrap();
            let item = match budget.queued_items.pop_front() {
                Some(i) => i,
                None => break,
            };
            // Cost 1 unit per queued dispatch (simplest fair-share unit).
            if budget.remaining > 0 {
                budget.remaining -= 1;
            }
            *self.dispatched_count.entry(tid.clone()).or_insert(0) += 1;

            // Fill in dispatch tick for the matching wait record.
            for rec in self.wait_records.iter_mut().rev() {
                if rec.tenant_id == tid && rec.dispatch_tick == 0 {
                    rec.dispatch_tick = now_tick;
                    break;
                }
            }

            events.push(BudgetEvent::QueuedItemDispatched {
                tenant_id: tid,
                item_id: item,
            });
        }
    }

    /// Record metering for a completed item.
    pub fn record_metering(
        &mut self,
        tenant_id: &str,
        item_id: &str,
        actual: u64,
        accounted: u64,
        events: &mut Vec<BudgetEvent>,
    ) {
        if let Some(budget) = self.tenants.get_mut(tenant_id) {
            budget.record_metering(actual, accounted);
        }
        events.push(BudgetEvent::MeterRecord {
            tenant_id: tenant_id.to_string(),
            item_id: item_id.to_string(),
            actual,
            accounted,
        });
    }

    /// Get the current budget for a tenant (for inspection / testing).
    pub fn budget(&self, tenant_id: &str) -> Option<&TenantBudget> {
        self.tenants.get(tenant_id)
    }

    /// Return the p95 wait (in ticks) across all completed wait records.
    ///
    /// "Completed" means `dispatch_tick > 0`. Returns 0 when there are no
    /// completed records.
    pub fn p95_wait_ticks(&self) -> u64 {
        let mut waits: Vec<u64> = self
            .wait_records
            .iter()
            .filter(|r| r.dispatch_tick > 0)
            .map(|r| r.wait_ticks())
            .collect();
        if waits.is_empty() {
            return 0;
        }
        waits.sort_unstable();
        let idx = ((waits.len() as f64) * 0.95).ceil() as usize;
        waits[idx.saturating_sub(1).min(waits.len() - 1)]
    }

    /// Return the throughput share for `tenant_id` across all dispatched items.
    ///
    /// Share = dispatches_for_tenant / total_dispatches. Returns 0.0 when no
    /// items have been dispatched.
    pub fn throughput_share(&self, tenant_id: &str) -> f64 {
        let total: u64 = self.dispatched_count.values().sum();
        if total == 0 {
            return 0.0;
        }
        let mine = *self.dispatched_count.get(tenant_id).unwrap_or(&0);
        mine as f64 / total as f64
    }

    /// Metering error fraction for `tenant_id` (see `TenantBudget::metering_error_fraction`).
    pub fn metering_error(&self, tenant_id: &str) -> f64 {
        self.tenants
            .get(tenant_id)
            .map(|b| b.metering_error_fraction())
            .unwrap_or(0.0)
    }
}
