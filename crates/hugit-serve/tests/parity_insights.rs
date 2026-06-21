//! Parity test for `build_insights`. REAL ledger (counts + rows) backbone;
//! empty-log validity; the SECRET-MATRIX guard on the ledger-row charter.

use hugit_http_contracts::InsightsVm;
use hugit_refstore::EventLog;
use hugit_serve::handlers::insights::build_insights;

const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// Append a raw record via the test-support door (read-path scrub proven in isolation).
fn push(log: &mut EventLog, kind: &str, payload: serde_json::Value, at: u64) {
    log.append_for_test(kind, vec!["test".to_string()], payload.to_string(), at);
}

#[test]
fn empty_log_is_valid_and_honest() {
    let vm = build_insights(&EventLog::new(), "hugit");
    assert_eq!(vm.repo, "hugit");
    assert!(vm.ledger.campaigns.is_empty(), "no ledger on empty log");
    assert!(vm.cost_xray.is_empty());
    assert!(vm.cost_xray_totals.is_none());
    assert!(vm.tokens_by_campaign.is_empty());
    let json = serde_json::to_string(&vm).unwrap();
    let back: InsightsVm = serde_json::from_str(&json).unwrap();
    assert_eq!(vm, back, "round-trip lossless");
}

#[test]
fn ledger_counts_are_real() {
    let mut log = EventLog::new();
    push(
        &mut log,
        "campaign.opened",
        serde_json::json!({ "campaign": "wave-test" }),
        1_000,
    );
    push(
        &mut log,
        "intent.landed",
        serde_json::json!({
            "intent_id": "i-001", "campaign": "wave-test",
            "charter": "rate-limit per tenant", "deep_link_target": "i-001"
        }),
        2_000,
    );
    push(
        &mut log,
        "intent.landed",
        serde_json::json!({
            "intent_id": "i-002", "campaign": "wave-test",
            "charter": "auth hardening", "deep_link_target": "i-002"
        }),
        3_000,
    );
    push(
        &mut log,
        "verdict.recorded",
        serde_json::json!({
            "intent": "i-002", "tree_hash": "t0", "lens": "correctness",
            "model": "test", "prompt_digest": "d0", "verdict": "approve",
            "claims_checked": ["correctness:approve"], "evidence_refs": []
        }),
        4_000,
    );

    let vm = build_insights(&log, "hugit");
    assert_eq!(vm.ledger.campaigns.len(), 1, "one campaign");
    let c = &vm.ledger.campaigns[0];
    assert_eq!(c.campaign.id, "wave-test");
    assert_eq!(c.asked, 2);
    assert_eq!(c.done, 2);
    assert_eq!(c.proven, 1, "i-002 has an approve verdict");
    assert_eq!(c.rows.len(), 2);
    // KPI derived from real counts.
    let asked_kpi = vm
        .kpis
        .iter()
        .find(|k| k.label == "Intents pousados")
        .expect("kpi present");
    assert_eq!(asked_kpi.value, "2");
}

#[test]
fn secret_in_charter_redacts_in_ledger_row() {
    let mut log = EventLog::new();
    push(
        &mut log,
        "campaign.opened",
        serde_json::json!({ "campaign": "wave-sec" }),
        1_000,
    );
    push(
        &mut log,
        "intent.landed",
        serde_json::json!({
            "intent_id": "i-s", "campaign": "wave-sec",
            "charter": PAT, "deep_link_target": "i-s"
        }),
        2_000,
    );
    let vm = build_insights(&log, "hugit");
    let json = serde_json::to_string(&vm).unwrap();
    assert!(!json.contains(PAT), "raw PAT must NEVER reach the VM JSON");
    let row = &vm.ledger.campaigns[0].rows[0];
    assert_eq!(
        row.asked, "[REDACTED]",
        "secret-shaped charter scrubs in the ledger row"
    );
}

// ── F4a: raw integer fields on insights VM ────────────────────────────────────

/// F4a: on an empty log, the x-ray rows and totals are absent, so there are no
/// raw-int fields to check — but the VM is still valid and round-trips.
#[test]
fn empty_log_raw_int_fields_are_absent() {
    let vm = build_insights(&EventLog::new(), "hugit");
    assert!(vm.cost_xray.is_empty());
    assert!(vm.cost_xray_totals.is_none());
    let json = serde_json::to_string(&vm).unwrap();
    let back: hugit_http_contracts::InsightsVm = serde_json::from_str(&json).unwrap();
    assert_eq!(vm, back);
}

/// F4a: ledger rows built from a log carry honest-zero raw-int cost fields
/// (no cost seam on `intent.landed` events — the ledger view has no USD figures).
#[test]
fn ledger_row_raw_int_cost_fields_are_honest_zero() {
    let mut log = EventLog::new();
    push(
        &mut log,
        "campaign.opened",
        serde_json::json!({ "campaign": "wf4a" }),
        1_000,
    );
    push(
        &mut log,
        "intent.landed",
        serde_json::json!({
            "intent_id": "i-f4a", "campaign": "wf4a",
            "charter": "add feature", "deep_link_target": "i-f4a"
        }),
        2_000,
    );
    let vm = build_insights(&log, "hugit");
    let row = &vm.ledger.campaigns[0].rows[0];
    // The ledger view has no cost seam: all cost fields are honest-zero.
    assert_eq!(row.cost_micros, 0, "cost_micros honest-zero on ledger row");
    assert_eq!(
        row.savings_micros, 0,
        "savings_micros honest-zero on ledger row"
    );
    assert_eq!(
        row.tokens_count, 0,
        "tokens_count honest-zero on ledger row"
    );
    assert_eq!(row.spend_proof, None, "spend_proof None on ledger row");
}
