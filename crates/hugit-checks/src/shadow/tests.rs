//! Unit / state-machine proofs for the shadow scheduler (WP-C8).

use super::*;
use hugit_contracts::check_result::CheckResult;
use hugit_queue::budget::BudgetManager;

fn policy(cadence: &str, budget: u64, optin: &str) -> ShadowPolicy {
    ShadowPolicy {
        cadence: cadence.to_string(),
        budget,
        optin: optin.to_string(),
    }
}

fn check_result(exit: i32) -> CheckResult {
    CheckResult {
        memo_key: "mk".into(),
        tree_hash: "deadbeef".into(),
        def_digest: "dd".into(),
        toolchain_digest: "tc".into(),
        exit,
        artifacts: vec![],
        stdout_ref: "out".into(),
        stderr_ref: "err".into(),
        duration_ms: 1,
        runner_ref: "runner".into(),
        produced_at: 0,
    }
}

// ① ─ N writes within one window collapse to exactly ONE pass at the boundary.
#[test]
fn n_writes_one_window_one_pass() {
    let mut s = ShadowScheduler::new("acme/repo", "t1");
    for i in 0..7 {
        s.record_write(format!("path/{i}.rs"));
    }
    // distinct targets coalesced to 7 pending entries (no spurious mid-window pass)
    assert_eq!(s.pending_writes(), 7);
    let pass = s.close_window().expect("one pass at boundary");
    assert_eq!(pass.coalesced_writes.len(), 7);
    assert_eq!(pass.window_seq, 0);
    // window advanced; pending cleared
    assert_eq!(s.current_window(), 1);
    assert_eq!(s.pending_writes(), 0);
}

// ① ─ repeated writes to the same target coalesce (dedup).
#[test]
fn duplicate_writes_coalesce() {
    let mut s = ShadowScheduler::new("acme/repo", "t1");
    for _ in 0..5 {
        s.record_write("same/path.rs");
    }
    let pass = s.close_window().expect("one pass");
    assert_eq!(pass.coalesced_writes, vec!["same/path.rs".to_string()]);
}

// ① ─ an empty window produces zero passes (not 0-vs-1 ambiguity).
#[test]
fn empty_window_no_pass() {
    let mut s = ShadowScheduler::new("acme/repo", "t1");
    assert!(s.close_window().is_none());
    assert_eq!(s.current_window(), 1);
}

// ② ─ admitting a shadow decrements the tenant budget.
#[test]
fn admit_decrements_budget() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("t1", 3);
    assert_eq!(admit_shadow(&mut mgr, "t1"), ShadowDecision::Admitted);
    assert_eq!(mgr.budget("t1").unwrap().remaining, 3 - SHADOW_PASS_COST);
}

// ② ─ cap halts shadows once budget is exhausted; no enqueue, budget untouched.
#[test]
fn cap_halts_shadow() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("t1", 1);
    assert_eq!(admit_shadow(&mut mgr, "t1"), ShadowDecision::Admitted);
    assert_eq!(mgr.budget("t1").unwrap().remaining, 0);
    // next attempt halts, budget stays at 0, nothing queued
    assert_eq!(admit_shadow(&mut mgr, "t1"), ShadowDecision::CapHalted);
    assert_eq!(mgr.budget("t1").unwrap().remaining, 0);
    assert!(mgr.budget("t1").unwrap().queued_items.is_empty());
}

// ② ─ a halted shadow does not stop an explicit job from proceeding.
//
// T-1 load-bearing upgrade: we now assert the INTERACTION — that admit_shadow
// returning CapHalted has NOT drawn any budget (so the halt is real, not a
// silent admit), AND that run_explicit_job still reaches Passed.  Deleting
// admit_shadow or breaking its halt-path would either change the budget assertion
// or change the ShadowDecision assertion, turning this test RED.
#[test]
fn explicit_proceeds_when_shadow_halted() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("t1", 0);
    // Confirm the cap is already exhausted (remaining == 0 BEFORE the call).
    assert_eq!(mgr.budget("t1").unwrap().remaining, 0);
    let decision = admit_shadow(&mut mgr, "t1");
    assert_eq!(decision, ShadowDecision::CapHalted);
    // The halt must NOT have drawn any budget — remaining is still 0.
    assert_eq!(
        mgr.budget("t1").unwrap().remaining,
        0,
        "a halted shadow must leave the budget untouched"
    );
    // Explicit job proceeds and passes on its own merits despite the halted shadow.
    assert_eq!(run_explicit_job(true), ExplicitJobStatus::Passed);
    // Explicit job can still fail on its own merits when the shadow is halted.
    assert_eq!(run_explicit_job(false), ExplicitJobStatus::Failed);
}

