//! Parity test for `build_campaign` (by-name → Option). Found/404 + the
//! SECRET-MATRIX guard (a PAT in the campaign charter redacts in `why`).

use hugit_refstore::EventLog;
use hugit_serve::handlers::campaign::build_campaign;

const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn push(log: &mut EventLog, kind: &str, payload: serde_json::Value, at: u64) {
    log.append_for_test(kind, vec!["test".to_string()], payload.to_string(), at);
}

fn seed(log: &mut EventLog, campaign: &str, owner: &str, charter: &str) {
    push(
        log,
        "campaign.opened",
        serde_json::json!({
            "campaign": campaign, "owner": owner, "charter": charter
        }),
        1_000,
    );
    push(
        log,
        "pr.opened",
        serde_json::json!({
            "pr_id": "77", "campaign": campaign, "author_kind": "orchestrator",
            "intent_ids": ["i-a"], "principal": serde_json::Value::Null, "run_id": "r-1"
        }),
        2_000,
    );
    push(
        log,
        "intent.landed",
        serde_json::json!({
            "intent_id": "i-a", "campaign": campaign,
            "charter": "fix token TTL", "deep_link_target": "i-a"
        }),
        3_000,
    );
}

#[test]
fn found_returns_real_fields() {
    let mut log = EventLog::new();
    seed(&mut log, "wave-a", "gustavo", "harden the auth layer");
    let vm = build_campaign(&log, "hugit", "wave-a").expect("present → Some");
    assert_eq!(vm.repo, "hugit");
    assert_eq!(vm.name, "wave-a");
    assert_eq!(vm.operator, "gustavo");
    assert_eq!(vm.why, "harden the auth layer");
    assert_eq!(vm.pr_count, 1);
    assert_eq!(vm.prs.len(), 1);
    assert_eq!(vm.prs[0].number, 77);
    // honest defaults — no seam.
    assert!(!vm.operator_signed);
    assert!(!vm.mirror.synced);
    let json = serde_json::to_string(&vm).unwrap();
    let back: hugit_http_contracts::CampaignVm = serde_json::from_str(&json).unwrap();
    assert_eq!(vm, back, "round-trip lossless");
}

#[test]
fn unknown_name_is_none_404() {
    let mut log = EventLog::new();
    seed(&mut log, "wave-a", "g", "x");
    assert!(
        build_campaign(&log, "hugit", "nope").is_none(),
        "unknown → None (404, no leak)"
    );
    assert!(
        build_campaign(&EventLog::new(), "hugit", "wave-a").is_none(),
        "empty log → None"
    );
}

#[test]
fn secret_in_charter_redacts_in_why() {
    let mut log = EventLog::new();
    seed(&mut log, "wave-sec", "gustavo", &format!("harden: {PAT}"));
    let vm = build_campaign(&log, "hugit", "wave-sec").expect("Some — scrubbed, not dropped");
    let json = serde_json::to_string(&vm).unwrap();
    assert!(!json.contains(PAT), "raw PAT must NEVER reach the VM JSON");
    assert!(
        vm.why.contains("[REDACTED]"),
        "secret-shaped charter scrubs in why, got: {}",
        vm.why
    );
}
