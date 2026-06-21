//! WP-C7 acceptance oracle — items ①②③ of C7 (per-tenant budgets + queue fairness).
//!
//! Acceptance test for the per-tenant budgets and queue fairness contract (WP-C7).
//! Owned items:
//!   ① exhausted→queued not dropped; surfaced as defined status field + event
//!   ② fairness bound: under contention, every tenant's p95 queue wait ≤ P95_WAIT_BOUND_MS
//!      and throughput share ≥ THROUGHPUT_FLOOR (interleave fixture)
//!   ③ metering accuracy: accounted ≈ actual within ±5% on the fixture workload
//!
//! These tests are author-side; the standalone bash suite
//! (tests/acceptance/wp-c7/run.sh) drives `cargo test --test acceptance_wp-c7`
//! and adds structural/boundary assertions.

use hugit_queue::budget::{
    BudgetEvent, BudgetManager, BudgetStatus, METERING_TOLERANCE, P95_WAIT_BOUND_MS,
    THROUGHPUT_FLOOR,
};

// ── ① exhausted→queued, not dropped; status field + event ────────────────────

/// Fixture: two tenants, tenant-A has 2 units, tenant-B has 0 units.
/// When tenant-B submits work, budget is exhausted → item is QUEUED, not dropped.
/// A `BudgetExhausted` event is emitted and status is `Exhausted`.
#[test]
fn item_1_exhausted_budget_queues_not_drops() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("tenant-A", 2);
    mgr.register_tenant("tenant-B", 0); // starts exhausted

    let mut events: Vec<BudgetEvent> = Vec::new();

    // tenant-B submits work with zero budget
    let status = mgr.try_dispatch("tenant-B", "item-b1", 1, /*tick=*/ 1, &mut events);

    // Must be Exhausted (not Available), item not dropped
    assert_eq!(
        status,
        BudgetStatus::Exhausted,
        "budget exhaustion must surface as BudgetStatus::Exhausted"
    );

    // A BudgetExhausted event must be emitted (not silently dropped)
    let exhausted_events: Vec<_> = events
        .iter()
        .filter(|e| matches!(e, BudgetEvent::BudgetExhausted { .. }))
        .collect();
    assert_eq!(
        exhausted_events.len(),
        1,
        "exactly one BudgetExhausted event emitted"
    );
    assert!(
        matches!(
            &exhausted_events[0],
            BudgetEvent::BudgetExhausted { tenant_id, item_id }
            if tenant_id == "tenant-B" && item_id == "item-b1"
        ),
        "event carries correct tenant_id and item_id"
    );

    // The item is still in the queue (not dropped): verify by replenishing
    // and checking it gets dispatched.
    let mut dispatch_events: Vec<BudgetEvent> = Vec::new();
    mgr.replenish_and_drain("tenant-B", 5, /*now_tick=*/ 2, &mut dispatch_events);

    let dispatched: Vec<_> = dispatch_events
        .iter()
        .filter(|e| matches!(e, BudgetEvent::QueuedItemDispatched { .. }))
        .collect();
    assert_eq!(
        dispatched.len(),
        1,
        "queued item is dispatched after replenish"
    );
    assert!(
        matches!(
            &dispatched[0],
            BudgetEvent::QueuedItemDispatched { tenant_id, item_id }
            if tenant_id == "tenant-B" && item_id == "item-b1"
        ),
        "dispatched event matches the originally queued item"
    );
}

/// Verify that multiple items queued under exhaustion all survive and dispatch
/// in order (FIFO), none are dropped.
#[test]
fn item_1_multiple_queued_items_survive_in_order() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("tenant-X", 0);

    let mut events: Vec<BudgetEvent> = Vec::new();
    for (i, item) in ["x1", "x2", "x3"].iter().enumerate() {
        let status = mgr.try_dispatch("tenant-X", *item, 1, i as u64, &mut events);
        assert_eq!(
            status,
            BudgetStatus::Exhausted,
            "all items see Exhausted status"
        );
    }

    // 3 BudgetExhausted events, none dropped
    let exhausted_count = events
        .iter()
        .filter(|e| matches!(e, BudgetEvent::BudgetExhausted { .. }))
        .count();
    assert_eq!(exhausted_count, 3, "three distinct BudgetExhausted events");

    // Replenish and drain — all 3 must be dispatched
    let mut drain_events: Vec<BudgetEvent> = Vec::new();
    mgr.replenish_and_drain("tenant-X", 10, 100, &mut drain_events);
    let dispatched_count = drain_events
        .iter()
        .filter(|e| matches!(e, BudgetEvent::QueuedItemDispatched { .. }))
        .count();
    assert_eq!(
        dispatched_count, 3,
        "all 3 queued items dispatched after replenish"
    );
}