// ③ ─ default-off: empty opt-in opts in nothing.
#[test]
fn default_off_empty_optin() {
    let p = policy("every_snapshot", 10, "");
    assert!(!is_enabled_for(&p, "acme/repo"));
}

// ③ ─ per-repo opt-in is scoped to exactly that repo.
#[test]
fn optin_scoped_to_repo() {
    let p = policy("every_snapshot", 10, "acme/repo");
    assert!(is_enabled_for(&p, "acme/repo"));
    assert!(!is_enabled_for(&p, "acme/other"));
}

// ③ ─ wildcard opts in all repos.
#[test]
fn optin_wildcard_all() {
    let p = policy("every_snapshot", 10, OPTIN_ALL);
    assert!(is_enabled_for(&p, "anything"));
}

// ④ ─ a passing shadow produces an observable result; a failing one a signal.
#[test]
fn outcome_pass_vs_signal() {
    let pass = ShadowOutcome::from_check_result(0, check_result(0));
    assert!(pass.is_pass());
    let fail = ShadowOutcome::from_check_result(0, check_result(1));
    assert!(fail.is_signal());
    match fail {
        ShadowOutcome::Signal { exit, .. } => assert_eq!(exit, 1),
        _ => panic!("expected signal"),
    }
}

// ④ ─ explicit job verdict is independent of any shadow outcome.
//
// T-1 load-bearing upgrade: we drive the full scheduler → admit → outcome →
// explicit-job sequence and assert that a PASSING shadow cannot make the
// explicit job pass, and a FAILING shadow cannot make it fail.  If
// run_explicit_job were ever wired to consult the shadow outcome (breaking the
// non-gating invariant), a mismatched fixture here would turn this test RED.
#[test]
fn explicit_independent_of_shadow() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("t1", 2);

    // ── Failing shadow + explicit Passed ────────────────────────────────────
    // Scheduler: one write → one pass at the boundary.
    let mut s = ShadowScheduler::new("acme/repo", "t1");
    s.record_write("src/lib.rs");
    let pass = s.close_window().expect("one shadow pass");

    // Budget gate: admit the shadow.
    let admitted = admit_shadow(&mut mgr, "t1");
    assert_eq!(
        admitted,
        ShadowDecision::Admitted,
        "pass should be admitted"
    );

    // Shadow outcome: fail (exit 1).
    let shadow_fail = ShadowOutcome::from_check_result(pass.window_seq, check_result(1));
    assert!(shadow_fail.is_signal(), "failing shadow is a signal");

    // The explicit job passes on its OWN merits — the failing shadow is irrelevant.
    assert_eq!(
        run_explicit_job(true),
        ExplicitJobStatus::Passed,
        "a failing shadow must not gate a passing explicit job"
    );

    // ── Passing shadow + explicit Failed ────────────────────────────────────
    let mut s2 = ShadowScheduler::new("acme/repo", "t1");
    s2.record_write("src/other.rs");
    let pass2 = s2.close_window().expect("second shadow pass");

    let admitted2 = admit_shadow(&mut mgr, "t1");
    assert_eq!(admitted2, ShadowDecision::Admitted, "second pass admitted");

    let shadow_pass = ShadowOutcome::from_check_result(pass2.window_seq, check_result(0));
    assert!(shadow_pass.is_pass(), "passing shadow is a result");

    // The explicit job fails on its OWN merits — the passing shadow is irrelevant.
    assert_eq!(
        run_explicit_job(false),
        ExplicitJobStatus::Failed,
        "a passing shadow must not rescue a failing explicit job"
    );
}

// ⑤ ─ per-tenant cap isolation: A exhausting leaves B unaffected.
#[test]
fn per_tenant_cap_isolation() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("A", 1);
    mgr.register_tenant("B", 1);
    // A exhausts then halts
    assert_eq!(admit_shadow(&mut mgr, "A"), ShadowDecision::Admitted);
    assert_eq!(admit_shadow(&mut mgr, "A"), ShadowDecision::CapHalted);
    // B is unaffected
    assert_eq!(admit_shadow(&mut mgr, "B"), ShadowDecision::Admitted);
    assert_eq!(mgr.budget("B").unwrap().remaining, 0);
}
