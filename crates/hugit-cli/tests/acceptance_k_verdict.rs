//! K-VERDICT acceptance suite — reject-sticky resolution + post-seal guard.
//!
//! Drives the REAL `hugit` binary against the two confirmed integrity defects:
//!
//! 1. **Rejection laundering by lens substitution**: a reject under one lens
//!    MUST NOT be erased by an approve under a different lens name.  The
//!    projection must stay `rejected:1, proven:0`.
//!
//! 2. **Post-seal append allowed**: after `campaign close`, `verdict --store`
//!    must error with `campaign_sealed`/exit-2 and NOT mutate the projection.
//!
//! # Tests
//!
//! A. **Laundering closed**: reject(security) → approve(security2) →
//!    projection still rejected:1, proven:0.
//!
//! B. **Legitimate same-lens override**: reject(security) → approve(security)
//!    [same lens] → projection proven:1, rejected:0.
//!
//! C. **Reject wins over prior approve**: approve(security) → reject(security)
//!    → projection rejected:1, proven:0.
//!
//! D. **Post-seal guard**: campaign close → verdict --store → exit-2,
//!    kind `campaign_sealed`.

use std::path::PathBuf;
use std::process::Command;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-k-verdict-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Bootstrap: open campaign + land intent.  Returns (log_path, store_path).
fn bootstrap(dir: &std::path::Path, campaign: &str, intent_id: &str) -> (PathBuf, PathBuf) {
    let log = dir.join("log.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();

    let out = Command::new(hugit_bin())
        .args([
            "campaign",
            "open",
            "--log",
            log_s,
            "--campaign",
            campaign,
            "--charter",
            "k-verdict test",
            "--owner",
            "test@test.com",
        ])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "campaign open: {}", stdout(&out));

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
            "k-verdict test intent",
            "--id",
            intent_id,
        ])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "intent new: {}", stdout(&out));

    (log, store)
}

fn record_verdict_lens(
    log_s: &str,
    intent_id: &str,
    lens: &str,
    result: &str,
) -> (i32, serde_json::Value) {
    let out = Command::new(hugit_bin())
        .args([
            "verdict", "--log", log_s, "--store", "--intent", intent_id, "--lens", lens,
            "--result", result,
        ])
        .output()
        .expect("hugit runs");
    let code = out.status.code().unwrap_or(-1);
    let v = serde_json::from_str(std::str::from_utf8(&out.stdout).unwrap().trim())
        .unwrap_or(serde_json::Value::Null);
    (code, v)
}

fn campaign_show(log_s: &str, campaign: &str) -> serde_json::Value {
    let out = Command::new(hugit_bin())
        .args(["campaign", "show", "--log", log_s, "--campaign", campaign])
        .output()
        .expect("hugit runs");
    assert!(out.status.success(), "campaign show: {}", stdout(&out));
    serde_json::from_str(std::str::from_utf8(&out.stdout).unwrap().trim())
        .expect("campaign show emits JSON")
}

fn campaign_close(log_s: &str, campaign: &str) -> (i32, serde_json::Value) {
    let out = Command::new(hugit_bin())
        .args(["campaign", "close", "--log", log_s, "--campaign", campaign])
        .output()
        .expect("hugit runs");
    let code = out.status.code().unwrap_or(-1);
    let v = serde_json::from_str(std::str::from_utf8(&out.stdout).unwrap().trim())
        .unwrap_or(serde_json::Value::Null);
    (code, v)
}

fn stdout(out: &std::process::Output) -> &str {
    std::str::from_utf8(&out.stdout).unwrap_or("<utf8 error>")
}

// ── A: Laundering closed ──────────────────────────────────────────────────────

/// K-VERDICT A: reject(security) → approve(security2) → projection STILL
/// rejected:1, proven:0.
///
/// The approve under a novel lens name MUST NOT erase the prior reject from
/// a different lens.  Reject is sticky; a cross-lens approve is not an override.
#[test]
fn a_lens_substitution_laundering_is_closed() {
    let dir = scratch("a-launder");
    let campaign = "camp-k-a";
    let intent_id = "intent-k-a";
    let (log, _store) = bootstrap(&dir, campaign, intent_id);
    let log_s = log.to_str().unwrap();

    // Step 1: security lens rejects.
    let (code, v) = record_verdict_lens(log_s, intent_id, "security", "reject");
    assert_eq!(code, 0, "reject(security) recorded: {v}");
    assert_eq!(
        v["aggregate"], "reject",
        "single-lens reject aggregate: {v}"
    );

    // Step 2: security2 (different lens) approves.
    let (code2, v2) = record_verdict_lens(log_s, intent_id, "security2", "approve");
    assert_eq!(code2, 0, "approve(security2) recorded: {v2}");

    // Projection: security is still rejected; security2 approve CANNOT clear it.
    let show = campaign_show(log_s, campaign);
    let proven = show["ledger"]["proven"].as_u64().unwrap_or(99);
    let rejected = show["ledger"]["rejected"].as_u64().unwrap_or(0);

    assert_eq!(
        proven, 0,
        "K-VERDICT A: approve(security2) MUST NOT clear reject(security) — proven must stay 0: \
         {show}"
    );
    assert!(
        rejected >= 1,
        "K-VERDICT A: rejected must be >= 1 (security lens still rejected): {show}"
    );
}

// ── B: Legitimate same-lens override ─────────────────────────────────────────

