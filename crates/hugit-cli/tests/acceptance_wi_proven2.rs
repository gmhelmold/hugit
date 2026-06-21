//! WP-WI-PROVEN2 acceptance suite — campaign close honest gate + revision
//! coherence end-to-end.
//!
//! Drives the REAL `hugit` binary against two defects confirmed in Round 5:
//!
//! 1. **proven-revision incoherence**: approve-then-REJECT on one intent must
//!    leave proven:0, rejected:1 in campaign show (latest verdict wins; before
//!    the fix: proven:1, rejected:1 — stuck proven on revision).
//!
//! 2. **campaign close doesn't gate on rejected**: `campaign close` must
//!    surface rejected work and refuse with `campaign_has_rejected`/exit-2 by
//!    default; `--allow-rejected` seals over it with `sealed_with_rejected:true`.
//!
//! # Tests
//! A. **revision-coherence** — approve-then-reject → show: proven:0, rejected:1
//! B. **close refuses rejected** — close on a campaign with a rejected intent →
//!    `campaign_has_rejected`/exit-2, NOT a clean all-proven seal
//! C. **close with --allow-rejected** — seals, output carries
//!    `sealed_with_rejected:true`, `rejected_count:1`
//! D. **close clean** — all-approved campaign → clean seal, `sealed_with_rejected:false`
//! E. **revision approve-non-regression** — reject-then-approve → proven:1,
//!    rejected:0; close succeeds cleanly (no rejected_count block)

use std::path::PathBuf;
use std::process::Command;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wi-proven2-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Open a campaign, create an intent, and record a verdict.
/// Returns (log_path, store_path).
fn bootstrap_campaign_with_intent(
    dir: &std::path::Path,
    campaign: &str,
    intent_id: &str,
) -> (PathBuf, PathBuf) {
    let log = dir.join("log.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();

    // Open campaign.
    let out = Command::new(hugit_bin())
        .args([
            "campaign",
            "open",
            "--log",
            log_s,
            "--campaign",
            campaign,
            "--charter",
            "wi-proven2 test campaign",
            "--owner",
            "test@test.com",
        ])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "campaign open succeeded");

    // Land intent.
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
            "wi-proven2 test intent",
            "--id",
            intent_id,
        ])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "intent new succeeded");

    (log, store)
}

fn record_verdict(log_s: &str, intent_id: &str, result: &str) {
    let out = Command::new(hugit_bin())
        .args([
            "verdict", "record", "--log", log_s, "--store", "--intent", intent_id, "--lens",
            "security", "--result", result,
        ])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "verdict recorded");
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
    let v = serde_json::from_str(std::str::from_utf8(&out.stdout).unwrap().trim())
        .unwrap_or(serde_json::Value::Null);
    (code, v)
}

// ── A: revision coherence — approve-then-reject → proven:0, rejected:1 ───────

/// WI-PROVEN2 A: approve-then-REJECT on one intent must leave the campaign
/// showing proven:0, rejected:1 in `campaign show`.
///
/// Pre-fix behaviour: proven:1, rejected:1 (stuck proven on revision).
/// Post-fix behaviour: proven:0, rejected:1 (latest=reject wins, mutually exclusive).
#[test]
fn a_approve_then_reject_latest_reject_wins_in_campaign_show() {
    let dir = scratch("a-a2r");
    let campaign = "camp-wi-a2r";
    let intent_id = "intent-wi-a2r";
    let (log, _store) = bootstrap_campaign_with_intent(&dir, campaign, intent_id);
    let log_s = log.to_str().unwrap();

    // First verdict: approve.
    record_verdict(log_s, intent_id, "approve");

    // Revision: reject.
    record_verdict(log_s, intent_id, "reject");

    // campaign show: latest=reject → proven:0, rejected:1.
    let show = campaign_show(log_s, campaign);
    let proven = show["ledger"]["proven"].as_u64().unwrap_or(99);
    let rejected = show["ledger"]["rejected"].as_u64().unwrap_or(0);

    assert_eq!(
        proven, 0,
        "WI-PROVEN2 A: approve-then-REJECT → proven must be 0 (latest reject wins): {show}"
    );
    assert!(
        rejected >= 1,
        "WI-PROVEN2 A: approve-then-REJECT → rejected must be >= 1: {show}"
    );

    // Explicit mutual-exclusion check: proven count + rejected count must not
    // add up to more than the done count (a stuck proven would give 2 for 1 intent).
    let done = show["ledger"]["done"].as_u64().unwrap_or(0);
    assert!(
        proven + rejected <= done,
        "WI-PROVEN2 A: proven + rejected must not exceed done (mutual exclusion): \
         proven={proven}, rejected={rejected}, done={done}: {show}"
    );
}

