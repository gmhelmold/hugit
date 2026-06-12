//! WP-WJ-UNIFY acceptance — the ONE structural scrub boundary for identifier
//! fields, proven over the REAL `hugit` binary (`CARGO_BIN_EXE_hugit`), Wave J
//! (adversarial Round-6, Cluster A).
//!
//! Round 6 found Wave I's "single structural scrub boundary" was OVER-CLAIMED:
//! `pr` and `intent --id` BYPASSED the central boundary and pre-scrubbed
//! identifier fields through the FULL free-text engine. That redacted bare
//! 40/64-hex and ULID ADDRESSES, with two CRITICAL consequences:
//!
//!   1. **pr identifier collapse → WRONG-PR landing.** Two distinct 40-hex
//!      `pr_id`s both collapsed to `[REDACTED]`, so `pr land --pr <B>` resolved
//!      to the collapsed `A` record and landed the WRONG PR.
//!   2. **intent `--id` collapse → lost records + un-provable.** A ULID `--id`
//!      redacted; distinct ULID intents collapsed to one (`already_exists`), and
//!      `verdict --intent <real-ULID>` saw `intent_not_found` → `proven` stuck.
//!
//! The fix routes EVERY identifier field through the ONE structural scrub at the
//! central boundary (a prefixed secret REDACTS; a 40/64-hex / ULID / slug
//! ADDRESS SURVIVES verbatim). These tests drive the real binary and pin:
//!   (a) two distinct 40-hex `pr_id`s stay DISTINCT + addressable (no wrong-land);
//!   (b) a ULID `intent new --id` SURVIVES verbatim, is verdict-able, advances
//!       `proven`;
//!   (c) a `xoxb-`/`ghp_` in a `pr`/`intent` identifier REDACTS (0 verbatim);
//!   (d) two distinct ULID intents stay distinct (no `already_exists` collapse).

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wjunify-{tag}-{}", std::process::id()));
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

fn stdout_json(out: &Output) -> Value {
    let s = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(s.trim()).unwrap_or_else(|_| panic!("stdout is JSON: {s}"))
}

/// `[REDACTED]` sentinel — a redacted identifier collapses to this.
const REDACTED: &str = "[REDACTED]";

/// Two distinct 40-hex content addresses (git-sha shape — legitimate ids).
const PR_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PR_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
/// A 40-hex address never opened — landing it must be `unknown_pr`, NOT a
/// resolve-to-A collapse.
const PR_C: &str = "cccccccccccccccccccccccccccccccccccccccc";

/// Two distinct ULIDs (Crockford base32, the canonical intent-id shape).
const ULID_1: &str = "01HQXW000000000000000000A1";
const ULID_2: &str = "01HQXW000000000000000000B2";

fn open_pr(log_s: &str, pr: &str) -> Output {
    run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        pr,
        "--campaign",
        "camp-x",
        "--author-kind",
        "orchestrator",
        "--run-id",
        "run-1",
        "--intent",
        "i1",
    ])
}

// ─────────────────────────────────────────────────────────────────────────────
// (a) pr: two distinct 40-hex pr_ids stay DISTINCT + addressable (no wrong-land).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pr_two_distinct_40hex_pr_ids_do_not_collapse_and_land_their_own() {
    let dir = scratch("pr-no-collapse");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    // Open two distinct 40-hex PRs.
    let a = stdout_json(&open_pr(log_s, PR_A));
    let b = stdout_json(&open_pr(log_s, PR_B));
    assert_eq!(a["pr_id"], PR_A, "PR_A stored verbatim (no collapse): {a}");
    assert_eq!(b["pr_id"], PR_B, "PR_B stored verbatim (no collapse): {b}");
    assert_ne!(a["pr_id"], b["pr_id"], "the two pr_ids must be DISTINCT");

    // The log carries BOTH addresses verbatim, NEITHER collapsed to the sentinel.
    let bytes = std::fs::read_to_string(&log).unwrap();
    assert!(bytes.contains(PR_A), "PR_A on the log verbatim:\n{bytes}");
    assert!(bytes.contains(PR_B), "PR_B on the log verbatim:\n{bytes}");
    assert!(
        !bytes.contains(REDACTED),
        "no address may be redacted to the sentinel:\n{bytes}"
    );

    // Land B → must address B (the original collapse landed A for any --pr).
    let land_b = stdout_json(&run(&["pr", "land", "--log", log_s, "--pr", PR_B]));
    assert_eq!(
        land_b["pr_id"], PR_B,
        "land B must address B, not A: {land_b}"
    );

    // Land an UNOPENED 40-hex C → `unknown_pr`, NOT a resolve-to-A collapse.
    let land_c = run(&["pr", "land", "--log", log_s, "--pr", PR_C]);
    let v = stdout_json(&land_c);
    assert_eq!(
        v["error"]["kind"], "unknown_pr",
        "landing an unopened 40-hex C must be unknown_pr, NOT resolve to A: {v}"
    );

    // pr show resolves each distinctly.
    let show_a = stdout_json(&run(&["pr", "show", "--log", log_s, "--pr", PR_A]));
    let show_b = stdout_json(&run(&["pr", "show", "--log", log_s, "--pr", PR_B]));
    assert_eq!(show_a["pr_id"], PR_A, "show A resolves A: {show_a}");
    assert_eq!(show_b["pr_id"], PR_B, "show B resolves B: {show_b}");
}

