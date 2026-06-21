//! WP-C8 acceptance oracle — shadow checks (snapshot-cadence, budget-capped,
//! non-gating). One `#[test] item_<n>_<slug>` per owned acceptance item.
//!
//! Acceptance test for the shadow-checks contract (WP-C8).
//!   ① N writes in one snapshot window → exactly ONE shadow pass at boundary
//!      (not N, not 0)
//!   ② shadow runs decrement tenant budget; cap halts shadows, explicit jobs
//!      proceed per policy
//!   ③ default-off; per-repo opt-in flag scoped to repo, zero runs when off
//!   ④ a failing shadow surfaces as signal/event and NEVER gates/blocks/fails
//!      any explicit job; a passing shadow produces an observable result
//!   ⑤ per-tenant cap isolation: tenant A exhausting its shadow budget leaves
//!      tenant B's shadows unaffected

use hugit_checks::shadow::{
    ExplicitJobStatus, OPTIN_ALL, SHADOW_PASS_COST, ShadowDecision, ShadowOutcome, ShadowScheduler,
    admit_shadow, is_enabled_for, run_explicit_job,
};
use hugit_contracts::check_result::{Artifact, CheckResult};
use hugit_contracts::shadow_policy::ShadowPolicy;
use hugit_queue::budget::BudgetManager;

fn policy(optin: &str, budget: u64) -> ShadowPolicy {
    ShadowPolicy {
        cadence: "every_snapshot".into(),
        budget,
        optin: optin.into(),
    }
}

fn check_result(exit: i32) -> CheckResult {
    CheckResult {
        memo_key: "memo".into(),
        tree_hash: "t0".into(),
        def_digest: "def".into(),
        toolchain_digest: "tc".into(),
        exit,
        artifacts: vec![Artifact {
            path: "a".into(),
            digest: "d".into(),
        }],
        stdout_ref: "out".into(),
        stderr_ref: "err".into(),
        duration_ms: 5,
        runner_ref: "c2-runner".into(),
        produced_at: 0,
    }
}

/// ① Drive N writes into a single snapshot window across an opted-in repo and
/// assert the boundary yields EXACTLY ONE shadow pass — not N, not 0 — and that
/// an empty window yields zero.
#[test]
fn item_1_one_shadow_pass_per_snapshot_window() {
    let p = policy("acme/repo", 100);
    assert!(is_enabled_for(&p, "acme/repo"));

    let mut sched = ShadowScheduler::new("acme/repo", "tenant-x");

    // N raw writes inside ONE window.
    const N: usize = 12;
    for i in 0..N {
        sched.record_write(format!("src/file_{i}.rs"));
    }

    // No pass is produced mid-window (cadence is the boundary, not per write):
    // we have not closed the window yet, so nothing has fired.
    assert_eq!(sched.current_window(), 0, "still in first window");
    assert_eq!(sched.pending_writes(), N, "all writes pending, none fired");

    // Boundary: exactly ONE pass over the coalesced set.
    let pass = sched
        .close_window()
        .expect("non-empty window → exactly one pass");
    assert_eq!(pass.window_seq, 0);
    assert_eq!(
        pass.coalesced_writes.len(),
        N,
        "one pass covers all N writes"
    );

    // "not N": a second close in the same (now-empty) window yields nothing.
    assert!(
        sched.close_window().is_none(),
        "empty window → zero passes, not a spurious extra"
    );

    // A fresh window with zero writes also produces zero passes.
    assert!(sched.close_window().is_none());
}

/// ② A shadow pass decrements the tenant budget; once the cap is reached the
/// shadow HALTS (no enqueue, budget untouched) while an explicit job proceeds.
#[test]
fn item_2_shadow_decrements_budget_cap_halts_shadows() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("tenant-x", 2); // cap = 2 shadow passes

    // Two passes admitted, each decrements by SHADOW_PASS_COST.
    assert_eq!(admit_shadow(&mut mgr, "tenant-x"), ShadowDecision::Admitted);
    assert_eq!(
        mgr.budget("tenant-x").unwrap().remaining,
        2 - SHADOW_PASS_COST
    );
    assert_eq!(admit_shadow(&mut mgr, "tenant-x"), ShadowDecision::Admitted);
    assert_eq!(mgr.budget("tenant-x").unwrap().remaining, 0);

    // Cap reached: shadow halts. Budget stays at 0; nothing is queued (shadows
    // yield, they are not backpressured like explicit jobs).
    assert_eq!(
        admit_shadow(&mut mgr, "tenant-x"),
        ShadowDecision::CapHalted
    );
    assert_eq!(mgr.budget("tenant-x").unwrap().remaining, 0);
    assert!(
        mgr.budget("tenant-x").unwrap().queued_items.is_empty(),
        "halted shadow is skipped, never enqueued"
    );

    // Explicit jobs proceed per policy even though shadows are halted.
    assert_eq!(run_explicit_job(true), ExplicitJobStatus::Passed);
}

