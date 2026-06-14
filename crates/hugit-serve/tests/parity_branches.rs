//! Parity test for `build_branches`. REAL branch/tag counts + rows from
//! `replay`; default-branch pick; head_sha is the 6-char prefix.

use hugit_http_contracts::BranchesVm;
use hugit_refstore::EventLog;
use hugit_refstore::intent::ExternalChangeKind;
use hugit_serve::handlers::build_branches;

/// Append a ref update (the typed cross-crate door to refs/*).
fn ref_update(log: &mut EventLog, ref_name: &str, target: &str, at: u64) {
    log.append_external_change(
        ExternalChangeKind::RefUpdate,
        vec!["agent:opus".to_string()],
        serde_json::json!({ "ref": ref_name, "target": target }).to_string(),
        at,
    );
}

fn populated() -> EventLog {
    let mut log = EventLog::new();
    ref_update(
        &mut log,
        "refs/heads/main",
        "aaaaaabbbbbbccccccddddddeeeeeeff00112233",
        1_000,
    );
    ref_update(
        &mut log,
        "refs/heads/feat/x",
        "1111112222223333334444445555556666667777",
        2_000,
    );
    ref_update(
        &mut log,
        "refs/tags/v1.0.0",
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
        3_000,
    );
    log
}

#[test]
fn empty_log_is_honest_and_round_trips() {
    let vm = build_branches(&EventLog::new(), "hugit");
    assert_eq!(vm.branch_count, 0);
    assert_eq!(vm.tag_count, 0);
    assert!(vm.branches.is_empty());
    let json = serde_json::to_string(&vm).unwrap();
    let back: BranchesVm = serde_json::from_str(&json).unwrap();
    assert_eq!(vm, back);
}

#[test]
fn counts_and_default_are_real() {
    let vm = build_branches(&populated(), "hugit");
    assert_eq!(vm.branch_count, 2, "refs/heads/* only");
    assert_eq!(vm.tag_count, 1, "refs/tags/* only");
    assert_eq!(vm.default_branch.name, "main");
    assert!(vm.default_branch.is_default);
    let main = vm
        .branches
        .iter()
        .find(|r| r.name == "main")
        .expect("main row");
    assert_eq!(main.head_sha, "aaaaaa", "head_sha is the 6-char prefix");
    // honest defaults — no seam.
    assert!(!main.protected);
    assert!(main.pr.is_none());
}
