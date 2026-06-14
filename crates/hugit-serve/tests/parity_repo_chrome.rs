//! Parity test for `build_repo_chrome`. REAL `intents_count` (from
//! `intents_from_log`) + derived `clone_cmd`; everything else is an honest
//! default. No log free-text is echoed, so there is no secret-leak surface here.

use hugit_contracts::IntentSidecar;
use hugit_http_contracts::RepoChromeVm;
use hugit_refstore::EventLog;
use hugit_refstore::intent::import_sidecar;
use hugit_serve::handlers::build_repo_chrome;

fn sidecar(intent_id: &str, charter: &str) -> IntentSidecar {
    IntentSidecar {
        intent_id: intent_id.to_string(),
        charter: charter.to_string(),
        acceptance: vec![],
        context_ref: String::new(),
        authoritative: false,
    }
}

fn land(log: &mut EventLog, id: &str, sha: &str, at: u64) {
    import_sidecar(
        log,
        &sidecar(id, "charter"),
        "refs/hugit/intents",
        sha,
        vec!["agent:opus".to_string()],
        at,
    )
    .expect("intent landing is allowed");
}

#[test]
fn empty_log_is_honest_and_round_trips() {
    let vm = build_repo_chrome(&EventLog::new(), "hugit");
    assert_eq!(vm.intents_count, 0);
    assert_eq!(vm.clone_cmd, "hugit clone hugit");
    assert_eq!(vm.issues_count, 0);
    assert_eq!(vm.stars, "");
    let json = serde_json::to_string(&vm).unwrap();
    let back: RepoChromeVm = serde_json::from_str(&json).unwrap();
    assert_eq!(vm, back);
}

#[test]
fn intents_count_is_real() {
    let mut log = EventLog::new();
    land(
        &mut log,
        "i-0",
        "0000000000000000000000000000000000000001",
        1_000,
    );
    land(
        &mut log,
        "i-1",
        "0000000000000000000000000000000000000002",
        2_000,
    );
    land(
        &mut log,
        "i-2",
        "0000000000000000000000000000000000000003",
        3_000,
    );
    let vm = build_repo_chrome(&log, "humangr/hugit");
    assert_eq!(vm.intents_count, 3, "REAL count of landed intents");
    assert_eq!(vm.clone_cmd, "hugit clone humangr/hugit");
}
