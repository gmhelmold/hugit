//! Acceptance — WH-IDENT: identifier validation at verb entry.
//!
//! Drives the REAL `hugit` binary (`CARGO_BIN_EXE_hugit`).
//!
//! ## What is proven
//!
//! 1. `campaign open --campaign ''` → `invalid_argument` exit-2 (empty campaign key).
//! 2. `campaign open --owner ''` → `invalid_argument` exit-2 (empty owner).
//! 3. `campaign open --campaign ghp_REALSECRET…` → `secret_in_identifier` exit-2.
//! 4. `campaign open --campaign <40-hex>` SUCCEEDS, then `campaign show --campaign
//!    <same-key>` finds it — the key is stored and addressed verbatim (no
//!    collapse, no `[REDACTED]` — the no-collapse proof required by WH-IDENT).
//! 5. `intent new --id ''` → `invalid_argument` exit-2 (empty explicit id).
//! 6. `intent new --id ghp_…` → `secret_in_identifier` exit-2.
//! 7. `pr open --pr ''` → `invalid_argument` exit-2 (empty PR id).
//! 8. `pr open --campaign ''` → `invalid_argument` exit-2 (empty campaign in pr open).
//! 9. Existing tests pass — no regression in `pc1`/`pc4`/`wb*`/`wf*`/`wg*` suites.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-whident-{tag}-{}-{}",
        std::process::id(),
        tag.len()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `(exit_code, parsed_stdout_json)`.
fn run(args: &[&str]) -> (Option<i32>, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code(), v)
}

// ─────────────────────────────────────────────────────────────────────────────
// campaign open — empty identifiers
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn campaign_open_empty_campaign_key_is_exit2_invalid_argument() {
    let dir = scratch("camp-empty-key");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&[
        "campaign", "open", "--log", log_s, "--campaign", "", "--charter", "c", "--owner",
        "o@h.com",
    ]);
    assert_eq!(
        code,
        Some(2),
        "empty --campaign MUST exit 2 (invalid_argument): {v}"
    );
    assert_eq!(v["error"]["kind"], "invalid_argument", "{v}");
    assert!(v["error"]["fix"].is_string(), "fix hint present: {v}");
    // No log file must have been created from an invalid argument.
    assert!(
        !log.exists(),
        "empty --campaign must not create the log file"
    );
}

#[test]
fn campaign_open_empty_owner_is_exit2_invalid_argument() {
    let dir = scratch("camp-empty-owner");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "my-campaign",
        "--charter",
        "c",
        "--owner",
        "",
    ]);
    assert_eq!(
        code,
        Some(2),
        "empty --owner MUST exit 2 (invalid_argument): {v}"
    );
    assert_eq!(v["error"]["kind"], "invalid_argument", "{v}");
    assert!(v["error"]["fix"].is_string(), "fix hint present: {v}");
}

// ─────────────────────────────────────────────────────────────────────────────
// campaign open — secret-shaped campaign key
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn campaign_open_secret_shaped_campaign_key_is_exit2_secret_in_identifier() {
    let dir = scratch("camp-secret-key");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();

    // A realistically-shaped GitHub PAT (not a real credential).
    let pat = "ghp_16C7e42F292c6912E7710c838347Ae178B4aZ9";

    let (code, v) = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        pat,
        "--charter",
        "auth work",
        "--owner",
        "o@h.com",
    ]);
    assert_eq!(
        code,
        Some(2),
        "a ghp_-shaped --campaign MUST exit 2 (secret_in_identifier): {v}"
    );
    assert_eq!(v["error"]["kind"], "secret_in_identifier", "{v}");
    // The fix hint must mention that identifiers are stored unredacted.
    let fix = v["error"]["fix"].as_str().unwrap_or("");
    assert!(
        fix.contains("unredacted"),
        "fix must mention 'unredacted': {fix}"
    );
    // No log created.
    assert!(!log.exists(), "a secret-shaped key must not create the log");
}

// ─────────────────────────────────────────────────────────────────────────────
// No-collapse proof: 40-hex campaign key is addressed verbatim (WH-IDENT core)
// ─────────────────────────────────────────────────────────────────────────────

