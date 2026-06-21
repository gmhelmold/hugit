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
//! K-SCRUB (Round-7 residual): the `cas:` exemption was itself a hole — ANY
//! `cas:`-prefixed value (e.g. `cas:ghp_…`) was blanket-treated as digest-shaped
//! and stored VERBATIM. That `cas:` exemption is now VALUE-gated too (the payload
//! must be a genuine content-address shape, not a credential), and `--toolchain`
//! AND `--tree-hash` are routed through the structural-secret DOOR (exit-2
//! `secret_in_identifier`) so a secret is refused at input, never persisted. A
//! REAL `cas:<64hex>` tree-hash still passes the door and survives verbatim.
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

// ─────────────────────────────────────────────────────────────────────────────
// THE Round-4 hole #1 — `check --toolchain <secret>` → the `toolchain_digest`
// field. SUPERSEDED by WK-AC: `--toolchain` is a memo AXIS the cache keys on, so
// it CANNOT be scrubbed at rest (scrubbing it would bust every cache hit — the
// recomputed memo_key would not match). The fix is to REJECT a secret-shaped axis
// at the DOOR (exit-2 `secret_in_identifier`), never to store-then-scrub it. So a
// secret `--toolchain` no longer reaches the log/`.ac` at all — it is refused at
// input. (The `.ac` closure is proven end-to-end in
// `acceptance_wj_matrix::check_toolchain_secret_rejected_at_door_and_never_in_ac`.)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn check_toolchain_secret_is_rejected_at_the_door() {
    let dir = scratch("check-tc");
    let log = dir.join("log.json");
    let ac = dir.join("ac");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).unwrap();
    bootstrap_log(log.to_str().unwrap());

    // The PAT is routed RAW into `--toolchain`, the third memo axis. WK-AC rejects
    // a secret-shaped axis at the door rather than persisting+scrubbing it.
    let out = run(&[
        "check",
        "run",
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
    assert_eq!(
        out.status.code(),
        Some(2),
        "a secret --toolchain is rejected at the door (exit 2): {}",
        stdout_of(&out)
    );
    assert!(
        stdout_of(&out).contains("secret_in_identifier"),
        "the door surfaces the structured secret_in_identifier error: {}",
        stdout_of(&out)
    );

    // Nothing was persisted: the PAT is NOWHERE at rest (log absent of it; the
    // `.ac` was never written behind the rejected door).
    let log_bytes = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !log_bytes.contains(PAT),
        "the rejected toolchain secret never reached the --log:\n{log_bytes}"
    );
    let ac_bytes = std::fs::read_to_string(&ac).unwrap_or_default();
    assert!(
        !ac_bytes.contains(PAT),
        "the rejected toolchain secret never reached the .ac:\n{ac_bytes}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// THE Round-4 hole #2 — `verdict --tree-hash <secret>` → the `tree_hash` field.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn verdict_tree_hash_secret_is_rejected_at_the_door() {
    // K-SCRUB: `--tree-hash` is now routed through the SAME structural-secret door
    // as `--id`/`--campaign`/`--toolchain` (it is a content-address identifier
    // reaching the forever-log). A PAT smuggled into it is rejected at the door
    // (exit-2 `secret_in_identifier`) — nothing is persisted. (The central scrub
    // boundary remains the backstop; the door is the clean early rejection.)
    let dir = scratch("verdict-th");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    bootstrap_log(log_s);

    // The PAT is routed RAW into `--tree-hash`, which becomes `tree_hash`.
    let out = run(&[
        "verdict",
        "record",
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
    assert_eq!(
        out.status.code(),
        Some(2),
        "a secret --tree-hash is rejected at the door (exit 2): {}",
        stdout_of(&out)
    );
    assert!(
        stdout_of(&out).contains("secret_in_identifier"),
        "the door surfaces the structured secret_in_identifier error: {}",
        stdout_of(&out)
    );
    // Nothing was persisted: the PAT is NOWHERE at rest (no verdict.recorded row
    // was appended behind the rejected door).
    let log_bytes = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !log_bytes.contains(PAT),
        "the rejected tree-hash secret never reached the --log:\n{log_bytes}"
    );
}

#[test]
fn verdict_legit_cas_tree_hash_survives_verbatim() {
    // K-SCRUB addressability half: a real `cas:<64-hex>` tree-hash passes the door
    // AND survives verbatim in the recorded payload (the content address is
    // load-bearing — the value-gated `cas:` exemption preserves it).
    let dir = scratch("verdict-th-legit");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    bootstrap_log(log_s);

    const CAS64: &str = "cas:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let out = run(&[
        "verdict",
        "record",
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
        CAS64,
    ]);
    assert!(
        out.status.success(),
        "a legit cas: tree-hash records (exit 0): {}",
        stdout_of(&out)
    );

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
        payload["tree_hash"], CAS64,
        "a legit cas:<64hex> tree-hash survives verbatim (addressable): {payload}"
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
        "run",
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
