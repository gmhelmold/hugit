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
fn check_redacts_free_text_secrets_in_principal_and_cmd_on_the_log() {
    let dir = scratch("check");
    let log = dir.join("log.json");
    let ac = dir.join("ac");
    let root = dir.join("root");
    std::fs::create_dir_all(&root).unwrap();
    bootstrap_log(log.to_str().unwrap());

    // The adversary's exact repro for the FREE-TEXT vectors: a PAT in
    // `--principal` and a connection string in `--cmd` (echoed into the def
    // identity). `--store` forces the `check.recorded` append onto the
    // hash-chained log.
    //
    // WH-SCRUB note: `--pr` lands in the `pr_id` identifier-ADDRESS field, which
    // is now EXEMPT from the free-text scrub (scrubbing it would collapse distinct
    // addresses) — contract-coupled with WH-IDENT, which validates identifiers at
    // INPUT (rejecting a secret-shaped id) so an address can never carry a known
    // secret. `--pr` therefore no longer carries a planted secret here; the
    // free-text leak vectors below remain the assertion.
    //
    // WK-AC note: `--def` is a memo AXIS (it keys the cache via `def_digest`), so
    // a secret-shaped `--def` is now REJECTED at the door (exit-2), not scrubbed
    // at rest — the door rejection is covered by the WK-AC matrix. So `--def`
    // carries a NON-secret name here; the free-text scrub vector this test owns is
    // `--principal` (the principal-chain string), which remains scrubbed at rest.
    let out = run(&[
        "check",
        "--def",
        "deploy-check",
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
        "pr-42",
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

/// WJ-UNIFY + WJ-INT contract (was: `pr_open_redacts_secrets_in_campaign_runid_principal`).
///
/// The WG-SCRUB era routed `pr open`'s `--campaign`/`--run-id` through the FULL
/// free-text engine, which redacted EVERY high-entropy value — including a bare
/// 40-hex content ADDRESS. That collapsed two distinct addresses to one
/// `[REDACTED]` (the wrong-PR-landing bug). The WI/WJ contract: a PREFIXED secret
/// (`ghp_`/`xoxb-`/conn-string) in an identifier field is handled, but a 40-hex /
/// slug ADDRESS SURVIVES verbatim. This test pins BOTH halves.
///
/// WJ-INT note: `--campaign`/`--run-id` are DOOR-validated identifier fields, so
/// a structurally-secret value is now rejected at the door (exit-2
/// `secret_in_identifier`) BEFORE any write — the secret never reaches the log at
/// all (strictly stronger than redact-at-rest). The contract is door-OR-rest: the
/// secret is rejected at the door OR `[REDACTED]` at rest. Either way 0 verbatim.
#[test]
fn pr_open_redacts_prefixed_secret_but_keeps_address_in_campaign_runid() {
    let dir = scratch("pr-open");

    // (a) A PREFIXED secret in --campaign is now DOOR-rejected (exit-2): the
    // forever-log is never created, so the secret cannot leak. (The door covers
    // the identifier fields it validates; the central scrub boundary covers the
    // rest — see the matrix suite `acceptance_wj_matrix.rs` for the full grid.)
    let log_a = dir.join("log-secret.json");
    let out = run(&[
        "pr",
        "open",
        "--log",
        log_a.to_str().unwrap(),
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
    assert_eq!(
        out.status.code(),
        Some(2),
        "pr open with a prefixed secret in --campaign is DOOR-rejected: {}",
        stdout_of(&out)
    );
    assert!(
        stdout_of(&out).contains("secret_in_identifier"),
        "the door rejection names secret_in_identifier: {}",
        stdout_of(&out)
    );
    // Door-rejected before any write — the log was never created (clean by
    // non-existence). `assert_log_clean` reads an absent file as empty.
    assert_log_clean(&log_a, "pr open (prefixed secret, door-rejected)");

    // (b) A bare 40-hex ADDRESS in --campaign + --run-id SURVIVES verbatim — it
    // is a content address, not a secret; collapsing it broke addressing.
    let log_b = dir.join("log-addr.json");
    let camp_addr = "abcdef0123456789abcdef0123456789abcdef01"; // 40-hex
    let run_addr = "1234567890abcdef1234567890abcdef12345678"; // 40-hex
    let out = run(&[
        "pr",
        "open",
        "--log",
        log_b.to_str().unwrap(),
        "--pr",
        "2",
        "--campaign",
        camp_addr,
        "--author-kind",
        "orchestrator",
        "--run-id",
        run_addr,
        "--intent",
        "i1",
    ]);
    assert!(
        out.status.success(),
        "pr open (orchestrator, 40-hex address) exits 0: {}",
        stdout_of(&out)
    );
    let bytes = std::fs::read_to_string(&log_b).unwrap();
    assert!(
        bytes.contains(camp_addr),
        "the 40-hex campaign address must SURVIVE verbatim (no collapse):\n{bytes}"
    );
    assert!(
        bytes.contains(run_addr),
        "the 40-hex run_id address must SURVIVE verbatim (no collapse):\n{bytes}"
    );
    assert!(
        !bytes.contains(REDACTED),
        "no identifier address may be redacted to the sentinel:\n{bytes}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// LEAK VECTOR 5 — `pr land` (queued) + `pr land --settle` (landed)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pr_queued_and_landed_redact_secret_pr_id_on_the_log() {
    let dir = scratch("pr-land");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    // The pr_id itself is a user string that flows into pr.opened / pr.queued /
    // pr.landed. A structurally-secret pr_id is now DOOR-rejected at `pr open`
    // (WJ-INT: `--pr` is a door-validated identifier), so the secret never
    // reaches the queued/landed records — strictly stronger than redact-at-rest.
    let pr = format!("pr-{PAT}");
    let open = open_pr(log_s, &pr, "run-1");
    assert_eq!(
        open.status.code(),
        Some(2),
        "pr open with a secret-shaped --pr is DOOR-rejected: {}",
        stdout_of(&open)
    );
    assert!(
        stdout_of(&open).contains("secret_in_identifier"),
        "the door rejection names secret_in_identifier: {}",
        stdout_of(&open)
    );
    // The log was never created (door-rejected before any write) — clean.
    assert_log_clean(&log, "pr open (secret pr_id, door-rejected)");

    // And a CLEAN pr_id still flows through queued + landed without leaking the
    // unrelated PAT (the redact-at-rest path for the non-identifier fields is
    // unaffected): open → land → settle, all exit-0, log carries no PAT.
    let clean_pr = "pr-clean-1";
    assert!(open_pr(log_s, clean_pr, "run-1").status.success());
    let land = run(&["pr", "land", "--log", log_s, "--pr", clean_pr]);
    assert!(
        land.status.success(),
        "pr land exits 0: {}",
        stdout_of(&land)
    );
    let settle = run(&["pr", "land", "--log", log_s, "--pr", clean_pr, "--settle"]);
    assert!(
        settle.status.success(),
        "pr land --settle exits 0: {}",
        stdout_of(&settle)
    );
    assert_log_clean(&log, "pr queued/landed (clean id)");
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
