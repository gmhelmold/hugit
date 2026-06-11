//! WP-WI-SCRUB acceptance — the identifier-field exemption is now STRUCTURAL,
//! over the REAL `hugit` binary (`CARGO_BIN_EXE_hugit`), adversarial Round 5.
//!
//! ## The Round-5 hole (reproduced live by two adversaries)
//!
//! Wave H's contract was "identifier fields {campaign,intent_id,pr_id,run_id} are
//! EXEMPT-from-scrub because validated-at-input (`ident.rs`)". That coupling broke
//! two ways:
//!
//! 1. The input validator was WEAKER than the redaction engine — `ident.rs`'s
//!    reject list omits Slack (`xoxb-`), CoreLink (`clp_`), and `Bearer`, uses
//!    `starts_with` not substring, and does not trim. A Slack token in
//!    `campaign open --campaign <xoxb-…>` PASSES input validation and, under the
//!    blanket exemption, persisted VERBATIM in the hash-chained forever-log.
//! 2. The validator was NOT CALLED on every write path — `check --pr <secret>`
//!    routes the raw flag into `pr_id` (identifier-exempt) but the check verb never
//!    validated it, so a credential persisted verbatim.
//!
//! An exemption that relies on incomplete EXTERNAL validation is a hole.
//!
//! ## The fix (structural, single-source-of-truth — WI-SCRUB)
//!
//! The scrub boundary itself is now safe for identifiers: an identifier value is
//! routed through the engine's STRUCTURAL detectors (known prefixes / `Bearer` /
//! JWT / PEM / connection-string / keyword) but EXEMPT from the bare-hex + entropy
//! scan. So a `xoxb-`/`clp_`/`ghp_`/`Bearer`/conn-string/JWT/PEM in ANY identifier
//! field, for ANY verb, REDACTS at the boundary — no dependence on per-verb
//! validation — while a 40/64-hex content address (or a high-entropy slug id)
//! SURVIVES verbatim (two distinct addresses stay distinct + addressable).
//!
//! These tests drive the real binary end-to-end and prove: (a) a Slack token in
//! `--pr`/`--campaign` no longer persists verbatim (0 verbatim in the forever-log);
//! (b) a 40-hex campaign address survives the round trip; (c) a real `sha256:`/
//! 64-hex digest field still survives (the WH-SCRUB exemption is intact).

use std::path::PathBuf;
use std::process::{Command, Output};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wiscrub-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A realistically-shaped Slack bot token — the EXACT prefix `ident.rs` OMITS, so
/// it passes input validation and is the live leak the structural scrub closes.
/// Never a real credential.
const SLACK: &str = "xoxb-2222222222-3333333333-abcdefghijklmnop";
/// A realistically-shaped GitHub classic PAT.
const GHP: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4aZ9";
const REDACTED: &str = "[REDACTED]";

/// Seed the canonical log with a benign campaign-open (which bootstraps an absent
/// log) so a subsequent `check --store` has a log to append onto.
fn bootstrap_log(log_s: &str) {
    let out = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "seed",
        "--charter",
        "seed",
        "--owner",
        "o@h.com",
    ]);
    assert!(
        out.status.success(),
        "bootstrap campaign open exits 0: {}",
        stdout_of(&out)
    );
}

fn read_log(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Find the payload object of the first record of `kind` on the log.
fn payload_of(log_bytes: &str, kind: &str) -> serde_json::Value {
    let records: serde_json::Value = serde_json::from_str(log_bytes).expect("log is JSON array");
    let row = records
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == kind)
        .unwrap_or_else(|| panic!("a {kind} row is on the log"));
    serde_json::from_str(row["payload"].as_str().unwrap()).expect("payload is JSON")
}