// ── B: close refuses campaign with rejected intent ───────────────────────────

/// WI-PROVEN2 B: `campaign close` must refuse with `campaign_has_rejected`/exit-2
/// when a campaign has an intent with a REJECTED latest verdict — NOT silently
/// seal as a clean all-proven campaign.
#[test]
fn b_close_refuses_rejected_intent_exit_2() {
    let dir = scratch("b-close-refused");
    let campaign = "camp-wi-b";
    let intent_id = "intent-wi-b";
    let (log, _store) = bootstrap_campaign_with_intent(&dir, campaign, intent_id);
    let log_s = log.to_str().unwrap();

    // Record a REJECT verdict.
    record_verdict(log_s, intent_id, "reject");

    // campaign close must REFUSE.
    let (code, v) = campaign_close(log_s, campaign, false);
    assert_eq!(
        code, 2,
        "WI-PROVEN2 B: close with rejected intent must exit 2: {v}"
    );
    assert_eq!(
        v["error"]["kind"], "campaign_has_rejected",
        "WI-PROVEN2 B: structured error kind must be campaign_has_rejected: {v}"
    );
    let rejected_count = v["error"]["rejected_count"].as_u64().unwrap_or(0);
    assert!(
        rejected_count >= 1,
        "WI-PROVEN2 B: error must surface rejected_count >= 1: {v}"
    );
    let fix = v["error"]["fix"]
        .as_str()
        .expect("fix hint must be present");
    assert!(
        fix.contains("--allow-rejected"),
        "WI-PROVEN2 B: fix hint must name --allow-rejected: got {fix:?}: {v}"
    );

    // The campaign must NOT be closed (no campaign.closed record).
    let show = campaign_show(log_s, campaign);
    assert_eq!(
        show.get("closed").and_then(|v| v.as_bool()),
        Some(false),
        "WI-PROVEN2 B: campaign must NOT be closed after a refused close: {show}"
    );
}

// ── C: close with --allow-rejected seals, output carries sealed_with_rejected ─

/// WI-PROVEN2 C: `campaign close --allow-rejected` must seal the campaign AND
/// carry `sealed_with_rejected:true`, `rejected_count:N` in the output so
/// downstream tooling sees the non-clean seal.
#[test]
fn c_close_allow_rejected_seals_with_signal() {
    let dir = scratch("c-allow-rejected");
    let campaign = "camp-wi-c";
    let intent_id = "intent-wi-c";
    let (log, _store) = bootstrap_campaign_with_intent(&dir, campaign, intent_id);
    let log_s = log.to_str().unwrap();

    // Record a REJECT verdict.
    record_verdict(log_s, intent_id, "reject");

    // campaign close --allow-rejected must SUCCEED.
    let (code, v) = campaign_close(log_s, campaign, true);
    assert_eq!(
        code, 0,
        "WI-PROVEN2 C: close --allow-rejected must exit 0: {v}"
    );
    assert_eq!(v["closed"], true, "closed must be true: {v}");
    assert_eq!(
        v["sealed_with_rejected"], true,
        "WI-PROVEN2 C: sealed_with_rejected must be true when closing over rejected work: {v}"
    );
    let rejected_count = v["rejected_count"].as_u64().unwrap_or(0);
    assert!(
        rejected_count >= 1,
        "WI-PROVEN2 C: rejected_count must be >= 1 in the close output: {v}"
    );
    // The ledger block must surface the rejected count too.
    assert!(
        v["ledger"]["rejected"].as_u64().unwrap_or(0) >= 1,
        "WI-PROVEN2 C: ledger.rejected must be >= 1 in close output: {v}"
    );
}

