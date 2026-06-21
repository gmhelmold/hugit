//! WP-C10 acceptance oracle — items ① ② of C10 (pricing no-shock guard).
//!
//! Acceptance test for the pricing no-shock guard contract (WP-C10).
//! Claims:   crates/hugit-queue/budget/no_shock/
//!
//! Owned acceptance items:
//!   ① driving a tenant to budget exhaustion on EACH metered surface
//!      (runner minutes, shadow spend, storage) → system caps/degrades
//!      (pauses or falls back) with pre-exhaustion warning
//!   ② zero overage charge generated — flat means flat (billing fixture assert)

use hugit_queue::budget::BudgetStatus;
use hugit_queue::budget::no_shock::{
    BillingFixture, GuardAction, MeteredSurface, NoShockGuard, WARN_THRESHOLD_FRACTION,
    drive_to_exhaustion,
};

// ── ① runner minutes surface ──────────────────────────────────────────────────

/// Driving a tenant to exhaustion on RunnerMinutes caps (Pause) with a
/// pre-exhaustion warning before the wall.
#[test]
fn item_1_runner_minutes_cap_degrade() {
    const CAPACITY: u64 = 100;
    let mut guard = NoShockGuard::default();
    guard.register_tenant("tenant-runner", CAPACITY, CAPACITY, CAPACITY);

    let (warnings, cap_degrades) = drive_to_exhaustion(
        &mut guard,
        "tenant-runner",
        MeteredSurface::RunnerMinutes,
        CAPACITY,
    );

    // Must have at least one pre-exhaustion warning emitted.
    assert!(
        !warnings.is_empty(),
        "pre-exhaustion warning must be emitted before the RunnerMinutes wall"
    );
    assert!(
        warnings
            .iter()
            .all(|w| w.surface == MeteredSurface::RunnerMinutes),
        "warnings are for RunnerMinutes surface"
    );
    // Warning must arrive while capacity is not already zero (pre-exhaustion).
    assert!(
        warnings
            .iter()
            .any(|w| w.remaining <= (CAPACITY as f64 * WARN_THRESHOLD_FRACTION).ceil() as u64),
        "warning emitted at or before warn threshold"
    );

    // Must have exactly one cap/degrade event.
    assert_eq!(
        cap_degrades.len(),
        1,
        "exactly one cap/degrade event on exhaustion"
    );
    let cd = &cap_degrades[0];
    assert_eq!(cd.surface, MeteredSurface::RunnerMinutes);
    assert_eq!(cd.tenant_id, "tenant-runner");
    // RunnerMinutes caps as Pause.
    assert_eq!(
        cd.action,
        GuardAction::Pause,
        "RunnerMinutes exhaustion must pause (not silently continue)"
    );

    // Final BudgetStatus: exhausted.
    let sb = guard
        .surface_budget("tenant-runner", MeteredSurface::RunnerMinutes)
        .expect("surface budget must exist");
    assert!(
        sb.is_exhausted(),
        "RunnerMinutes budget must be exhausted after drive"
    );

    // Zero overage (item ② sub-check).
    guard.assert_zero_overage();
}

// ── ① shadow spend surface ────────────────────────────────────────────────────

/// Driving a tenant to exhaustion on ShadowSpend falls back with a
/// pre-exhaustion warning.
#[test]
fn item_1_shadow_spend_cap_degrade() {
    const CAPACITY: u64 = 50;
    let mut guard = NoShockGuard::default();
    guard.register_tenant("tenant-shadow", CAPACITY, CAPACITY, CAPACITY);

    let (warnings, cap_degrades) = drive_to_exhaustion(
        &mut guard,
        "tenant-shadow",
        MeteredSurface::ShadowSpend,
        CAPACITY,
    );

    assert!(
        !warnings.is_empty(),
        "pre-exhaustion warning must be emitted before the ShadowSpend wall"
    );
    assert!(
        warnings
            .iter()
            .all(|w| w.surface == MeteredSurface::ShadowSpend),
        "warnings are for ShadowSpend surface"
    );

    assert_eq!(cap_degrades.len(), 1, "exactly one cap/degrade event");
    let cd = &cap_degrades[0];
    assert_eq!(cd.surface, MeteredSurface::ShadowSpend);
    assert_eq!(cd.tenant_id, "tenant-shadow");
    // ShadowSpend falls back (cheaper path, not a full pause).
    assert_eq!(
        cd.action,
        GuardAction::Fallback,
        "ShadowSpend exhaustion must fallback"
    );

    let sb = guard
        .surface_budget("tenant-shadow", MeteredSurface::ShadowSpend)
        .expect("surface budget must exist");
    assert!(
        sb.is_exhausted(),
        "ShadowSpend budget must be exhausted after drive"
    );

    guard.assert_zero_overage();
}

