//! WP-WH-SCRUB acceptance — the digest exemption is VALUE-gated, over the REAL
//! `hugit` binary (`CARGO_BIN_EXE_hugit`), adversarial Round 4.
//!
//! Round 3's structural scrub-on-append (WG-SCRUB) exempted digest fields by KEY
//! NAME — `memo_key`/`tree_hash`/`commit`/`*_digest`/`hash`. Two fresh adversaries
//! reproduced a live leak THROUGH that exemption: `hugit check --toolchain
//! <ghp_…>` routes a RAW user flag into the `toolchain_digest` field and `hugit
//! verdict --tree-hash <ghp_…>` into the `tree_hash` field — both digest-NAMED,
//! both key-exempt, so the secret persisted VERBATIM in the hash-chained,
//! append-only forever-log (same class as the Round-2 hex leak: an exemption is a
//! hole).
//!
//! The fix is VALUE-GATED: a digest-named field is exempt from the scrub ONLY IF
//! its value is actually digest-shaped (64/40-hex or a `sha256:`/`cas:`/`<algo>:`
//! content-address prefix). A secret-shaped value in a digest-named field is NOT
//! exempt → it scrubs like any other value. A REAL content address still survives.
//!
//! These tests drive the real binary and assert: the smuggled secret no longer
//! persists verbatim in the log; the binary's own real `memo_key`/`tree_hash`
//! content addresses still survive (the false-positive guard holds). The Wave-G
//! `acceptance_wg_scrub.rs` suite (the 5 leak vectors + digest survival) stays
//! green alongside this one.

use std::path::PathBuf;
use std::process::{Command, Output};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-whscrub-{tag}-{}", std::process::id()));
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

/// A realistically-shaped GitHub classic PAT (the adversary's specimen). Never a
/// real credential — the exact shape the `ghp_` detector keys on.
const PAT: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4aZ9";
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

/// Assert the persisted log file carries the PAT NOWHERE verbatim — the
/// forever-log is clean of the smuggled secret.
fn assert_no_pat(path: &std::path::Path, ctx: &str) {
    let bytes = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        !bytes.contains(PAT),
        "{ctx}: the PAT smuggled into a digest-named field must NOT persist \
         verbatim in the forever-log:\n{bytes}"
    );
    assert!(
        bytes.contains(REDACTED),
        "{ctx}: the smuggled secret is replaced by the sentinel, not dropped:\n{bytes}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// THE Round-4 hole #1 — `check --toolchain <secret>` → the `toolchain_digest`
// field. A digest-NAMED field; key-exempt under WG-SCRUB; the raw flag leaked.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn check_toolchain_secret_redacts_in_the_toolchain_digest_field() {
    let dir = scratch("check-tc");
    let log = dir.join("log.json");
    let ac = dir.join("ac");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).unwrap();
    bootstrap_log(log.to_str().unwrap());

    // The PAT is routed RAW into `--toolchain`, which becomes `toolchain_digest`.
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
        PAT,
    ]);
    assert!(
        out.status.success(),
        "check --store exits 0: {}",
        stdout_of(&out)
    );
    assert_no_pat(&log, "check --toolchain");

    // The persisted toolchain_digest field specifically carries the sentinel.
    let records: serde_json::Value = serde_json::from_slice(&std::fs::read(&log).unwrap()).unwrap();
    let row = records
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "check.recorded")
        .expect("a check.recorded row is on the log");
    let payload: serde_json::Value =
        serde_json::from_str(row["payload"].as_str().unwrap()).unwrap();
    assert_eq!(
        payload["toolchain_digest"], REDACTED,
        "a secret smuggled into toolchain_digest is scrubbed, not key-exempt: {payload}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// THE Round-4 hole #2 — `verdict --tree-hash <secret>` → the `tree_hash` field.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn verdict_tree_hash_secret_redacts_in_the_tree_hash_field() {
    let dir = scratch("verdict-th");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    bootstrap_log(log_s);

    // The PAT is routed RAW into `--tree-hash`, which becomes `tree_hash`.
    let out = run(&[
        "verdict",
        "--intent",
        "i1",
        "--log",
        log_s,
        "--store",
        "--lens",
        "security",
        "--result",
        "approve",
        "--tree-hash",
        PAT,
    ]);
    assert!(
        out.status.success(),
        "verdict --store exits 0: {}",
        stdout_of(&out)
    );
    assert_no_pat(&log, "verdict --tree-hash");

    let records: serde_json::Value = serde_json::from_slice(&std::fs::read(&log).unwrap()).unwrap();
    let row = records
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "verdict.recorded")
        .expect("a verdict.recorded row is on the log");
    let payload: serde_json::Value =
        serde_json::from_str(row["payload"].as_str().unwrap()).unwrap();
    assert_eq!(
        payload["tree_hash"], REDACTED,
        "a secret smuggled into tree_hash is scrubbed, not key-exempt: {payload}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// THE EXEMPTION STILL HOLDS — a clean `check --store` (no secret) persists its
// REAL content-address digests verbatim. Value-gating must not over-redact.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn real_content_address_digests_still_survive_value_gating() {
    let dir = scratch("survive");
    let log = dir.join("log.json");
    let ac = dir.join("ac");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).unwrap();
    bootstrap_log(log.to_str().unwrap());

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

    let records: serde_json::Value = serde_json::from_slice(&std::fs::read(&log).unwrap()).unwrap();
    let row = records
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "check.recorded")
        .expect("a check.recorded row is on the log");
    let payload: serde_json::Value =
        serde_json::from_str(row["payload"].as_str().unwrap()).unwrap();

    // memo_key / tree_hash are REAL 64-hex content addresses computed by the
    // binary — they MUST survive the value-gated exemption verbatim.
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