// ── D: clean close — all approved → sealed_with_rejected:false ───────────────

/// WI-PROVEN2 D: a campaign where all intents are approved must close cleanly
/// with `sealed_with_rejected:false` and `rejected_count:0`.
#[test]
fn d_clean_close_all_approved_sealed_with_rejected_false() {
    let dir = scratch("d-clean-close");
    let campaign = "camp-wi-d";
    let intent_id = "intent-wi-d";
    let (log, _store) = bootstrap_campaign_with_intent(&dir, campaign, intent_id);
    let log_s = log.to_str().unwrap();

    // Record an APPROVE verdict.
    record_verdict(log_s, intent_id, "approve");

    // campaign close must succeed cleanly.
    let (code, v) = campaign_close(log_s, campaign, false);
    assert_eq!(code, 0, "WI-PROVEN2 D: clean close must exit 0: {v}");
    assert_eq!(v["closed"], true, "closed must be true: {v}");
    assert_eq!(
        v["sealed_with_rejected"], false,
        "WI-PROVEN2 D: sealed_with_rejected must be false for a clean all-approved close: {v}"
    );
    assert_eq!(
        v["rejected_count"].as_u64().unwrap_or(99),
        0,
        "WI-PROVEN2 D: rejected_count must be 0 for a clean close: {v}"
    );
    // proven is surfaced correctly.
    assert!(
        v["ledger"]["proven"].as_u64().unwrap_or(0) >= 1,
        "WI-PROVEN2 D: ledger.proven must be >= 1 for an all-approved close: {v}"
    );
    assert_eq!(
        v["ledger"]["rejected"].as_u64().unwrap_or(99),
        0,
        "WI-PROVEN2 D: ledger.rejected must be 0 for a clean close: {v}"
    );
}

// ── E: revision approve non-regression — reject-then-approve → close succeeds ─

/// WI-PROVEN2 E: reject-then-approve revision — the LATEST verdict is approve.
/// campaign show must report proven:1, rejected:0; close must succeed cleanly
/// (no campaign_has_rejected block).
#[test]
fn e_reject_then_approve_close_succeeds_cleanly() {
    let dir = scratch("e-r2a-close");
    let campaign = "camp-wi-e";
    let intent_id = "intent-wi-e";
    let (log, _store) = bootstrap_campaign_with_intent(&dir, campaign, intent_id);
    let log_s = log.to_str().unwrap();

    // First verdict: reject.
    record_verdict(log_s, intent_id, "reject");

    // Revision: approve.
    record_verdict(log_s, intent_id, "approve");

    // campaign show: latest=approve → proven:1, rejected:0.
    let show = campaign_show(log_s, campaign);
    let proven = show["ledger"]["proven"].as_u64().unwrap_or(0);
    let rejected = show["ledger"]["rejected"].as_u64().unwrap_or(99);
    assert!(
        proven >= 1,
        "WI-PROVEN2 E: reject-then-APPROVE → proven must be >= 1: {show}"
    );
    assert_eq!(
        rejected, 0,
        "WI-PROVEN2 E: reject-then-APPROVE → rejected must be 0 (latest approve wins): {show}"
    );

    // close must succeed cleanly — no campaign_has_rejected block.
    let (code, v) = campaign_close(log_s, campaign, false);
    assert_eq!(
        code, 0,
        "WI-PROVEN2 E: reject-then-APPROVE → close must exit 0 (latest=approve): {v}"
    );
    assert_eq!(v["closed"], true, "closed must be true: {v}");
    assert_eq!(
        v["sealed_with_rejected"], false,
        "WI-PROVEN2 E: sealed_with_rejected must be false (latest=approve, not rejected): {v}"
    );
    assert_eq!(
        v["rejected_count"].as_u64().unwrap_or(99),
        0,
        "WI-PROVEN2 E: rejected_count must be 0 after reject-then-approve: {v}"
    );
}