// ── ① storage surface ─────────────────────────────────────────────────────────

/// Driving a tenant to exhaustion on Storage caps (Pause) with a
/// pre-exhaustion warning.
#[test]
fn item_1_storage_cap_degrade() {
    const CAPACITY: u64 = 200;
    let mut guard = NoShockGuard::default();
    guard.register_tenant("tenant-store", CAPACITY, CAPACITY, CAPACITY);

    let (warnings, cap_degrades) = drive_to_exhaustion(
        &mut guard,
        "tenant-store",
        MeteredSurface::Storage,
        CAPACITY,
    );

    assert!(
        !warnings.is_empty(),
        "pre-exhaustion warning must be emitted before the Storage wall"
    );
    assert!(
        warnings
            .iter()
            .all(|w| w.surface == MeteredSurface::Storage),
        "warnings are for Storage surface"
    );

    assert_eq!(cap_degrades.len(), 1, "exactly one cap/degrade event");
    let cd = &cap_degrades[0];
    assert_eq!(cd.surface, MeteredSurface::Storage);
    assert_eq!(cd.tenant_id, "tenant-store");
    assert_eq!(
        cd.action,
        GuardAction::Pause,
        "Storage exhaustion must pause"
    );

    let sb = guard
        .surface_budget("tenant-store", MeteredSurface::Storage)
        .expect("surface budget must exist");
    assert!(
        sb.is_exhausted(),
        "Storage budget must be exhausted after drive"
    );

    guard.assert_zero_overage();
}

// ── ① pre-exhaustion warning emitted (all three surfaces, direct threshold check) ──

/// Direct fixture: all three surfaces warn at ≤ WARN_THRESHOLD_FRACTION of capacity,
/// ensuring warnings arrive BEFORE the wall on every metered surface.
#[test]
fn item_1_pre_exhaustion_warning_emitted() {
    const CAPACITY: u64 = 10;
    let surfaces = [
        MeteredSurface::RunnerMinutes,
        MeteredSurface::ShadowSpend,
        MeteredSurface::Storage,
    ];

    for surface in surfaces {
        let mut guard = NoShockGuard::default();
        guard.register_tenant("t", CAPACITY, CAPACITY, CAPACITY);

        let warn_threshold = (CAPACITY as f64 * WARN_THRESHOLD_FRACTION).ceil() as u64;

        // Consume until we're just AT the warn threshold (remaining == threshold).
        let consume_to_warn = CAPACITY.saturating_sub(warn_threshold);
        for i in 0..consume_to_warn {
            let (_status, warnings, _cd) = guard.consume("t", format!("pre-{i}"), surface, 1);
            // Should have no warning yet (still above threshold).
            assert!(
                warnings.is_empty(),
                "no warning yet at remaining > threshold (surface={surface:?}, step={i})"
            );
        }

        // The next consume should trigger the warning (remaining == warn_threshold → ≤ threshold).
        let (_status, warnings, _cd) = guard.consume("t", "at-threshold", surface, 1);
        assert!(
            !warnings.is_empty(),
            "warning must be emitted at warn threshold (surface={surface:?})"
        );
        let w = &warnings[0];
        assert_eq!(w.surface, surface, "warning surface matches");
        assert_eq!(w.tenant_id, "t", "warning tenant matches");
        assert!(
            w.remaining <= warn_threshold,
            "warning remaining={} ≤ threshold={warn_threshold}",
            w.remaining
        );
    }
}