// ─────────────────────────────────────────────────────────────────────────────
// LEAK #1 — `check --pr <secret>` → the `pr_id` identifier field. The check verb
// never validates `--pr`; the blanket exemption persisted the credential VERBATIM.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn check_pr_secret_redacts_in_the_pr_id_field_no_verbatim() {
    let dir = scratch("check-pr");
    let log = dir.join("log.json");
    let ac = dir.join("ac");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).unwrap();
    bootstrap_log(log.to_str().unwrap());

    // The Slack token is routed RAW into `--pr`, which becomes `pr_id`.
    let out = run(&[
        "check",
        "--def",
        "green",
        "--cmd",
        "true",
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--ac",
        ac.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--pr",
        SLACK,
    ]);
    assert!(
        out.status.success(),
        "check --store exits 0: {}",
        stdout_of(&out)
    );

    let bytes = read_log(&log);
    // The forever-log carries the secret NOWHERE verbatim (0 verbatim).
    assert!(
        !bytes.contains(SLACK),
        "a Slack token in --pr must NOT persist verbatim in the forever-log:\n{bytes}"
    );
    // The pr_id field specifically carries the sentinel.
    let payload = payload_of(&bytes, "check.recorded");
    assert_eq!(
        payload["pr_id"], REDACTED,
        "a structural secret smuggled into pr_id is scrubbed at the boundary: {payload}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// LEAK #2 — `campaign open --campaign <xoxb-…>`. `ident.rs` OMITS `xoxb-`, so the
// token passes input validation; the blanket exemption persisted it verbatim.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn campaign_open_slack_token_redacts_in_the_campaign_field_no_verbatim() {
    let dir = scratch("camp-open");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    // The Slack token is the `--campaign` identifier — it passes the weak input
    // validator (which omits xoxb-) and reaches the log.
    let out = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        SLACK,
        "--charter",
        "ship it",
        "--owner",
        "o@h.com",
    ]);
    assert!(
        out.status.success(),
        "campaign open exits 0 (xoxb- passes the weak input validator): {}",
        stdout_of(&out)
    );

    let bytes = read_log(&log);
    assert!(
        !bytes.contains(SLACK),
        "a Slack token in --campaign must NOT persist verbatim in the forever-log:\n{bytes}"
    );
    let payload = payload_of(&bytes, "campaign.opened");
    assert_eq!(
        payload["campaign"], REDACTED,
        "a structural secret in the campaign identifier is scrubbed at the boundary: {payload}"
    );
}

#[test]
fn campaign_open_ghp_token_redacts_if_it_reaches_the_log() {
    // A `ghp_` token: `ident.rs` MAY reject it at input (exit 2). Either way the
    // forever-log must never carry it verbatim — the boundary is the security line.
    let dir = scratch("camp-ghp");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    let out = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        GHP,
        "--charter",
        "x",
        "--owner",
        "o@h.com",
    ]);
    // Whether it was rejected at input OR opened, the log must be clean of the PAT.
    let bytes = read_log(&log);
    assert!(
        !bytes.contains(GHP),
        "a ghp_ token in --campaign must NEVER persist verbatim (exit={:?}):\n{bytes}",
        out.status.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ADDRESS SURVIVAL — a 40-hex campaign address survives the round trip verbatim
// (no collapse), so distinct addresses stay distinct + the flow can look it up.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn forty_hex_campaign_address_survives_the_round_trip() {
    let dir = scratch("camp-hex");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    // A 40-hex content-address used as a campaign key (a legitimate address shape).
    let hex40 = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
    let out = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        hex40,
        "--charter",
        "x",
        "--owner",
        "o@h.com",
    ]);
    assert!(
        out.status.success(),
        "campaign open with a 40-hex address exits 0: {}",
        stdout_of(&out)
    );

    let bytes = read_log(&log);
    assert!(
        bytes.contains(hex40),
        "a 40-hex campaign ADDRESS must survive verbatim (no collapse):\n{bytes}"
    );
    let payload = payload_of(&bytes, "campaign.opened");
    assert_eq!(
        payload["campaign"], hex40,
        "the 40-hex address survives in the campaign field verbatim: {payload}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// DIGEST SURVIVAL — a real 64-hex / `sha256:` digest field is untouched (the
// WH-SCRUB value-gated exemption is intact; WI-SCRUB does not widen into it).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn real_digest_fields_still_survive_alongside_the_structural_scrub() {
    let dir = scratch("digest");
    let log = dir.join("log.json");
    let ac = dir.join("ac");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).unwrap();
    bootstrap_log(log.to_str().unwrap());

    // A clean check computes REAL memo_key / tree_hash 64-hex content addresses.
    let out = run(&[
        "check",
        "--def",
        "green",
        "--cmd",
        "true",
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--ac",
        ac.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--toolchain",
        "tc-fixed",
    ]);
    assert!(
        out.status.success(),
        "clean check --store exits 0: {}",
        stdout_of(&out)
    );

    let bytes = read_log(&log);
    let payload = payload_of(&bytes, "check.recorded");
    for key in ["memo_key", "tree_hash"] {
        let v = payload[key]
            .as_str()
            .unwrap_or_else(|| panic!("{key} present"));
        assert_ne!(v, REDACTED, "real digest `{key}` survives: {payload}");
        assert!(
            v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()),
            "content-address `{key}` survives as verbatim 64-hex: {v}"
        );
    }
}
