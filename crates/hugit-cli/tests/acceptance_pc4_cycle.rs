//! Acceptance — WP-PC4: the porcelain verbs COMPOSE on one canonical log.
//!
//! The lead's cross-verb smoke found the porcelain verbs did not compose:
//! `campaign open` wrote a MAP-shaped world document, `pr open` expected a
//! `[EventRecord, …]` SEQUENCE, and `intent new` never touched the shared log —
//! so `pr open --intent` could not see intents and `campaign close` sealed an
//! empty campaign. PC4 makes the `--log` file ONE canonical on-disk seam (the
//! engine's `hugit_refstore::EventLog` shape: a hash-chained `[EventRecord, …]`
//! array) that every verb reads and writes.
//!
//! This suite drives the REAL `hugit` binary (`CARGO_BIN_EXE_hugit`) through the
//! full cross-verb cycle on a single shared log file, and pins the exact repro
//! as a regression (a map-shaped log is rejected, never silently accepted).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

/// Per-process call counter — prevents name collisions when multiple tests in
/// this suite call `scratch()` within the same process (or on a fast re-run
/// that recycles the same pid). The pid anchors cross-process isolation; the
/// counter anchors intra-process call-site isolation. Both are combined so no
/// two `scratch()` calls ever share a directory, even under parallel test
/// execution.
///
/// Cleanup is best-effort: the `remove_dir_all` at the START of each call
/// clears any stale dir from a previous run with the same pid+counter (which
/// would only occur after a pid wrap, i.e., extremely rarely). A test failure
/// may leave the dir behind for post-mortem inspection; that is intentional.
static SCRATCH_CTR: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = SCRATCH_CTR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("hugit-pc4-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit <args>` and return `(exit_success, parsed_stdout_json, stderr)`.
fn run(args: &[&str]) -> (bool, Value, String) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.success(), v, stderr)
}

const CAMPAIGN: &str = "auth-hardening";

fn count_kind(path: &Path, kind: &str) -> usize {
    let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    v.as_array()
        .expect("the canonical log is a JSON [EventRecord, …] array")
        .iter()
        .filter(|e| e["kind"] == kind)
        .count()
}