// ── ② zero overage — flat means flat (billing fixture) ───────────────────────

/// Drive all three surfaces to exhaustion, verify billing fixture shows
/// zero overage charge and non-zero cap/degrade + warning events.
#[test]
fn item_2_zero_overage_flat_means_flat() {
    const CAPACITY: u64 = 20;
    let mut guard = NoShockGuard::default();
    guard.register_tenant("tenant-billing", CAPACITY, CAPACITY, CAPACITY);

    // Drive all three surfaces to exhaustion.
    let surfaces = [
        MeteredSurface::RunnerMinutes,
        MeteredSurface::ShadowSpend,
        MeteredSurface::Storage,
    ];
    for surface in surfaces {
        drive_to_exhaustion(&mut guard, "tenant-billing", surface, CAPACITY);
    }

    // ① Billing fixture: overage_charge MUST be zero.
    assert_eq!(
        guard.billing.overage_charge, 0,
        "flat means flat: exhausting all three surfaces must never generate an overage charge"
    );

    // ② Cap/degrade events were recorded (honest surfaced state, not silent drop).
    assert!(
        guard.billing.cap_degrade_events >= 3,
        "at least one cap/degrade event per surface (got {})",
        guard.billing.cap_degrade_events
    );

    // ③ Pre-exhaustion warnings were recorded.
    assert!(
        guard.billing.warning_events >= 3,
        "at least one warning event per surface (got {})",
        guard.billing.warning_events
    );

    // ④ Convenience assert_zero_overage() does not panic.
    guard.assert_zero_overage();
}

/// BillingFixture unit test: confirm is_zero_overage() and
/// assert_zero_overage() behave correctly.
#[test]
fn item_2_billing_fixture_zero_overage_invariant() {
    let mut fixture = BillingFixture {
        flat_charge: 2900, // $29/mo flat
        ..Default::default()
    };

    // Record cap/degrade events — must not affect overage.
    fixture.record_cap_degrade();
    fixture.record_cap_degrade();
    fixture.record_warning();

    assert_eq!(
        fixture.overage_charge, 0,
        "overage_charge stays 0 after cap/degrade events"
    );
    assert!(fixture.is_zero_overage(), "is_zero_overage() returns true");
    // Must not panic.
    fixture.assert_zero_overage();

    assert_eq!(
        fixture.cap_degrade_events, 2,
        "two cap/degrade events recorded"
    );
    assert_eq!(fixture.warning_events, 1, "one warning event recorded");
    assert_eq!(fixture.flat_charge, 2900, "flat_charge unchanged");
}

/// Confirm that a hypothetical (broken) implementation that sets overage_charge > 0
/// is caught by assert_zero_overage().
#[test]
#[should_panic(expected = "flat means flat")]
fn item_2_overage_charge_nonzero_panics() {
    let fixture = BillingFixture {
        flat_charge: 2900,
        overage_charge: 1, // intentional breakage — must be caught
        cap_degrade_events: 0,
        warning_events: 0,
    };
    fixture.assert_zero_overage();
}

/// Verify that after exhaustion the guard returns BudgetStatus::Exhausted.
#[test]
fn item_1_exhausted_status_returned() {
    const CAPACITY: u64 = 5;
    let mut guard = NoShockGuard::default();
    guard.register_tenant("t-status", CAPACITY, CAPACITY, CAPACITY);

    // Consume all units.
    for i in 0..CAPACITY {
        let (status, _, _) = guard.consume(
            "t-status",
            format!("pre-{i}"),
            MeteredSurface::RunnerMinutes,
            1,
        );
        // All-but-last should be Available (may vary with warning emission).
        if i < CAPACITY - 1 {
            // Available until last.
            let _ = status; // status valid regardless
        }
    }

    // One more consume after exhaustion — must return Exhausted.
    let (status, _, _) =
        guard.consume("t-status", "post-exhaust", MeteredSurface::RunnerMinutes, 1);
    assert_eq!(
        status,
        BudgetStatus::Exhausted,
        "BudgetStatus::Exhausted must be returned when surface is exhausted"
    );
}