// ─────────────────────────────────────────────────────────────────────────────
// (b) intent: a ULID --id SURVIVES verbatim, is verdict-able, advances proven.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn intent_ulid_id_survives_is_verdictable_and_advances_proven() {
    let dir = scratch("intent-ulid");
    let log = dir.join("log.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();

    // A campaign so `proven` has a home (campaign open bootstraps the log).
    let camp = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "camp-i",
        "--owner",
        "alice",
        "--charter",
        "do work",
    ]);
    assert!(camp.status.success(), "campaign open exits 0");

    // intent new --id <ULID> — the id must SURVIVE verbatim on the canonical log.
    let out = run(&[
        "intent",
        "new",
        "--store",
        store_s,
        "--log",
        log_s,
        "--campaign",
        "camp-i",
        "--charter",
        "c1",
        "--id",
        ULID_1,
    ]);
    let v = stdout_json(&out);
    assert!(out.status.success(), "intent new (ULID) exits 0: {v}");
    assert_eq!(
        v["intent_id"], ULID_1,
        "the ULID --id survives verbatim: {v}"
    );
    assert_eq!(v["already_exists"], false, "first landing: {v}");

    let bytes = std::fs::read_to_string(&log).unwrap();
    assert!(
        bytes.contains(ULID_1),
        "the ULID intent_id is on the canonical log verbatim:\n{bytes}"
    );

    // verdict --intent <ULID> must RESOLVE (not intent_not_found) and record.
    let verdict = run(&[
        "verdict", "--log", log_s, "--intent", ULID_1, "--store", "--lens", "security", "--result",
        "approve",
    ]);
    let vv = stdout_json(&verdict);
    assert!(verdict.status.success(), "verdict (ULID) exits 0: {vv}");
    assert_eq!(
        vv["intent"], ULID_1,
        "verdict resolves the ULID intent: {vv}"
    );
    assert_eq!(vv["aggregate"], "approve", "approve aggregate: {vv}");

    // campaign show: the ULID intent advanced to proven (was stuck at 0 on collapse).
    let show = stdout_json(&run(&[
        "campaign",
        "show",
        "--log",
        log_s,
        "--campaign",
        "camp-i",
    ]));
    assert_eq!(
        show["ledger"]["proven"], 1,
        "the ULID intent advanced to proven (no collapse → resolvable): {show}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (c) a prefixed secret in a pr/intent identifier REDACTS (0 verbatim on the log).
// ─────────────────────────────────────────────────────────────────────────────

/// A `xoxb-` Slack-bot token shape — caught by the engine's structural prefix
/// detector but NOT by the `ident.rs` door validator, so it exercises the
/// CENTRAL boundary (the security boundary, per WI-SCRUB).
const XOXB: &str = "xoxb-1111111111-2222222222-aaaaaaaaaaaa";
/// A `ghp_` GitHub PAT shape — caught at the door by `ident.rs` (exit-2) for the
/// explicit `--pr`/`--id`, and by the boundary for free-flowing fields.
const GHP: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4aZ9";

#[test]
fn prefixed_secret_in_pr_identifier_redacts_on_the_log() {
    let dir = scratch("pr-secret");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    // A `xoxb-` in --campaign + --run-id: the door validator misses these, so the
    // central structural boundary MUST redact them.
    let out = run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        "7",
        "--campaign",
        XOXB,
        "--author-kind",
        "orchestrator",
        "--run-id",
        XOXB,
    ]);
    assert!(
        out.status.success(),
        "pr open exits 0: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let bytes = std::fs::read_to_string(&log).unwrap();
    assert!(
        !bytes.contains(XOXB),
        "the xoxb- secret must NOT persist verbatim in campaign/run_id:\n{bytes}"
    );
    assert!(
        bytes.contains(REDACTED),
        "the secret was REDACTED to the sentinel, not merely dropped:\n{bytes}"
    );

    // A `ghp_` in the explicit --pr is rejected at the door (exit-2) — never logged.
    let dir2 = scratch("pr-secret-ghp");
    let log2 = dir2.join("log.json");
    let log2_s = log2.to_str().unwrap();
    let ghp = run(&[
        "pr",
        "open",
        "--log",
        log2_s,
        "--pr",
        GHP,
        "--campaign",
        "c",
        "--author-kind",
        "orchestrator",
        "--run-id",
        "run-1",
    ]);
    let v = stdout_json(&ghp);
    assert_eq!(
        v["error"]["kind"], "secret_in_identifier",
        "a ghp_ --pr is rejected at the door: {v}"
    );
    assert!(
        !log2.exists() || !std::fs::read_to_string(&log2).unwrap().contains(GHP),
        "the rejected ghp_ --pr never reaches the log"
    );
}