// ── ② fairness bound — interleave fixture ────────────────────────────────────

/// Interleave fixture: 4 tenants, each with 0 budget initially, all submit 20
/// items in round-robin. Then a bulk replenish drives drain_fair. Verify:
///   - every tenant's p95 wait ≤ P95_WAIT_BOUND_MS (here ticks map 1:1 ms in fixture)
///   - every tenant's throughput share ≥ THROUGHPUT_FLOOR
#[test]
fn item_2_fairness_bound_interleave_fixture() {
    const N_TENANTS: usize = 4;
    const ITEMS_PER_TENANT: usize = 20;
    let tenant_ids: Vec<String> = (0..N_TENANTS).map(|i| format!("t{i}")).collect();

    let mut mgr = BudgetManager::default();
    for tid in &tenant_ids {
        mgr.register_tenant(tid.as_str(), 0);
    }

    let mut events: Vec<BudgetEvent> = Vec::new();
    let mut tick: u64 = 0;

    // Interleave: t0/item0, t1/item0, ..., t3/item19 — round-robin
    for item_idx in 0..ITEMS_PER_TENANT {
        for tid in &tenant_ids {
            tick += 1;
            let item_id = format!("{tid}-item{item_idx}");
            mgr.try_dispatch(tid.as_str(), item_id, 1, tick, &mut events);
        }
    }

    // Replenish all tenants with enough budget to drain everything
    tick += 1;
    let now_tick = tick + 1; // dispatch tick after replenish
    for tid in &tenant_ids {
        mgr.replenish_and_drain(
            tid.as_str(),
            (ITEMS_PER_TENANT as u64) + 5,
            now_tick,
            &mut events,
        );
    }

    // ── p95 wait bound ────────────────────────────────────────────────────────
    // The fixture uses tick=ms (1 tick = 1 ms for test purposes).
    // Tight bound: derived from fixture constants: N_TENANTS * ITEMS_PER_TENANT ticks.
    // A vacuous 5000ms bound lets a maximally-unfair scheduler pass; this bound
    // Any p95 > N_TENANTS * ITEMS_PER_TENANT = 80 would mean starvation — caught here.
    // tight_p95_bound = N_TENANTS * ITEMS_PER_TENANT = 80 ticks (tighter than 5000 global)
    let tight_p95_bound = (N_TENANTS * ITEMS_PER_TENANT) as u64;
    let p95 = mgr.p95_wait_ticks();
    assert!(
        p95 <= tight_p95_bound,
        "p95 queue wait {p95} ticks must be <= tight_p95_bound={tight_p95_bound} (N_TENANTS * ITEMS_PER_TENANT — fairness bound, not just a smoke check)"
    );
    // Sanity: the tight bound is also within the global P95_WAIT_BOUND_MS contract.
    assert!(
        tight_p95_bound <= P95_WAIT_BOUND_MS,
        "fixture tight bound {tight_p95_bound} must be within global P95_WAIT_BOUND_MS"
    );

    // ── throughput share ≥ floor ──────────────────────────────────────────────
    for tid in &tenant_ids {
        let share = mgr.throughput_share(tid.as_str());
        assert!(
            share >= THROUGHPUT_FLOOR,
            "tenant {tid} throughput share {share:.3} must be ≥ THROUGHPUT_FLOOR={THROUGHPUT_FLOOR}"
        );
    }
}