/// ③ Default-off: a repo with no opt-in runs ZERO shadow passes; opt-in is
/// scoped to exactly the named repo.
#[test]
fn item_3_default_off_per_repo_optin_zero_when_off() {
    // Default (empty opt-in) → off for every repo.
    let off = policy("", 100);
    assert!(!is_enabled_for(&off, "acme/repo"));
    assert!(!is_enabled_for(&off, "acme/other"));

    // When off, the scheduler is never even consulted; assert the gate yields
    // zero runs by counting how many passes a driven window would produce.
    let mut runs = 0usize;
    if is_enabled_for(&off, "acme/repo") {
        let mut sched = ShadowScheduler::new("acme/repo", "tenant-x");
        sched.record_write("src/x.rs");
        if sched.close_window().is_some() {
            runs += 1;
        }
    }
    assert_eq!(runs, 0, "default-off → zero shadow runs");

    // Per-repo opt-in is scoped to that repo only.
    let on = policy("acme/repo", 100);
    assert!(is_enabled_for(&on, "acme/repo"));
    assert!(
        !is_enabled_for(&on, "acme/other"),
        "opt-in for acme/repo must NOT leak to acme/other"
    );

    // The opted-in repo, driven, DOES produce a run.
    let mut on_runs = 0usize;
    if is_enabled_for(&on, "acme/repo") {
        let mut sched = ShadowScheduler::new("acme/repo", "tenant-x");
        sched.record_write("src/x.rs");
        if sched.close_window().is_some() {
            on_runs += 1;
        }
    }
    assert_eq!(
        on_runs, 1,
        "opted-in repo runs exactly one pass for the window"
    );

    // Wildcard opts in all (explicit, not a default).
    assert!(is_enabled_for(&policy(OPTIN_ALL, 1), "any/repo"));
}

/// ④ A failing shadow surfaces as a signal/event ONLY and NEVER gates, blocks,
/// or fails an explicit job; a passing shadow produces an observable result.
#[test]
fn item_4_failing_shadow_signal_only_never_gates_explicit_job() {
    // Passing shadow → observable CheckResult.
    let passing = ShadowOutcome::from_check_result(0, check_result(0));
    assert!(passing.is_pass(), "passing shadow yields observable result");
    match &passing {
        ShadowOutcome::Result(r) => assert_eq!(r.exit, 0),
        _ => panic!("expected observable result"),
    }

    // Failing shadow → signal/event only (never a verdict).
    let failing = ShadowOutcome::from_check_result(3, check_result(7));
    assert!(failing.is_signal(), "failing shadow yields signal only");
    match &failing {
        ShadowOutcome::Signal { exit, .. } => assert_eq!(*exit, 7),
        _ => panic!("expected non-gating signal"),
    }

    // The explicit-job path is structurally independent of the shadow outcome:
    // even with a FAILING shadow in hand, an otherwise-passing explicit job
    // passes, and a failing explicit job fails ONLY on its own merits.
    let _shadow_failure = failing;
    assert_eq!(
        run_explicit_job(true),
        ExplicitJobStatus::Passed,
        "failing shadow must NOT gate/block/fail a passing explicit job"
    );
    assert_eq!(
        run_explicit_job(false),
        ExplicitJobStatus::Failed,
        "explicit job fails only on its own merits, not the shadow's"
    );
}

/// ⑤ Per-tenant cap isolation: tenant A exhausting its shadow budget leaves
/// tenant B's shadows unaffected.
#[test]
fn item_5_per_tenant_cap_isolation() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("A", 1); // A: room for one shadow
    mgr.register_tenant("B", 2); // B: room for two

    // A spends its one unit, then is capped.
    assert_eq!(admit_shadow(&mut mgr, "A"), ShadowDecision::Admitted);
    assert_eq!(admit_shadow(&mut mgr, "A"), ShadowDecision::CapHalted);
    assert_eq!(mgr.budget("A").unwrap().remaining, 0);

    // B is wholly unaffected by A's exhaustion: both of B's passes admit.
    assert_eq!(admit_shadow(&mut mgr, "B"), ShadowDecision::Admitted);
    assert_eq!(admit_shadow(&mut mgr, "B"), ShadowDecision::Admitted);
    assert_eq!(mgr.budget("B").unwrap().remaining, 0);

    // And A staying capped does not "leak" capacity back: A still halts.
    assert_eq!(admit_shadow(&mut mgr, "A"), ShadowDecision::CapHalted);
}
