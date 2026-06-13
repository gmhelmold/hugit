//! Parity test for `build_pr_detail` (Wave 1 prs/{n} handler).
//!
//! Verifies:
//!   1. An empty log → `build_pr_detail(&log, "hugit", 128)` returns `None`
//!      (no such PR → HTTP 404, `get_opt` semantics, no existence leak).
//!   2. A minimal log carrying ONE real `pr.opened` for PR 128 (constructed via
//!      the public `hugit_cli::pr::open` verb, the same way the engine creates
//!      the event) → `Some(vm)`, whose serialization round-trips losslessly back
//!      into `hugit_http_contracts::PrDetailVm`.
//!   3. The STUB fields equal their honest defaults — never faked: empty diff /
//!      reviewers / labels / branches, and (no PR-altitude envelope on the log)
//!      a ZERO cost block.

use hugit_cli::pr::{AuthorKind, OpenArgs, open};
use hugit_http_contracts::PrDetailVm;
use hugit_refstore::EventLog;
use hugit_serve::handlers::build_pr_detail;

/// A fresh empty `EventLog` — trivially chain-verified (the handler contract:
/// the log is ALREADY verified by the caller).
fn empty_log() -> EventLog {
    EventLog::new()
}

/// Build a minimal log with ONE real `pr.opened` for PR `n` (human author,
/// no campaign, no bundle — all of which pass the open validators on a log with
/// no campaign/intent vocabulary). No PR-altitude envelope is captured, so the
/// cost block must be honest ZERO.
fn log_with_open_pr(n: u32) -> EventLog {
    let mut log = EventLog::new();
    let args = OpenArgs {
        pr_id: n.to_string(),
        campaign: String::new(),
        author_kind: AuthorKind::Human,
        run_id: None,
        principal: Some("human:owner".to_string()),
        intent_ids: vec![],
        recorded_at: 1_700_000_000_000,
    };
    open(&mut log, &args).expect("pr.opened append succeeds on a fresh log");
    log
}

#[test]
fn empty_log_unknown_pr_is_none() {
    let log = empty_log();
    assert!(
        build_pr_detail(&log, "hugit", 128).is_none(),
        "no pr.opened for 128 on an empty log → None (HTTP 404, no existence leak)"
    );
}

#[test]
fn empty_log_none_path_is_total() {
    // The None path is the 404 contract — exercise a few ids to be sure it is a
    // total "not found", not an id-specific quirk.
    let log = empty_log();
    for n in [0u32, 1, 42, 128, u32::MAX] {
        assert!(build_pr_detail(&log, "hugit", n).is_none());
    }
}

#[test]
fn open_pr_round_trips() {
    let log = log_with_open_pr(128);
    let vm = build_pr_detail(&log, "hugit", 128).expect("PR 128 is present → Some(vm)");

    // 1. Serializes without error.
    let json = serde_json::to_string(&vm).expect("PrDetailVm serializes");
    // 2. Re-parses losslessly through the FROZEN contract type.
    let reparsed: PrDetailVm = serde_json::from_str(&json).expect("JSON re-parses into PrDetailVm");
    assert_eq!(vm, reparsed, "round-trip must be lossless");
}

#[test]
fn open_pr_real_fields() {
    let log = log_with_open_pr(128);
    let vm = build_pr_detail(&log, "hugit", 128).unwrap();
    assert_eq!(vm.repo, "hugit");
    assert_eq!(vm.number, 128);
    assert_eq!(
        vm.change_id, "128",
        "change_id is the PR's stable id (REAL)"
    );
    assert_eq!(
        vm.state_label, "proposed",
        "an opened-but-not-queued PR is `proposed` (REAL projection)"
    );
}

#[test]
fn open_pr_no_envelope_cost_is_zero() {
    // No PR-altitude envelope on the log → honest ZERO cost, never faked.
    let log = log_with_open_pr(128);
    let vm = build_pr_detail(&log, "hugit", 128).unwrap();
    assert_eq!(vm.cost.work_usd, 0.0, "no envelope → work_usd 0.0");
    assert_eq!(vm.cost.orchestration_usd, 0.0);
    assert_eq!(vm.cost.verification_usd, 0.0);
    assert_eq!(vm.cost.ci_usd, 0.0);
    assert_eq!(vm.cost.total_usd, 0.0);
    assert_eq!(vm.cost.waste_usd, 0.0);
    assert_eq!(vm.cost.overhead_pct, 0);
    assert_eq!(vm.cost.cache_savings_pct, 0);
    assert_eq!(
        vm.cost.work_note, "",
        "ZERO cost notes are empty, not faked"
    );
}

#[test]
fn open_pr_stub_fields_are_honest_defaults() {
    let log = log_with_open_pr(128);
    let vm = build_pr_detail(&log, "hugit", 128).unwrap();

    // No diffstat seam.
    assert!(vm.diff.files.is_empty(), "diff.files [] — no diffstat seam");
    assert!(vm.diff.hunks.is_empty(), "diff.hunks [] — no diffstat seam");
    assert_eq!(vm.file_count, 0);
    assert_eq!(vm.added, 0);
    assert_eq!(vm.removed, 0);

    // No git-ref tracking.
    assert_eq!(
        vm.source_branch, "",
        "no git-ref tracking → source_branch \"\""
    );
    assert_eq!(
        vm.target_branch, "",
        "no git-ref tracking → target_branch \"\""
    );

    // No reviewer / label / milestone seams.
    assert!(
        vm.reviewers.is_empty(),
        "reviewers [] — no reviewer rail seam"
    );
    assert!(vm.labels.is_empty(), "labels [] — STUB");
    assert!(vm.milestone.is_none(), "milestone None — STUB");
    assert!(vm.conversation.is_empty(), "conversation [] — no Q&A seam");

    // P2 mirror is honest false/empty.
    assert!(!vm.mirror.synced, "mirror.synced false — P2 STUB");
    assert_eq!(vm.mirror.detail, "", "mirror.detail \"\" — P2 STUB");

    // Impact blast-radius is empty (not wired); the checks counts are REAL (0
    // here — no check.recorded on this log).
    assert!(vm.impact.crates_touched.is_empty());
    assert!(vm.impact.direct_dependents.is_empty());
    assert_eq!(vm.impact.checks_selected, 0, "no check.recorded → 0 (REAL)");
    assert_eq!(vm.impact.checks_total, 0);
}