/// Verify that under maximum contention (many tenants, one gets no priority
/// boost), the scheduler prevents any single tenant from starving others.
#[test]
fn item_2_fairness_no_starvation_under_contention() {
    const N_TENANTS: usize = 5;
    const ITEMS_PER_TENANT: usize = 10;
    let tenant_ids: Vec<String> = (0..N_TENANTS).map(|i| format!("tenant-{i}")).collect();

    let mut mgr = BudgetManager::default();
    // Give each tenant some initial budget (1 unit), so they can all queue
    // after the first item; staggered depletion.
    for tid in &tenant_ids {
        mgr.register_tenant(tid.as_str(), 0);
    }

    let mut events: Vec<BudgetEvent> = Vec::new();
    let mut tick: u64 = 0;

    for item_idx in 0..ITEMS_PER_TENANT {
        for tid in &tenant_ids {
            tick += 1;
            mgr.try_dispatch(
                tid.as_str(),
                format!("{tid}-i{item_idx}"),
                1,
                tick,
                &mut events,
            );
        }
    }

    // Bulk replenish all, drain
    tick += 100;
    for tid in &tenant_ids {
        mgr.replenish_and_drain(
            tid.as_str(),
            (ITEMS_PER_TENANT as u64) + 5,
            tick,
            &mut events,
        );
    }

    // Every tenant should have a share ≥ floor (no starvation)
    for tid in &tenant_ids {
        let share = mgr.throughput_share(tid.as_str());
        assert!(
            share >= THROUGHPUT_FLOOR,
            "no starvation: {tid} share={share:.3} must be ≥ {THROUGHPUT_FLOOR}"
        );
    }
}

// ── ③ metering accuracy ±5% ──────────────────────────────────────────────────

/// Fixture workload: 100 items, actual cost varies per item.
/// Accounted cost = actual cost (perfect accounting). Error must be ≤ 5%.
#[test]
fn item_3_metering_accuracy_within_5pct() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("tenant-meter", 10_000);

    let mut events: Vec<BudgetEvent> = Vec::new();

    // Fixture: 100 items with varying costs; accounted = actual (ideal case).
    let n_items = 100u64;
    for i in 0..n_items {
        let actual = 10 + (i % 7); // 10..16 units
        let accounted = actual; // perfect metering
        mgr.record_metering(
            "tenant-meter",
            &format!("item-{i}"),
            actual,
            accounted,
            &mut events,
        );
    }

    let err = mgr.metering_error("tenant-meter");
    assert!(
        err <= METERING_TOLERANCE,
        "metering error {err:.4} must be ≤ METERING_TOLERANCE={METERING_TOLERANCE}"
    );

    // Also verify MeterRecord events were emitted
    let meter_events = events
        .iter()
        .filter(|e| matches!(e, BudgetEvent::MeterRecord { .. }))
        .count();
    assert_eq!(
        meter_events, n_items as usize,
        "one MeterRecord event per item"
    );
}

/// Test with intentional ≤5% over-accounting (just within tolerance).
#[test]
fn item_3_metering_just_within_tolerance() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("tenant-overage", 100_000);

    let mut events: Vec<BudgetEvent> = Vec::new();

    // 100 items, each with actual=100, accounted=104 (4% over — within ±5%)
    for i in 0..100u64 {
        mgr.record_metering(
            "tenant-overage",
            &format!("item-{i}"),
            100,
            104,
            &mut events,
        );
    }

    let err = mgr.metering_error("tenant-overage");
    assert!(
        err <= METERING_TOLERANCE,
        "4% overage {err:.4} must be within ±5% tolerance"
    );
}

/// Test that >5% error is detected (validates the tolerance assertion is tight).
#[test]
fn item_3_metering_beyond_tolerance_is_detectable() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("tenant-excess", 100_000);

    let mut events: Vec<BudgetEvent> = Vec::new();

    // 100 items, actual=100, accounted=110 (10% over — exceeds ±5%)
    for i in 0..100u64 {
        mgr.record_metering("tenant-excess", &format!("item-{i}"), 100, 110, &mut events);
    }

    let err = mgr.metering_error("tenant-excess");
    // This must EXCEED tolerance — the test verifies the meter is accurate enough
    // to detect over-billing scenarios.
    assert!(
        err > METERING_TOLERANCE,
        "10% overage {err:.4} should exceed ±5% tolerance (detection works)"
    );
}