// ─────────────────────────────────────────────────────────────────────────────
// The cross-verb cycle on ONE shared log.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cross_verb_cycle_composes_on_one_canonical_log() {
    let dir = scratch("cycle");
    let log = dir.join("L.json");
    let log = log.to_str().unwrap();
    let store = dir.join("store.json");
    let store = store.to_str().unwrap();

    // ① campaign open — writes the canonical log.
    let (ok, v, _) = run(&[
        "campaign",
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    assert!(ok, "campaign open exits 0: {v}");
    assert_eq!(v["opened"], true);
    assert_eq!(count_kind(Path::new(log), "campaign.opened"), 1);

    // ② intent new --log — appends intent.landed to the SAME log.
    let (ok, v, _) = run(&[
        "intent",
        "new",
        "--log",
        log,
        "--store",
        store,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "land the intent",
        "--id",
        "i1",
    ]);
    assert!(ok, "intent new exits 0: {v}");
    assert_eq!(v["intent_id"], "i1");
    assert_eq!(v["already_exists"], false);
    assert_eq!(
        count_kind(Path::new(log), "intent.landed"),
        1,
        "intent.landed is on the shared log"
    );

    // ③ pr open --intent i1 — composes on the same log (the original repro).
    let (ok, v, _) = run(&[
        "pr",
        "open",
        "--log",
        log,
        "--pr",
        "1",
        "--campaign",
        CAMPAIGN,
        "--author-kind",
        "orchestrator",
        "--run-id",
        "r",
        "--intent",
        "i1",
    ]);
    assert!(ok, "pr open composes on the campaign-written log: {v}");
    assert_eq!(v["pr_id"], "1");
    assert_eq!(v["state"], "proposed");
    assert_eq!(v["already_exists"], false);

    // ④ pr land — enters the union queue (still in-flight: queued, not landed).
    let (ok, v, _) = run(&["pr", "land", "--log", log, "--pr", "1"]);
    assert!(ok, "pr land exits 0: {v}");
    assert_eq!(v["queued"], true);

    // ⑤ campaign close REFUSES — the PR is in-flight (structured error + fix).
    let (ok, v, _) = run(&["campaign", "close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(!ok, "close must refuse while a PR is in-flight");
    assert_eq!(v["error"]["kind"], "in_flight_prs");
    assert_eq!(v["error"]["fix"], "land or abandon first");
    let in_flight = v["error"]["detail"]["in_flight"].as_array().unwrap();
    assert!(in_flight.iter().any(|p| p == "1"), "names the in-flight PR");
    assert_eq!(
        count_kind(Path::new(log), "campaign.closed"),
        0,
        "no seal on a refused close"
    );

    // ⑥ campaign show shows exactly 1 in-flight PR (the cross-verb projection).
    let (ok, v, _) = run(&["campaign", "show", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok, "show is read-only and exits 0: {v}");
    assert_eq!(v["progress"]["in_flight"], 1);
    assert_eq!(v["progress"]["landed"], 0);
    let prs = v["prs"].as_array().unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0]["pr_id"], "1");
    assert_eq!(prs[0]["phase"], "in_flight");

    // ⑦ idempotent re-runs of every mutating verb on the SAME log — no dupes.
    let (ok, v, _) = run(&[
        "campaign",
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    assert!(ok);
    assert_eq!(v["already_exists"], true, "campaign open idempotent");

    let (ok, v, _) = run(&[
        "intent",
        "new",
        "--log",
        log,
        "--store",
        store,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "land the intent",
        "--id",
        "i1",
    ]);
    assert!(ok);
    assert_eq!(v["already_exists"], true, "intent new idempotent");

    let (ok, v, _) = run(&[
        "pr",
        "open",
        "--log",
        log,
        "--pr",
        "1",
        "--campaign",
        CAMPAIGN,
        "--author-kind",
        "orchestrator",
        "--run-id",
        "r",
        "--intent",
        "i1",
    ]);
    assert!(ok);
    assert_eq!(v["already_exists"], true, "pr open idempotent");

    let (ok, v, _) = run(&["pr", "land", "--log", log, "--pr", "1"]);
    assert!(ok);
    assert_eq!(v["already_queued"], true, "pr land idempotent");

    // The log still carries exactly one of each — no duplicate records.
    assert_eq!(count_kind(Path::new(log), "campaign.opened"), 1);
    assert_eq!(count_kind(Path::new(log), "intent.landed"), 1);
    assert_eq!(count_kind(Path::new(log), "pr.opened"), 1);
    assert_eq!(count_kind(Path::new(log), "pr.queued"), 1);
    assert_eq!(count_kind(Path::new(log), "campaign.closed"), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Regression — the EXACT repro: a campaign-written log feeds pr open.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn exact_repro_campaign_log_feeds_pr_open() {
    // Before PC4 this printed:
    //   hugit: error: parse log "/tmp/L.json": invalid type: map, expected a
    //   sequence at line 1 column 0
    // PC4: the two verbs share the canonical sequence shape, so it composes.
    let dir = scratch("repro");
    let log = dir.join("L.json");
    let log = log.to_str().unwrap();

    let (ok, _, _) = run(&[
        "campaign",
        "open",
        "--log",
        log,
        "--campaign",
        "x",
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    assert!(ok, "campaign open writes the canonical log");

    let (ok, v, stderr) = run(&[
        "pr",
        "open",
        "--log",
        log,
        "--pr",
        "1",
        "--campaign",
        "x",
        "--author-kind",
        "orchestrator",
        "--run-id",
        "r",
        "--intent",
        "i1",
    ]);
    assert!(
        !stderr.contains("invalid type: map"),
        "the map-vs-sequence mismatch must never return; stderr: {stderr}"
    );
    assert!(ok, "pr open reads the campaign-written log: {v} / {stderr}");
    assert_eq!(v["pr_id"], "1");
}

// ─────────────────────────────────────────────────────────────────────────────
// Regression — a genuinely MAP-shaped log is rejected (never silently accepted).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn map_shaped_log_is_rejected_by_every_verb() {
    let dir = scratch("map-reject");
    let log = dir.join("map.json");
    // The pre-PC4 map shape: a `{"events":[...]}` object, not a record array.
    std::fs::write(&log, br#"{"events":[]}"#).unwrap();
    let log = log.to_str().unwrap();

    // campaign show rejects the map (structured parse error, never a fake ok).
    let (ok, v, _) = run(&["campaign", "show", "--log", log, "--campaign", "x"]);
    assert!(!ok, "campaign rejects a map-shaped log");
    assert_eq!(v["error"]["kind"], "parse");

    // pr show rejects it on the seam fault path (generic error on stderr).
    let (ok, _, stderr) = run(&["pr", "show", "--log", log, "--pr", "1"]);
    assert!(!ok, "pr rejects a map-shaped log");
    assert!(
        stderr.contains("parse log"),
        "pr surfaces the parse fault: {stderr}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// pr open --intent validates referenced intents once the log uses the seam.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pr_open_refuses_intent_not_on_the_log() {
    let dir = scratch("missing-intent");
    let log = dir.join("L.json");
    let log = log.to_str().unwrap();
    let store = dir.join("store.json");
    let store = store.to_str().unwrap();

    // Land i1 on the shared log (now the intent seam is in use).
    let (ok, _, _) = run(&[
        "intent",
        "new",
        "--log",
        log,
        "--store",
        store,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--id",
        "i1",
    ]);
    assert!(ok);

    // Open a PR referencing a DIFFERENT, unlanded intent → structured refusal.
    let (ok, v, _) = run(&[
        "pr",
        "open",
        "--log",
        log,
        "--pr",
        "1",
        "--campaign",
        CAMPAIGN,
        "--author-kind",
        "orchestrator",
        "--run-id",
        "r",
        "--intent",
        "i1",
        "--intent",
        "i-ghost",
    ]);
    assert!(!ok, "pr open must refuse an intent absent from the log");
    // Canonical WB0 error law: nested `{"error":{"kind",…,"fix"}}`, context flat.
    assert_eq!(v["error"]["kind"], "missing_intents");
    assert!(v["error"]["fix"].is_string());
    let missing = v["error"]["missing_intents"].as_array().unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0], "i-ghost");
    assert_eq!(
        count_kind(Path::new(log), "pr.opened"),
        0,
        "no pr.opened appended on a refused open"
    );
}