/// A 40-hex campaign key (typical content-address shape) must:
///   1. Pass validation (`campaign open` succeeds, exit 0).
///   2. Be stored and returned VERBATIM — no collapse to `[REDACTED]`.
///   3. Be addressable by the EXACT same key in `campaign show` (no rename).
#[test]
fn forty_hex_campaign_key_is_valid_addressable_and_not_collapsed() {
    let dir = scratch("camp-40hex");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();

    // A 40-hex key — a legitimate content-address (git sha shape, no secret prefix).
    let key = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";

    let (code, open_v) = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        key,
        "--charter",
        "content-hash campaign",
        "--owner",
        "o@h.com",
    ]);
    assert_eq!(
        code,
        Some(0),
        "40-hex campaign key MUST succeed (must be allowed as a valid address): {open_v}"
    );
    assert_eq!(
        open_v["campaign"], key,
        "the returned campaign key MUST be the exact 40-hex, not [REDACTED]: {open_v}"
    );
    assert_eq!(open_v["opened"], true, "{open_v}");

    // `campaign show` MUST find the same key — the forever-log stores it verbatim.
    let (show_code, show_v) = run(&["campaign", "show", "--log", log_s, "--campaign", key]);
    assert_eq!(
        show_code,
        Some(0),
        "campaign show with the same 40-hex key MUST exit 0 (no collapse): {show_v}"
    );
    // The projection is non-null, proving the key is addressable.
    assert!(
        show_v.get("campaign").is_some() || show_v.get("progress").is_some(),
        "campaign show must project the campaign (key is addressable): {show_v}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// intent new — explicit --id validation
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn intent_new_empty_explicit_id_is_exit2_invalid_argument() {
    let dir = scratch("intent-empty-id");
    let store = dir.join("store.json");
    let store_s = store.to_str().unwrap();

    let (code, v) = run(&[
        "intent",
        "new",
        "--store",
        store_s,
        "--campaign",
        "my-campaign",
        "--charter",
        "do the thing",
        "--id",
        "",
    ]);
    assert_eq!(
        code,
        Some(2),
        "empty explicit --id MUST exit 2 (invalid_argument): {v}"
    );
    assert_eq!(v["error"]["kind"], "invalid_argument", "{v}");
}

#[test]
fn intent_new_secret_shaped_explicit_id_is_exit2_secret_in_identifier() {
    let dir = scratch("intent-secret-id");
    let store = dir.join("store.json");
    let store_s = store.to_str().unwrap();

    let pat = "ghp_16C7e42F292c6912E7710c838347Ae178B4aZ9";

    let (code, v) = run(&[
        "intent",
        "new",
        "--store",
        store_s,
        "--campaign",
        "my-campaign",
        "--charter",
        "do the thing",
        "--id",
        pat,
    ]);
    assert_eq!(
        code,
        Some(2),
        "a ghp_-shaped explicit --id MUST exit 2 (secret_in_identifier): {v}"
    );
    assert_eq!(v["error"]["kind"], "secret_in_identifier", "{v}");
}

// ─────────────────────────────────────────────────────────────────────────────
// pr open — empty identifiers
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pr_open_empty_pr_id_is_exit2_invalid_argument() {
    let dir = scratch("pr-empty-pr");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        "",
        "--campaign",
        "my-campaign",
        "--author-kind",
        "orchestrator",
        "--run-id",
        "run-1",
    ]);
    assert_eq!(
        code,
        Some(2),
        "empty --pr MUST exit 2 (invalid_argument): {v}"
    );
    assert_eq!(v["error"]["kind"], "invalid_argument", "{v}");
    assert!(v["error"]["fix"].is_string(), "fix hint present: {v}");
}

#[test]
fn pr_open_empty_campaign_is_exit2_invalid_argument() {
    let dir = scratch("pr-empty-camp");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        "PR-1",
        "--campaign",
        "",
        "--author-kind",
        "orchestrator",
        "--run-id",
        "run-1",
    ]);
    assert_eq!(
        code,
        Some(2),
        "empty --campaign on pr open MUST exit 2 (invalid_argument): {v}"
    );
    assert_eq!(v["error"]["kind"], "invalid_argument", "{v}");
}
