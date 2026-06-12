//! WP-WJ-CLOSE acceptance suite — durable seal-condition audit trail.
//!
//! Drives the REAL `hugit` binary against the Round-6 Cluster D finding:
//! `sealed_with_rejected` / `rejected_count` lived only in stdout, NOT in the
//! `campaign.closed` event payload.  A downstream tool replaying the forever-log
//! could not tell whether a campaign was sealed over rejected work; and an
//! idempotent re-close re-derived the flag from the CURRENT live ledger, so a
//! post-close verdict revision (reject→approve) silently rewrote the historical
//! seal fact.
//!
//! # Tests
//!
//! F. **payload persisted** — the campaign.closed event payload on disk carries
//!    `sealed_with_rejected:true` and `rejected_count:1` after `close
//!    --allow-rejected` over one rejected intent.
//!
//! G. **seal-time immutability** — after a `close --allow-rejected` over a
//!    rejected intent, revising the verdict (reject→approve) does NOT rewrite
//!    the seal fact: idempotent `campaign close` and `campaign show` still
//!    report `sealed_with_rejected:true` / `rejected_count:1` (the seal-time
//!    truth), even though the live ledger now shows proven:1, rejected:0.
//!
//! H. **clean close persists false** — a campaign where all intents are
//!    approved persists `sealed_with_rejected:false` / `rejected_count:0` in
//!    the payload.

use std::path::PathBuf;
use std::process::Command;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wj-close-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn open_campaign(log_s: &str, campaign: &str) {
    let out = Command::new(hugit_bin())
        .args([
            "campaign",
            "open",
            "--log",
            log_s,
            "--campaign",
            campaign,
            "--charter",
            "wj-close test campaign",
            "--owner",
            "test@test.com",
        ])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "campaign open succeeded");
}

fn land_intent(log_s: &str, store_s: &str, campaign: &str, intent_id: &str) {
    let out = Command::new(hugit_bin())
        .args([
            "intent",
            "new",
            "--log",
            log_s,
            "--store",
            store_s,
            "--campaign",
            campaign,
            "--charter",
            "wj-close test intent",
            "--id",
            intent_id,
        ])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "intent new succeeded");
}

fn record_verdict(log_s: &str, intent_id: &str, result: &str) {
    let out = Command::new(hugit_bin())
        .args([
            "verdict", "--log", log_s, "--store", "--intent", intent_id, "--lens", "security",
            "--result", result,
        ])
        .output()
        .expect("hugit runs");
    assert!(
        out.status.success(),
        "verdict {result} succeeded: {}",
        std::str::from_utf8(&out.stdout).unwrap_or("")
    );
}

fn campaign_close(log_s: &str, campaign: &str, allow_rejected: bool) -> (i32, serde_json::Value) {
    let mut args = vec!["campaign", "close", "--log", log_s, "--campaign", campaign];
    if allow_rejected {
        args.push("--allow-rejected");
    }
    let out = Command::new(hugit_bin())
        .args(&args)
        .output()
        .expect("hugit runs");
    let code = out.status.code().unwrap_or(-1);
    let stdout = std::str::from_utf8(&out.stdout)
        .unwrap_or("")
        .trim()
        .to_string();
    let v = serde_json::from_str(&stdout).unwrap_or(serde_json::Value::Null);
    (code, v)
}

fn campaign_show(log_s: &str, campaign: &str) -> serde_json::Value {
    let out = Command::new(hugit_bin())
        .args(["campaign", "show", "--log", log_s, "--campaign", campaign])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "campaign show succeeded");
    serde_json::from_str(std::str::from_utf8(&out.stdout).unwrap().trim())
        .expect("campaign show emits JSON")
}

/// Read the raw log file and find the `campaign.closed` record's payload for
/// `campaign`.  Returns the parsed payload JSON.
fn read_closed_payload(log_s: &str, campaign: &str) -> serde_json::Value {
    let bytes = std::fs::read(log_s).expect("log file is readable");
    let records: Vec<serde_json::Value> =
        serde_json::from_slice(&bytes).expect("log file is valid JSON");
    for record in &records {
        if record.get("kind").and_then(|k| k.as_str()) != Some("campaign.closed") {
            continue;
        }
        // payload is a JSON string nested inside the record
        if let Some(payload_str) = record.get("payload").and_then(|p| p.as_str())
            && let Ok(payload) = serde_json::from_str::<serde_json::Value>(payload_str)
            && payload.get("campaign").and_then(|c| c.as_str()) == Some(campaign)
        {
            return payload;
        }
    }
    panic!("no campaign.closed record found for campaign '{campaign}' in log {log_s}");
}

