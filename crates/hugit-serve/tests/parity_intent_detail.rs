//! Parity test for `build_intent_detail` (by-id → Option). Found/404 + status
//! from the ledger + the SECRET-MATRIX guard (a PAT in the charter redacts).

use hugit_contracts::IntentSidecar;
use hugit_http_contracts::IntentDetailVm;
use hugit_refstore::EventLog;
use hugit_refstore::intent::import_sidecar;
use hugit_serve::handlers::build_intent_detail;

const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

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
fn found_returns_real_fields() {
    let mut log = EventLog::new();
    land(
        &mut log,
        "a31",
        "fix: refresh TTL",
        "aaaaaa0000000000000000000000000000000000",
    );
    let vm = build_intent_detail(&log, "hugit", "a31").expect("present → Some");
    assert_eq!(vm.id, "a31");
    assert_eq!(vm.repo, "hugit");
    assert_eq!(vm.charter, "fix: refresh TTL");
    assert_eq!(vm.title, "fix: refresh TTL");
    assert_eq!(vm.status, "LANDED", "no verdict → LANDED");
    assert_eq!(vm.commit_hash, "aaaaaa", "structural 6-char prefix");
    // honest defaults when no envelope.
    assert_eq!(vm.metrics.tokens, 0);
    assert!(vm.trajectory.is_empty());
    let json = serde_json::to_string(&vm).unwrap();
    let back: IntentDetailVm = serde_json::from_str(&json).unwrap();
    assert_eq!(vm, back, "round-trip lossless");
}

#[test]
fn unknown_id_is_none_404() {
    let log = EventLog::new();
    assert!(
        build_intent_detail(&log, "hugit", "nope").is_none(),
        "absent → None (404, no leak)"
    );
}

#[test]
fn secret_in_charter_redacts() {
    let mut log = EventLog::new();
    land(
        &mut log,
        "a-sec",
        &format!("auth via {PAT}"),
        "bbbbbb0000000000000000000000000000000000",
    );
    let vm = build_intent_detail(&log, "hugit", "a-sec").expect("Some — scrubbed, not dropped");
    let json = serde_json::to_string(&vm).unwrap();
    assert!(!json.contains(PAT), "raw PAT must NEVER reach the VM JSON");
    assert_eq!(vm.charter, "[REDACTED]");
    assert_eq!(vm.title, "[REDACTED]");
    assert_eq!(vm.id, "a-sec", "structural id is NOT scrubbed");
    assert_eq!(vm.commit_hash, "bbbbbb", "structural sha is NOT scrubbed");
}
