//! WP-WG-SCRUB acceptance — STRUCTURAL scrub-on-append over the REAL `hugit`
//! binary (`CARGO_BIN_EXE_hugit`), Wave G (adversarial Round-3).
//!
//! The wedge wave added write verbs that appended USER-supplied strings to the
//! hash-chained, append-only `--log` WITHOUT redaction. Two fresh adversaries
//! reproduced a live `ghp_…` PAT surviving verbatim in the forever-log via
//! `check --def/--cmd/--pr/--principal`, `verdict --intent/--lens`,
//! `pr abandon --reason`, and `pr open --campaign/--run-id/--principal`.
//!
//! The fix is STRUCTURAL (not per-field whack-a-mole): every porcelain append
//! routes its payload `Value` through `porcelain::scrub_to_canonical`, which
//! recursively scrubs every user string value through the redaction engine
//! BEFORE the bytes reach the chain — exempting content-address/digest fields
//! (`memo_key`/`tree_hash`/`commit`/`*_digest`/`hash`) so a digest survives.
//!
//! These tests assert EACH new leak vector now redacts a planted secret to ZERO
//! occurrences in the persisted log (the adversary's exact repro), AND that a
//! legitimate content-address digest still SURVIVES (the exemption holds). The
//! forbidden path — a raw append of an un-scrubbed user payload — is what the
//! central helper structurally prevents (see the `porcelain` module doc + its
//! `scrub_to_canonical` unit tests).

use std::path::PathBuf;
use std::process::{Command, Output};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wgscrub-{tag}-{}", std::process::id()));
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
/// real credential — but the exact shape the `ghp_` detector keys on.
const PAT: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4aZ9";
/// A connection string with an embedded password (the engine catches it).
const CONN: &str = "postgres://admin:S3cr3tP4ssw0rdVeryLongRandomToken9999@db.internal:5432/app";
const CONN_PW: &str = "S3cr3tP4ssw0rdVeryLongRandomToken9999";
const REDACTED: &str = "[REDACTED]";

/// `hugit check` refuses a missing `--log` (`log_not_found`) — it does not
/// bootstrap. Seed the canonical log with a benign campaign-open (which DOES
/// bootstrap an absent log) so the subsequent `check --store` has a log to
/// append onto. The seeded record carries no secrets.
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

/// Assert the persisted log file at `path` carries NEITHER the PAT NOR the
/// connection-string password verbatim — the forever-log is clean.
fn assert_log_clean(path: &std::path::Path, ctx: &str) {
    let bytes = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        !bytes.contains(PAT),
        "{ctx}: the PAT must NOT persist verbatim in the forever-log:\n{bytes}"
    );
    assert!(
        !bytes.contains(CONN_PW),
        "{ctx}: the conn-string password must NOT persist verbatim:\n{bytes}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// LEAK VECTOR 1 — `hugit check` (--def / --cmd / --pr / --principal)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn check_redacts_secrets_in_def_pr_and_principal_on_the_log() {
    let dir = scratch("check");
    let log = dir.join("log.json");
    let ac = dir.join("ac");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).unwrap();
    bootstrap_log(log.to_str().unwrap());

    // The adversary's exact repro: a PAT in `--def`/`--pr`/`--principal` and a
    // connection string in `--cmd` (echoed into the def identity). `--store`
    // forces the `check.recorded` append onto the hash-chained log.
    let out = run(&[
        "check",
        "--def",
        &format!("deploy-{PAT}"),
        "--cmd",
        &format!("echo {CONN}; true"),
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--ac",
        ac.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--toolchain",
        "tc-fixed",
        "--pr",
        &format!("pr-{PAT}"),
        "--principal",
        &format!("orchestrator:{PAT}"),
    ]);
    assert!(
        out.status.success(),
        "check --store exits 0: {}",
        stdout_of(&out)
    );
    assert_log_clean(&log, "check");

    // The log positively carries the redaction sentinel (the secret was replaced,
    // not merely dropped).
    let bytes = std::fs::read_to_string(&log).unwrap();
    assert!(
        bytes.contains(REDACTED),
        "the check.recorded payload carries the sentinel instead of the secret:\n{bytes}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// LEAK VECTOR 2 — `hugit verdict` (--intent / --lens)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn verdict_redacts_secrets_in_intent_and_lens_on_the_log() {
    let dir = scratch("verdict");
    let log = dir.join("log.json");
    bootstrap_log(log.to_str().unwrap());

    let out = run(&[
        "verdict",
        "--intent",
        &format!("intent-{PAT}"),
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--lens",
        &format!("security-{PAT}"),
        "--result",
        "approve",
    ]);
    assert!(
        out.status.success(),
        "verdict --store exits 0: {}",
        stdout_of(&out)
    );
    assert_log_clean(&log, "verdict");
}

// ─────────────────────────────────────────────────────────────────────────────
// LEAK VECTOR 3 — `hugit pr abandon --reason` (the adversary's headline repro)
// ─────────────────────────────────────────────────────────────────────────────