#[test]
fn prefixed_secret_in_intent_id_redacts_on_the_log() {
    let dir = scratch("intent-secret");
    let log = dir.join("log.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();

    // A `xoxb-` explicit --id: ident.rs misses this prefix, so the central
    // boundary on the canonical --log MUST redact it (0 verbatim on the log).
    let out = run(&[
        "intent",
        "new",
        "--store",
        store_s,
        "--log",
        log_s,
        "--campaign",
        "camp-i",
        "--charter",
        "c",
        "--id",
        XOXB,
    ]);
    assert!(
        out.status.success(),
        "intent new exits 0: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let bytes = std::fs::read_to_string(&log).unwrap();
    assert!(
        !bytes.contains(XOXB),
        "the xoxb- intent --id must NOT persist verbatim on the canonical log:\n{bytes}"
    );
    assert!(
        bytes.contains(REDACTED),
        "the xoxb- intent_id was REDACTED on the log:\n{bytes}"
    );

    // A `ghp_` explicit --id is rejected at the door (exit-2) — never logged.
    let dir2 = scratch("intent-secret-ghp");
    let log2 = dir2.join("log.json");
    let store2 = dir2.join("store.json");
    let ghp = run(&[
        "intent",
        "new",
        "--store",
        store2.to_str().unwrap(),
        "--log",
        log2.to_str().unwrap(),
        "--campaign",
        "camp-i",
        "--charter",
        "c",
        "--id",
        GHP,
    ]);
    let v = stdout_json(&ghp);
    assert_eq!(
        v["error"]["kind"], "secret_in_identifier",
        "a ghp_ intent --id is rejected at the door: {v}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// (d) two distinct ULID intents stay distinct (no `already_exists` collapse).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn two_distinct_ulid_intents_do_not_collapse() {
    let dir = scratch("intent-distinct");
    let log = dir.join("log.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();

    let camp = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "camp-i",
        "--owner",
        "alice",
        "--charter",
        "w",
    ]);
    assert!(camp.status.success());

    let one = stdout_json(&run(&[
        "intent",
        "new",
        "--store",
        store_s,
        "--log",
        log_s,
        "--campaign",
        "camp-i",
        "--charter",
        "c1",
        "--id",
        ULID_1,
    ]));
    let two = stdout_json(&run(&[
        "intent",
        "new",
        "--store",
        store_s,
        "--log",
        log_s,
        "--campaign",
        "camp-i",
        "--charter",
        "c2",
        "--id",
        ULID_2,
    ]));

    assert_eq!(one["intent_id"], ULID_1, "first ULID verbatim: {one}");
    assert_eq!(two["intent_id"], ULID_2, "second ULID verbatim: {two}");
    // The SECOND must be a genuine first landing — NOT an `already_exists` collapse
    // onto the first (which is what the full-engine pre-scrub produced).
    assert_eq!(
        two["already_exists"], false,
        "the distinct second ULID must NOT collapse to already_exists: {two}"
    );

    // The campaign saw BOTH intents land (done:2 — neither collapsed away).
    let show = stdout_json(&run(&[
        "campaign",
        "show",
        "--log",
        log_s,
        "--campaign",
        "camp-i",
    ]));
    assert_eq!(
        show["ledger"]["done"], 2,
        "both distinct ULID intents landed (no collapse): {show}"
    );
}