/// K-VERDICT B: reject(security) → approve(security) [same lens] → proven:1,
/// rejected:0.
///
/// A same-lens re-approval IS a legitimate override: the same reviewer
/// dimension re-ran and now approves.  This must clear the prior reject.
#[test]
fn b_same_lens_re_approval_is_legitimate_override() {
    let dir = scratch("b-override");
    let campaign = "camp-k-b";
    let intent_id = "intent-k-b";
    let (log, _store) = bootstrap(&dir, campaign, intent_id);
    let log_s = log.to_str().unwrap();

    // Step 1: security lens rejects.
    let (code, v) = record_verdict_lens(log_s, intent_id, "security", "reject");
    assert_eq!(code, 0, "reject(security) recorded: {v}");

    // Step 2: security lens approves (same lens, legitimate re-run).
    let (code2, v2) = record_verdict_lens(log_s, intent_id, "security", "approve");
    assert_eq!(code2, 0, "approve(security) recorded: {v2}");

    // Projection: the same-lens re-approval clears the prior reject.
    let show = campaign_show(log_s, campaign);
    let proven = show["ledger"]["proven"].as_u64().unwrap_or(0);
    let rejected = show["ledger"]["rejected"].as_u64().unwrap_or(99);

    assert!(
        proven >= 1,
        "K-VERDICT B: same-lens approve MUST clear prior reject — proven must be >= 1: {show}"
    );
    assert_eq!(
        rejected, 0,
        "K-VERDICT B: same-lens approve MUST clear prior reject — rejected must be 0: {show}"
    );
}

// ── C: Reject wins over prior approve ────────────────────────────────────────

/// K-VERDICT C: approve(security) → reject(security) → rejected:1, proven:0.
///
/// A later reject on the same lens must win over a prior approve on that lens.
/// Approve-then-reject = rejected (reject is sticky in both directions of the
/// same-lens history).
#[test]
fn c_reject_wins_over_prior_approve() {
    let dir = scratch("c-reject-wins");
    let campaign = "camp-k-c";
    let intent_id = "intent-k-c";
    let (log, _store) = bootstrap(&dir, campaign, intent_id);
    let log_s = log.to_str().unwrap();

    // Step 1: security lens approves.
    let (code, v) = record_verdict_lens(log_s, intent_id, "security", "approve");
    assert_eq!(code, 0, "approve(security) recorded: {v}");

    // Step 2: security lens rejects (later verdict wins for the same lens).
    let (code2, v2) = record_verdict_lens(log_s, intent_id, "security", "reject");
    assert_eq!(code2, 0, "reject(security) recorded: {v2}");

    // Projection: the later reject governs.
    let show = campaign_show(log_s, campaign);
    let proven = show["ledger"]["proven"].as_u64().unwrap_or(99);
    let rejected = show["ledger"]["rejected"].as_u64().unwrap_or(0);

    assert_eq!(
        proven, 0,
        "K-VERDICT C: approve then reject → proven must be 0 (reject wins): {show}"
    );
    assert!(
        rejected >= 1,
        "K-VERDICT C: approve then reject → rejected must be >= 1: {show}"
    );
}

// ── D: Post-seal guard ────────────────────────────────────────────────────────

/// K-VERDICT D: campaign close → verdict --store → exit-2, kind
/// `campaign_sealed`.
///
/// After a campaign is sealed (campaign.closed on the log), any further
/// `verdict --store` for an intent in that campaign MUST error with
/// `campaign_sealed`/exit-2.  The projection MUST NOT be mutated.
#[test]
fn d_post_seal_verdict_is_refused_exit_2() {
    let dir = scratch("d-seal");
    let campaign = "camp-k-d";
    let intent_id = "intent-k-d";
    let (log, _store) = bootstrap(&dir, campaign, intent_id);
    let log_s = log.to_str().unwrap();

    // Record an initial approve so the campaign can close cleanly.
    let (code, v) = record_verdict_lens(log_s, intent_id, "security", "approve");
    assert_eq!(code, 0, "initial approve verdict: {v}");

    // Seal the campaign.
    let (close_code, close_v) = campaign_close(log_s, campaign);
    assert_eq!(
        close_code, 0,
        "campaign close must succeed before the guard test: {close_v}"
    );
    assert_eq!(
        close_v["closed"], true,
        "campaign must be closed: {close_v}"
    );

    // Now attempt a post-close verdict.
    let (seal_code, seal_v) = record_verdict_lens(log_s, intent_id, "security", "reject");
    assert_eq!(
        seal_code, 2,
        "K-VERDICT D: post-close verdict --store MUST exit 2 (campaign_sealed): {seal_v}"
    );
    assert_eq!(
        seal_v["error"]["kind"], "campaign_sealed",
        "K-VERDICT D: error kind must be campaign_sealed: {seal_v}"
    );

    // The projection MUST NOT have been mutated — still proven:1, rejected:0.
    let show = campaign_show(log_s, campaign);
    let proven = show["ledger"]["proven"].as_u64().unwrap_or(0);
    let rejected = show["ledger"]["rejected"].as_u64().unwrap_or(99);
    assert!(
        proven >= 1,
        "K-VERDICT D: sealed campaign verdict must NOT mutate projection — proven stays >= 1: \
         {show}"
    );
    assert_eq!(
        rejected, 0,
        "K-VERDICT D: sealed campaign verdict must NOT mutate projection — rejected stays 0: \
         {show}"
    );
}
