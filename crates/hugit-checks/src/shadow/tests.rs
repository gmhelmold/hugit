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
#[test]
fn explicit_proceeds_when_shadow_halted() {
    let mut mgr = BudgetManager::default();
    mgr.register_tenant("t1", 0);
    assert_eq!(admit_shadow(&mut mgr, "t1"), ShadowDecision::CapHalted);
    // explicit path is independent of shadow admission
    assert_eq!(run_explicit_job(true), ExplicitJobStatus::Passed);
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
#[test]
fn explicit_independent_of_shadow() {
    // failing shadow present, explicit job still passes on its own merits
    let _shadow = ShadowOutcome::from_check_result(0, check_result(1));
    assert_eq!(run_explicit_job(true), ExplicitJobStatus::Passed);
    // explicit job fails ONLY on its own merits
    assert_eq!(run_explicit_job(false), ExplicitJobStatus::Failed);
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
