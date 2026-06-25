//! Parity test for `build_insights`. REAL ledger (counts + rows) backbone;
//! empty-log validity; the SECRET-MATRIX guard on the ledger-row charter;
//! spend_proof wiring (P3: cas: ref from raw envelope payloads).

use hugit_cli::pr::{INTENT_ENVELOPE_KIND, PR_ENVELOPE_KIND, PR_OPENED_KIND};
use hugit_http_contracts::InsightsVm;
use hugit_ledger::envelope::cold_ref_for;
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

// ── P3: spend_proof wiring ────────────────────────────────────────────────────

/// Minimal valid ContextEnvelope JSON for use in tests.
/// `altitude` must be "pr" or "intent"; `unit_id` is the PR id or intent id;
/// `campaign` is the campaign key string. The metrics carry non-zero
/// `cost_usd_micros` (5000) so `pr_record` produces a real cost figure and
/// the drill row is pushed (required for the campaign row to appear in
/// cost_xray).
///
/// The PR envelope uses `agent_type: "main"` + `parent_run_id: null` to
/// satisfy the D14 `ensure_top_level` guard inside `pr_record`. The intent
/// envelope uses `agent_type: "implementer"` (a subagent, which is correct
/// for an intent altitude) — `ensure_top_level` is not called on intent
/// envelopes by `pr_record`.
fn make_envelope_json(
    altitude: &str,
    unit_id: &str,
    campaign: &str,
    agent_type: &str,
    parent_run_id: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "1.2.0",
        "altitude": altitude,
        "intent_id": unit_id,
        "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "tree_hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "authorship": {
            "model": "claude-sonnet",
            "model_digest": "0000000000000000000000000000000000000000000000000000000000000000",
            "agent_type": agent_type,
            "spawn": {
                "run_id": "run-p3-test",
                "parent_run_id": parent_run_id,
                "born_at": 1000,
                "died_at": 2000
            },
            "operator": "test@example.com"
        },
        "charter": "test charter for spend proof",
        "campaign": campaign,
        "constraints": [],
        "acceptance": [],
        "parent_intents": [],
        "trajectory": {
            "raw_transcript_ref": null,
            "task_transcript_ref": null,
            "summary": null,
            "journal_ref": null,
            "redaction_policy": "default-v1"
        },
        "snapshot": {
            "files_read": [],
            "prompt_ref": null,
            "env_manifest": "test"
        },
        "metrics": {
            "tokens": { "input": 100, "output": 50, "cache_read": 0, "cache_write": 0, "total": 150 },
            "wall_ms": 1000,
            "active_ms": 500,
            "tool_calls": 0,
            "tool_breakdown": [],
            "model_turns": 1,
            "cost_usd_micros": 5000
        },
        "verdicts_ref": null
    })
}

/// P3: spend_proof is wired on both CostXrayRowVm and LedgerRowVm when the
/// matching envelope records are present in the log.
///
/// Verifies:
/// (1) cost_xray[0].spend_proof == Some(s) where s.starts_with("cas:") and
///     s == cold_ref_for(raw PR envelope payload bytes).
/// (2) ledger.campaigns[0].rows[0].spend_proof == Some(s) where
///     s.starts_with("cas:") and s == cold_ref_for(raw intent envelope payload bytes).
/// (3) Regression: rows from a log without envelope records still yield None
///     (covered by ledger_row_raw_int_cost_fields_are_honest_zero above).
#[test]
fn spend_proof_wired_on_cost_xray_and_ledger_row() {
    let mut log = EventLog::new();

    // campaign.opened
    push(
        &mut log,
        "campaign.opened",
        serde_json::json!({ "campaign": "wave-p3" }),
        1_000,
    );

    // intent.landed — creates the ledger entry for i-p3-001.
    push(
        &mut log,
        "intent.landed",
        serde_json::json!({
            "intent_id": "i-p3-001",
            "campaign": "wave-p3",
            "charter": "implement spend proof plumbing",
            "deep_link_target": "i-p3-001"
        }),
        2_000,
    );

    // pr.opened — PR bundles the intent above.
    push(
        &mut log,
        PR_OPENED_KIND,
        serde_json::json!({
            "pr_id": "pr-p3-001",
            "campaign": "wave-p3",
            "author_kind": "orchestrator",
            "run_id": "run-p3-test",
            "principal": null,
            "intent_ids": ["i-p3-001"]
        }),
        3_000,
    );

    // pr.envelope — altitude "pr", intent_id == pr_id. agent_type "main" is
    // required to pass D14 ensure_top_level inside pr_record.
    let pr_env_val = make_envelope_json("pr", "pr-p3-001", "wave-p3", "main", None);
    let expected_pr_cas = cold_ref_for(pr_env_val.to_string().as_bytes());
    push(&mut log, PR_ENVELOPE_KIND, pr_env_val, 4_000);

    // intent.envelope — altitude "intent", intent_id == i-p3-001.
    // Used by BOTH the intent_spend_map (→ ledger row spend_proof) AND
    // envelopes_for_pr (→ intent_envs for pr_record).
    let intent_env_val = make_envelope_json(
        "intent",
        "i-p3-001",
        "wave-p3",
        "implementer",
        Some("run-p3-test"),
    );
    let expected_intent_cas = cold_ref_for(intent_env_val.to_string().as_bytes());
    push(&mut log, INTENT_ENVELOPE_KIND, intent_env_val, 5_000);

    let vm = build_insights(&log, "hugit");

    // (1) CostXrayRowVm.spend_proof is the cas: ref of the raw PR envelope payload.
    assert_eq!(vm.cost_xray.len(), 1, "one campaign row in cost_xray");
    let xray_row = &vm.cost_xray[0];
    assert!(
        xray_row.spend_proof.is_some(),
        "cost_xray row spend_proof must be Some when a pr.envelope is present"
    );
    let sp = xray_row.spend_proof.as_ref().unwrap();
    assert!(
        sp.starts_with("cas:"),
        "spend_proof must start with 'cas:': {sp}"
    );
    assert_eq!(
        sp, &expected_pr_cas,
        "spend_proof must equal cold_ref_for of the raw PR envelope payload bytes"
    );

    // (2) LedgerRowVm.spend_proof is the cas: ref of the raw intent envelope payload.
    assert_eq!(vm.ledger.campaigns.len(), 1, "one campaign in ledger");
    let ledger_rows = &vm.ledger.campaigns[0].rows;
    let ledger_row = ledger_rows
        .iter()
        .find(|r| r.intent_id == "i-p3-001")
        .expect("ledger row for i-p3-001 must be present");
    assert!(
        ledger_row.spend_proof.is_some(),
        "ledger row spend_proof must be Some when a matching intent.envelope is present"
    );
    let lsp = ledger_row.spend_proof.as_ref().unwrap();
    assert!(
        lsp.starts_with("cas:"),
        "ledger spend_proof must start with 'cas:': {lsp}"
    );
    assert_eq!(
        lsp, &expected_intent_cas,
        "ledger spend_proof must equal cold_ref_for of the raw intent envelope payload bytes"
    );
}