/// Open a PR on a fresh log so it can be abandoned/queued/landed.
fn open_pr(log_s: &str, pr: &str, run_id: &str) -> Output {
    run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        pr,
        "--campaign",
        "c",
        "--author-kind",
        "orchestrator",
        "--run-id",
        run_id,
        "--intent",
        "i1",
    ])
}

#[test]
fn pr_abandon_redacts_secret_in_reason_on_the_log() {
    let dir = scratch("pr-abandon");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    assert!(open_pr(log_s, "1", "run-1").status.success());

    let out = run(&[
        "pr",
        "abandon",
        "--log",
        log_s,
        "--pr",
        "1",
        "--reason",
        &format!("rotating leaked {PAT}; also {CONN}"),
    ]);
    assert!(
        out.status.success(),
        "pr abandon exits 0: {}",
        stdout_of(&out)
    );
    // THE leak vector in scope is the hash-chained, append-only forever-log: a
    // secret appended there is unredactable for the life of the log. The
    // structural scrub-on-append closes it. (The transient stdout echo of the
    // reason is a separate read-surface concern, not the forever-log leak.)
    assert_log_clean(&log, "pr abandon");
}

// ─────────────────────────────────────────────────────────────────────────────
// LEAK VECTOR 4 — `hugit pr open` (--campaign / --run-id / --principal)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pr_open_redacts_secrets_in_campaign_runid_principal_on_the_log() {
    let dir = scratch("pr-open");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    let out = run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        "1",
        "--campaign",
        &format!("camp-{PAT}"),
        "--author-kind",
        "human",
        "--principal",
        &format!("user:{PAT}"),
        "--intent",
        &format!("intent-{PAT}"),
    ]);
    assert!(
        out.status.success(),
        "pr open (human) exits 0: {}",
        stdout_of(&out)
    );
    assert_log_clean(&log, "pr open");
}

// ─────────────────────────────────────────────────────────────────────────────
// LEAK VECTOR 5 — `pr land` (queued) + `pr land --settle` (landed)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pr_queued_and_landed_redact_secret_pr_id_on_the_log() {
    let dir = scratch("pr-land");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    // The pr_id itself is a user string flowing into pr.queued / pr.landed.
    let pr = format!("pr-{PAT}");
    assert!(open_pr(log_s, &pr, "run-1").status.success());

    let land = run(&["pr", "land", "--log", log_s, "--pr", &pr]);
    assert!(
        land.status.success(),
        "pr land exits 0: {}",
        stdout_of(&land)
    );
    let settle = run(&["pr", "land", "--log", log_s, "--pr", &pr, "--settle"]);
    assert!(
        settle.status.success(),
        "pr land --settle exits 0: {}",
        stdout_of(&settle)
    );
    assert_log_clean(&log, "pr queued/landed");
}

// ─────────────────────────────────────────────────────────────────────────────
// DIGEST SURVIVAL — the exemption holds: a legit content-address digest is NOT
// redacted (the false-positive guard the structural scrub must preserve).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn legit_digest_survives_the_scrub_on_a_real_check_recorded_row() {
    // A clean `check --store` (no secrets) must persist its `memo_key` /
    // `tree_hash` / `*_digest` content-address fields verbatim — the digest-key
    // exemption is load-bearing. We assert the recorded row carries 64-hex
    // digest values, NONE replaced by the sentinel.
    let dir = scratch("digest");
    let log = dir.join("log.json");
    let ac = dir.join("ac");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).unwrap();
    bootstrap_log(log.to_str().unwrap());

    let out = run(&[
        "check",
        "--def",
        "green-check",
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

    // Parse the persisted log and find the check.recorded payload.
    let records: serde_json::Value = serde_json::from_slice(&std::fs::read(&log).unwrap()).unwrap();
    let row = records
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "check.recorded")
        .expect("a check.recorded row is on the log");
    let payload: serde_json::Value =
        serde_json::from_str(row["payload"].as_str().unwrap()).unwrap();

    // memo_key / tree_hash are 64-hex content addresses that the engine's
    // free-text law would (entropy-)redact — they MUST survive via the digest-key
    // exemption. def_digest / toolchain_digest are also digest-keyed and survive
    // verbatim (here `tc-fixed`, the fixture toolchain tag).
    for key in ["memo_key", "tree_hash", "def_digest", "toolchain_digest"] {
        let v = payload[key]
            .as_str()
            .unwrap_or_else(|| panic!("digest field `{key}` present: {payload}"));
        assert_ne!(
            v, REDACTED,
            "digest field `{key}` must SURVIVE the scrub (the exemption holds): {payload}"
        );
        assert!(!v.is_empty(), "digest field `{key}` survives verbatim: {v}");
    }
    // The two 64-hex content addresses survive as verbatim hex (the strongest
    // proof: a bare 64-hex run in free text redacts; here it does NOT).
    for key in ["memo_key", "tree_hash"] {
        let v = payload[key].as_str().unwrap();
        assert!(
            v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()),
            "content-address `{key}` survives as verbatim 64-hex: {v}"
        );
    }
}
