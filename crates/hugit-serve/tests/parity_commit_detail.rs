//! Parity test for `build_commit_detail` (by-sha → Option). Found/404 + the
//! SECRET-MATRIX guard (a PAT in the charter must redact in the title).

use std::sync::Arc;

use hugit_contracts::IntentSidecar;
use hugit_refstore::EventLog;
use hugit_refstore::intent::import_sidecar;
use hugit_serve::handlers::build_commit_detail;

const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// No git source — these parity tests assert the projection + redaction, not the
/// numstat (which is honest-empty without a git seam).
fn no_git() -> Option<&'static Arc<dyn hugit_proto::ObjectSource + Send + Sync>> {
    None
}

fn sidecar(intent_id: &str, charter: &str) -> IntentSidecar {
    IntentSidecar {
        intent_id: intent_id.to_string(),
        charter: charter.to_string(),
        acceptance: vec![],
        context_ref: String::new(),
        authoritative: false,
    }
}

fn land(log: &mut EventLog, id: &str, charter: &str, sha: &str) {
    import_sidecar(
        log,
        &sidecar(id, charter),
        "refs/hugit/intents",
        sha,
        vec!["agent:opus".to_string()],
        1_000,
    )
    .expect("intent landing is allowed");
}

#[test]
fn found_by_6char_prefix() {
    let mut log = EventLog::new();
    land(
        &mut log,
        "i-1",
        "fix: token expiry",
        "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
    );
    let vm = build_commit_detail(&log, "hugit", "a1b2c3", no_git(), None).expect("prefix matches → Some");
    assert_eq!(vm.sha, "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2");
    assert_eq!(vm.intent_id.as_deref(), Some("i-1"));
    assert!(!vm.external);
    assert_eq!(vm.title, "fix: token expiry");
    assert_eq!(vm.parent_sha, "", "honest default — no git-parent seam");
}

#[test]
fn unknown_sha_is_none_404() {
    let log = EventLog::new();
    assert!(
        build_commit_detail(&log, "hugit", "deadbe", no_git(), None).is_none(),
        "absent → None (404, no leak)"
    );
}

#[test]
fn secret_in_charter_redacts_in_title() {
    let mut log = EventLog::new();
    land(
        &mut log,
        "i-sec",
        &format!("ship {PAT} now"),
        "cafe00cafe00cafe00cafe00cafe00cafe00cafe",
    );
    let vm = build_commit_detail(&log, "hugit", "cafe00", no_git(), None)
        .expect("Some — secret scrubbed, not dropped");
    let json = serde_json::to_string(&vm).unwrap();
    assert!(!json.contains(PAT), "raw PAT must NEVER reach the VM JSON");
    assert_eq!(
        vm.title, "[REDACTED]",
        "secret-shaped title scrubs to the sentinel"
    );
    assert_eq!(
        vm.sha, "cafe00cafe00cafe00cafe00cafe00cafe00cafe",
        "structural sha is NOT scrubbed"
    );
}
