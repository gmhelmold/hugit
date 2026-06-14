//! Parity test for `build_commits` (Wave 1 commits handler).
//!
//! Verifies:
//!   1. `build_commits` compiles and returns without panicking on an empty log.
//!   2. The result serializes via `serde_json` and round-trips losslessly back
//!      into `hugit_http_contracts::CommitsVm`.
//!   3. Honest defaults on an empty log: `days` is empty, all branch lists empty.
//!   4. The `repo` field is passed through exactly.

use hugit_http_contracts::CommitsVm;
use hugit_refstore::EventLog;
use hugit_serve::handlers::build_commits;

/// Build a fresh empty `EventLog` — trivially chain-valid per the frozen
/// handler contract ("log is ALREADY verified").
fn empty_log() -> EventLog {
    EventLog::new()
}

// ── round-trip ───────────────────────────────────────────────────────────────

#[test]
fn empty_log_serializes_and_round_trips() {
    let log = empty_log();
    let vm = build_commits(&log, "hugit");

    // 1. Serializes without error.
    let json = serde_json::to_string(&vm).expect("CommitsVm serializes");

    // 2. Re-parses losslessly.
    let reparsed: CommitsVm = serde_json::from_str(&json).expect("JSON re-parses into CommitsVm");
    assert_eq!(vm, reparsed, "round-trip must be lossless");
}

// ── honest empty defaults ────────────────────────────────────────────────────

#[test]
fn empty_log_days_empty() {
    let vm = build_commits(&empty_log(), "hugit");
    assert!(
        vm.days.is_empty(),
        "empty log → days must be [] (no commit rows)"
    );
}

#[test]
fn empty_log_branch_empty_string() {
    let vm = build_commits(&empty_log(), "hugit");
    assert_eq!(vm.branch, "", "empty log → branch must be \"\" (no refs)");
}

#[test]
fn empty_log_other_branches_empty() {
    let vm = build_commits(&empty_log(), "hugit");
    assert!(
        vm.other_branches.is_empty(),
        "empty log → other_branches must be []"
    );
}

#[test]
fn empty_log_generated_branches_empty() {
    let vm = build_commits(&empty_log(), "hugit");
    assert!(
        vm.generated_branches.is_empty(),
        "empty log → generated_branches must be []"
    );
}

// ── repo field passthrough ───────────────────────────────────────────────────

#[test]
fn repo_field_is_exact() {
    let vm = build_commits(&empty_log(), "hugit");
    assert_eq!(vm.repo, "hugit");
}

#[test]
fn repo_field_other_name() {
    let vm = build_commits(&empty_log(), "my-repo");
    assert_eq!(vm.repo, "my-repo");
}

// ── canonical JSON round-trip (Appendix-A fixture) ───────────────────────────

/// The Appendix-A canonical JSON from the contract module must round-trip
/// into a `CommitsVm` identical to the one the contract test already pins —
/// confirming that our type imports are byte-for-field correct.
#[test]
fn canonical_appendix_a_round_trips() {
    let canonical = r#"{
      "repo": "hugit", "branch": "main",
      "other_branches": ["feat/sessions"], "generated_branches": ["intent/a31"],
      "days": [{
        "label": "Commits em 9 de jun de 2026",
        "commits": [{ "message": "fix: sessão expira cedo no refresh", "author": "opus-4.8", "avatar_class": "opus", "age": "há 38 min", "intent_id": "a31", "sha": "a31f9c", "checks_ok": true }]
      }]
    }"#;

    let vm: CommitsVm =
        serde_json::from_str(canonical).expect("Appendix-A JSON parses into CommitsVm");

    // Field spot-checks that the wire types match exactly.
    assert_eq!(vm.repo, "hugit");
    assert_eq!(vm.branch, "main");
    assert_eq!(vm.other_branches, vec!["feat/sessions".to_string()]);
    assert_eq!(vm.generated_branches, vec!["intent/a31".to_string()]);
    assert_eq!(vm.days.len(), 1);
    assert_eq!(vm.days[0].label, "Commits em 9 de jun de 2026");
    assert_eq!(vm.days[0].commits.len(), 1);
    let row = &vm.days[0].commits[0];
    assert_eq!(row.sha, "a31f9c");
    assert!(row.checks_ok);
    assert_eq!(row.intent_id.as_deref(), Some("a31"));

    // Lossless re-serialise → re-parse.
    let reparsed: CommitsVm = serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
    assert_eq!(vm, reparsed, "Appendix-A CommitsVm round-trip is lossless");
}