// ── F: payload persisted ──────────────────────────────────────────────────────

/// WJ-CLOSE F: after `campaign close --allow-rejected` over one rejected intent,
/// the `campaign.closed` event payload on disk MUST carry
/// `"sealed_with_rejected":true` and `"rejected_count":1`.
///
/// This verifies the durable audit trail: a downstream tool replaying the
/// forever-log can determine the seal condition without re-deriving it from the
/// live ledger.
#[test]
fn f_closed_payload_carries_sealed_with_rejected_and_count() {
    let dir = scratch("f-payload");
    let log = dir.join("log.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();
    let campaign = "camp-wj-f";
    let intent_id = "intent-wj-f";

    open_campaign(log_s, campaign);
    land_intent(log_s, store_s, campaign, intent_id);
    record_verdict(log_s, intent_id, "reject");

    let (code, v) = campaign_close(log_s, campaign, true);
    assert_eq!(
        code, 0,
        "WJ-CLOSE F: close --allow-rejected must exit 0: {v}"
    );
    assert_eq!(v["closed"], true, "closed must be true: {v}");
    assert_eq!(
        v["sealed_with_rejected"], true,
        "WJ-CLOSE F: stdout sealed_with_rejected must be true: {v}"
    );
    assert_eq!(
        v["rejected_count"].as_u64().unwrap_or(0),
        1,
        "WJ-CLOSE F: stdout rejected_count must be 1: {v}"
    );

    // The key assertion: the PAYLOAD on disk carries the fields.
    let payload = read_closed_payload(log_s, campaign);
    assert_eq!(
        payload["sealed_with_rejected"], true,
        "WJ-CLOSE F: campaign.closed payload on disk must carry sealed_with_rejected:true: \
         {payload}"
    );
    assert_eq!(
        payload["rejected_count"].as_u64().unwrap_or(0),
        1,
        "WJ-CLOSE F: campaign.closed payload on disk must carry rejected_count:1: {payload}"
    );
}

// ── G: seal-time immutability — post-close verdict revision ──────────────────

/// WJ-CLOSE G: after sealing with `--allow-rejected` over a rejected intent,
/// the seal fact is immutable.  The K-VERDICT post-seal guard enforces this
/// structurally: any post-close `verdict --store` attempt is refused with
/// `campaign_sealed`/exit-2, so the log (and therefore the projection) cannot
/// be mutated after the seal.
///
/// This test verifies:
/// 1. The payload on disk carries `sealed_with_rejected:true` immediately after
///    close (pre-existing WJ-CLOSE assertion).
/// 2. A post-close `verdict --store` attempt is refused with
///    `campaign_sealed`/exit-2 (K-VERDICT post-seal guard — the structural
///    immutability guarantee: rejection is impossible by construction, not just
///    by projection isolation).
/// 3. The idempotent re-close still reports the seal-time truth from the
///    persisted payload, with the live ledger unchanged (the refused revision
///    left the log intact — rejected:1).
#[test]
fn g_post_close_verdict_revision_does_not_rewrite_seal_fact() {
    let dir = scratch("g-immutable");
    let log = dir.join("log.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();
    let campaign = "camp-wj-g";
    let intent_id = "intent-wj-g";

    // Set up: open → intent → reject → close --allow-rejected.
    open_campaign(log_s, campaign);
    land_intent(log_s, store_s, campaign, intent_id);
    record_verdict(log_s, intent_id, "reject");

    let (code, v) = campaign_close(log_s, campaign, true);
    assert_eq!(
        code, 0,
        "WJ-CLOSE G setup: close --allow-rejected must exit 0: {v}"
    );

    // Verify the payload on disk carries the seal condition.
    let payload = read_closed_payload(log_s, campaign);
    assert_eq!(
        payload["sealed_with_rejected"], true,
        "WJ-CLOSE G setup: payload must carry sealed_with_rejected:true: {payload}"
    );
    assert_eq!(
        payload["rejected_count"].as_u64().unwrap_or(0),
        1,
        "WJ-CLOSE G setup: payload must carry rejected_count:1: {payload}"
    );

    // K-VERDICT post-seal guard: a post-close `verdict --store` attempt MUST be
    // refused with `campaign_sealed`/exit-2 — the mutation never reaches the log.
    let post_close_out = Command::new(hugit_bin())
        .args([
            "verdict", "--log", log_s, "--store", "--intent", intent_id, "--lens", "security",
            "--result", "approve",
        ])
        .output()
        .expect("hugit runs");
    let post_close_code = post_close_out.status.code().unwrap_or(-1);
    let post_close_v: serde_json::Value =
        serde_json::from_str(std::str::from_utf8(&post_close_out.stdout).unwrap().trim())
            .unwrap_or(serde_json::Value::Null);
    assert_eq!(
        post_close_code, 2,
        "WJ-CLOSE G: post-close verdict --store MUST exit 2 (campaign_sealed): {post_close_v}"
    );
    assert_eq!(
        post_close_v["error"]["kind"], "campaign_sealed",
        "WJ-CLOSE G: error kind must be campaign_sealed: {post_close_v}"
    );

    // Idempotent close: the seal-time truth is preserved from the persisted payload.
    // The refused revision left the log intact, so the live ledger still shows rejected:1.
    let (code2, v2) = campaign_close(log_s, campaign, false);
    assert_eq!(code2, 0, "WJ-CLOSE G: idempotent close must exit 0: {v2}");
    assert_eq!(v2["already_closed"], true, "must be already_closed: {v2}");
    assert_eq!(
        v2["sealed_with_rejected"], true,
        "WJ-CLOSE G: idempotent close must report seal-time sealed_with_rejected:true: {v2}"
    );
    assert_eq!(
        v2["rejected_count"].as_u64().unwrap_or(0),
        1,
        "WJ-CLOSE G: idempotent close must report seal-time rejected_count:1: {v2}"
    );

    // The live ledger is unchanged: the post-close revision was blocked.
    // rejected:1 because the log was never mutated.
    assert_eq!(
        v2["ledger"]["proven"].as_u64().unwrap_or(99),
        0,
        "WJ-CLOSE G: live ledger.proven must be 0 (revision was blocked, log unchanged): {v2}"
    );
    assert!(
        v2["ledger"]["rejected"].as_u64().unwrap_or(0) >= 1,
        "WJ-CLOSE G: live ledger.rejected must be >= 1 (revision was blocked, log unchanged): {v2}"
    );

    // campaign show must also report the seal-time truth for the seal fact.
    let show = campaign_show(log_s, campaign);
    assert_eq!(
        show["sealed_with_rejected"], true,
        "WJ-CLOSE G: campaign show must report seal-time sealed_with_rejected:true: {show}"
    );
    assert_eq!(
        show["seal_rejected_count"].as_u64().unwrap_or(0),
        1,
        "WJ-CLOSE G: campaign show must report seal-time seal_rejected_count:1: {show}"
    );
}

// ── H: clean close persists false ────────────────────────────────────────────

/// WJ-CLOSE H: a campaign where all intents are approved must persist
/// `sealed_with_rejected:false` and `rejected_count:0` in the
/// `campaign.closed` payload.
#[test]
fn h_clean_close_persists_sealed_with_rejected_false() {
    let dir = scratch("h-clean");
    let log = dir.join("log.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();
    let campaign = "camp-wj-h";
    let intent_id = "intent-wj-h";

    open_campaign(log_s, campaign);
    land_intent(log_s, store_s, campaign, intent_id);
    record_verdict(log_s, intent_id, "approve");

    let (code, v) = campaign_close(log_s, campaign, false);
    assert_eq!(code, 0, "WJ-CLOSE H: clean close must exit 0: {v}");
    assert_eq!(v["closed"], true, "closed must be true: {v}");
    assert_eq!(
        v["sealed_with_rejected"], false,
        "WJ-CLOSE H: stdout sealed_with_rejected must be false: {v}"
    );
    assert_eq!(
        v["rejected_count"].as_u64().unwrap_or(99),
        0,
        "WJ-CLOSE H: stdout rejected_count must be 0: {v}"
    );

    // The payload on disk must also carry the clean-seal condition.
    let payload = read_closed_payload(log_s, campaign);
    assert_eq!(
        payload["sealed_with_rejected"], false,
        "WJ-CLOSE H: campaign.closed payload on disk must carry sealed_with_rejected:false: \
         {payload}"
    );
    assert_eq!(
        payload["rejected_count"].as_u64().unwrap_or(99),
        0,
        "WJ-CLOSE H: campaign.closed payload on disk must carry rejected_count:0: {payload}"
    );
}
