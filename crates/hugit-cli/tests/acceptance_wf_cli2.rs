//! Acceptance — WP-WF-CLI2: the robustness adversary's two follow-up bugs
//! WF-CLI did NOT cover, both against the REAL `hugit` binary
//! (`CARGO_BIN_EXE_hugit`).
//!
//! ## Bug 1 — the ghost record
//!
//! `campaign close`/`abandon` on a MISSING `--log` previously bootstrapped an
//! empty world and wrote a `campaign.closed`/`campaign.abandoned` record for a
//! campaign that never had a `campaign.opened` — a record from nothing, exit 0.
//! Proven fixed here:
//!   - `close`/`abandon` on a missing `--log` → structured `log_not_found`,
//!     exit-2, NO file created (the read-must-exist `load_existing` loader);
//!   - `close` on a log WITHOUT the campaign's `campaign.opened` record →
//!     structured `not_opened`, exit-2, no `campaign.closed` appended (the same
//!     guard `abandon` already carried);
//!   - `close` on a PROPERLY-opened + landed campaign STILL reaches
//!     `closed:true` (WF-CLI's deadlock fix is not regressed);
//!   - `open` STILL bootstraps an absent `--log` (it legitimately starts one).
//!
//! ## Bug 2 — the load→lock TOCTOU inversion
//!
//! `campaign open/close/abandon` + `intent new --store` loaded the file UNLOCKED
//! and only locked at persist (cloning the pre-lock snapshot), so two concurrent
//! writers could clobber. The fix acquires the advisory lock BEFORE the load and
//! holds it across load→mutate→persist (the WC1 `pr`/`canonical_log` pattern).
//! Proven here:
//!   - two SIMULTANEOUS `campaign open` for DIFFERENT keys on one `--log` →
//!     both `campaign.opened` records survive, the chain still verifies (no
//!     clobber); any loser fails structured `log_busy`;
//!   - two SIMULTANEOUS `intent new --store` for DIFFERENT intents on one store →
//!     both intents survive, the chain still verifies; any loser fails structured
//!     `store_busy`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-wfcli2-{tag}-{}-{}",
        std::process::id(),
        nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Run `hugit <args>` → `(exit_code, parsed_stdout_json_or_null)`.
fn run(args: &[&str]) -> (Option<i32>, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code(), v)
}

// ── Bug 1: the ghost record ──────────────────────────────────────────────────

#[test]
fn close_on_missing_log_is_log_not_found_no_ghost_file() {
    let dir = scratch("close-missing");
    let absent = dir.join("never-opened.json");
    assert!(!absent.exists(), "precondition: the log is absent");

    let (code, v) = run(&[
        "campaign",
        "close",
        "--log",
        absent.to_str().unwrap(),
        "--campaign",
        "never-opened",
    ]);
    assert_eq!(
        code,
        Some(2),
        "close on a missing --log MUST exit 2 (log_not_found), never bootstrap a \
         ghost record: {v}"
    );
    assert_eq!(v["error"]["kind"], "log_not_found", "{v}");
    assert!(v["error"]["fix"].is_string(), "fix hint present: {v}");
    // The crucial ghost-record assertion: NO file was created from nothing.
    assert!(
        !absent.exists(),
        "close on a missing --log must NOT create the log (no campaign.closed \
         from nothing)"
    );
}

#[test]
fn abandon_on_missing_log_is_log_not_found_no_ghost_file() {
    let dir = scratch("abandon-missing");
    let absent = dir.join("never-opened.json");
    assert!(!absent.exists(), "precondition: the log is absent");

    let (code, v) = run(&[
        "campaign",
        "abandon",
        "--log",
        absent.to_str().unwrap(),
        "--campaign",
        "never-opened",
        "--reason",
        "ghost",
    ]);
    assert_eq!(
        code,
        Some(2),
        "abandon on a missing --log MUST exit 2 (log_not_found): {v}"
    );
    assert_eq!(v["error"]["kind"], "log_not_found", "{v}");
    assert!(
        !absent.exists(),
        "abandon on a missing --log must NOT create the log (no \
         campaign.abandoned from nothing)"
    );
}

#[test]
fn close_on_log_without_opened_record_is_not_opened() {
    // A real, valid log that DOES exist but has no `campaign.opened` for this
    // key — the second half of the ghost-record guard. Seed the log by opening a
    // DIFFERENT campaign so the file exists and the chain is genuine.
    let dir = scratch("close-not-opened");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();

    let (code, _) = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "other-campaign",
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    assert_eq!(code, Some(0), "seed open of a different campaign succeeds");

    // Closing a campaign that has no opened record on this (existing) log.
    let (code, v) = run(&[
        "campaign",
        "close",
        "--log",
        log_s,
        "--campaign",
        "never-opened",
    ]);
    assert_eq!(
        code,
        Some(2),
        "close of a never-opened campaign MUST exit 2 (not_opened): {v}"
    );
    assert_eq!(v["error"]["kind"], "not_opened", "{v}");
    assert!(v["error"]["fix"].is_string(), "fix hint present: {v}");
    // No `campaign.closed` was appended for the ghost campaign.
    assert_eq!(
        count_kind(&log, "campaign.closed"),
        0,
        "no campaign.closed seal appended for a never-opened campaign"
    );
}

#[test]
fn close_on_properly_opened_campaign_with_no_unsettled_work_reaches_closed() {
    // The deadlock-fix non-regression: a campaign that IS opened and has no
    // in-flight PRs STILL seals (closed:true) — the new not_opened/load_existing
    // guards must not block the happy path that reaches the seal.
    let dir = scratch("close-happy");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();

    let (code, _) = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "shipit",
        "--charter",
        "ship the thing",
        "--owner",
        "o@h.com",
    ]);
    assert_eq!(code, Some(0), "open succeeds");

    let (code, v) = run(&["campaign", "close", "--log", log_s, "--campaign", "shipit"]);
    assert_eq!(
        code,
        Some(0),
        "a properly-opened campaign with no unsettled work STILL closes \
         (deadlock fix intact): {v}"
    );
    assert_eq!(v["closed"], true, "{v}");
    assert_eq!(v["already_closed"], false, "{v}");
    assert_eq!(count_kind(&log, "campaign.closed"), 1);
}

#[test]
fn open_still_bootstraps_an_absent_log() {
    // The fix must NOT break open's legitimate create-on-absent behavior.
    let dir = scratch("open-bootstrap");
    let log = dir.join("fresh.json");
    assert!(!log.exists(), "precondition: the log is absent");

    let (code, v) = run(&[
        "campaign",
        "open",
        "--log",
        log.to_str().unwrap(),
        "--campaign",
        "bootstrap-me",
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    assert_eq!(code, Some(0), "open bootstraps an absent log: {v}");
    assert_eq!(v["opened"], true);
    assert!(log.exists(), "open created the log");
    assert_eq!(count_kind(&log, "campaign.opened"), 1);
}

// ── Bug 2: the load→lock TOCTOU — concurrency proofs ─────────────────────────

#[test]
fn two_concurrent_campaign_open_different_keys_both_survive() {
    // Two DISTINCT campaign keys opened simultaneously on ONE --log. A correct
    // serialization lands BOTH campaign.opened records; a lock loss for one
    // lands one (the loser fails structured log_busy). NEVER a clobber: the log
    // ends VALID and BOTH records must survive whenever both processes report
    // success.
    let dir = scratch("camp-concurrency");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap().to_string();

    let spawn = |key: &str| {
        Command::new(hugit_bin())
            .args([
                "campaign",
                "open",
                "--log",
                &log_s,
                "--campaign",
                key,
                "--charter",
                "c",
                "--owner",
                "o@h.com",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn campaign open")
    };

    for round in 0..8 {
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(lock_path(&log));

        let key_a = format!("alpha-{round}");
        let key_b = format!("beta-{round}");
        let a = spawn(&key_a);
        let b = spawn(&key_b);
        let oa = a.wait_with_output().expect("wait a");
        let ob = b.wait_with_output().expect("wait b");

        let a_ok = oa.status.success();
        let b_ok = ob.status.success();
        assert!(
            a_ok || b_ok,
            "round {round}: at least one open must win the lock"
        );

        // A loser (if any) must have failed STRUCTURED with log_busy.
        for (label, out) in [("a", &oa), ("b", &ob)] {
            if !out.status.success() {
                let v: Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim())
                    .unwrap_or(Value::Null);
                assert_eq!(
                    v["error"]["kind"], "log_busy",
                    "round {round}: loser {label} must fail structured log_busy: {v}"
                );
            }
        }

        // The log is a VALID canonical array whose chain verifies — never a
        // clobber/fork. When BOTH won, BOTH distinct campaign.opened records
        // must be present (the TOCTOU-clobber would drop one).
        let keys = opened_campaign_keys(&log, round);
        if a_ok && b_ok {
            assert!(
                keys.contains(&key_a) && keys.contains(&key_b),
                "round {round}: both winners' records must survive — no clobber; \
                 got {keys:?}"
            );
        } else {
            // Exactly one winner → exactly one record, structured loss for the other.
            let winner = if a_ok { &key_a } else { &key_b };
            assert!(
                keys.contains(winner),
                "round {round}: the winner's record must be on the log; got {keys:?}"
            );
        }
    }
}

#[test]
fn two_concurrent_intent_new_store_different_intents_both_survive() {
    // Two DISTINCT intents authored simultaneously into ONE --store. A correct
    // serialization lands BOTH intent.landed records; a lock loss for one lands
    // one (the loser fails structured store_busy). NEVER a clobber: the store
    // ends VALID and BOTH intents survive whenever both processes report success.
    let dir = scratch("store-concurrency");
    let store = dir.join("store.json");
    let store_s = store.to_str().unwrap().to_string();

    let spawn = |charter: &str| {
        Command::new(hugit_bin())
            .args([
                "intent",
                "new",
                "--store",
                &store_s,
                "--campaign",
                "wfcli2",
                "--charter",
                charter,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn intent new")
    };

    for round in 0..8 {
        let _ = std::fs::remove_file(&store);
        let _ = std::fs::remove_file(lock_path(&store));

        let charter_a = format!("alpha-{round}");
        let charter_b = format!("beta-{round}");
        let a = spawn(&charter_a);
        let b = spawn(&charter_b);
        let oa = a.wait_with_output().expect("wait a");
        let ob = b.wait_with_output().expect("wait b");

        let a_ok = oa.status.success();
        let b_ok = ob.status.success();
        assert!(
            a_ok || b_ok,
            "round {round}: at least one intent new must win the lock"
        );

        // Collect the intent_ids the winners reported.
        let mut won_ids: Vec<String> = Vec::new();
        for out in [&oa, &ob] {
            if out.status.success() {
                let v: Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim())
                    .unwrap_or(Value::Null);
                if let Some(id) = v["intent_id"].as_str() {
                    won_ids.push(id.to_string());
                }
            } else {
                let v: Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim())
                    .unwrap_or(Value::Null);
                assert_eq!(
                    v["error"]["kind"], "store_busy",
                    "round {round}: a loser must fail structured store_busy: {v}"
                );
            }
        }

        // The store's event chain must VERIFY and carry every winner's intent
        // (the TOCTOU-clobber would drop one).
        let landed = landed_intent_ids(&store, round);
        for id in &won_ids {
            assert!(
                landed.contains(id),
                "round {round}: winner intent {id} must survive on the store — \
                 no clobber; got {landed:?}"
            );
        }
        if a_ok && b_ok {
            assert_eq!(
                won_ids.len(),
                2,
                "round {round}: both winners reported distinct intents"
            );
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn lock_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

fn count_kind(path: &Path, kind: &str) -> usize {
    let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == kind)
        .count()
}

/// Parse the canonical `[EventRecord, …]` log, assert the chain verifies through
/// the real engine, and return the set of campaign keys with a `campaign.opened`.
fn opened_campaign_keys(path: &Path, round: usize) -> Vec<String> {
    let records = verified_records(path, round);
    records
        .iter()
        .filter(|r| r.kind == "campaign.opened")
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter_map(|v| {
            v.get("campaign")
                .and_then(|c| c.as_str())
                .map(str::to_string)
        })
        .collect()
}

/// Parse the on-disk intent store, assert its event chain verifies, and return
/// the set of landed intent_ids.
fn landed_intent_ids(path: &Path, round: usize) -> Vec<String> {
    let bytes = std::fs::read(path).expect("store exists after the race");
    let file: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("round {round}: store must be VALID JSON: {e}"));
    let events: Vec<hugit_contracts::event_record::EventRecord> =
        serde_json::from_value(file["events"].clone())
            .unwrap_or_else(|e| panic!("round {round}: store events parse: {e}"));
    let mut log = hugit_refstore::EventLog::new();
    for r in events {
        log.push_record(r)
            .unwrap_or_else(|e| panic!("round {round}: store record rehydrates: {e}"));
    }
    hugit_refstore::verify_chain(log.records())
        .unwrap_or_else(|e| panic!("round {round}: store chain MUST verify after the race: {e}"));
    log.records()
        .iter()
        .filter(|r| r.kind == "intent.landed")
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter_map(|v| {
            v.get("intent_id")
                .and_then(|i| i.as_str())
                .map(str::to_string)
        })
        .collect()
}

/// Read + verify a canonical `[EventRecord, …]` log file through the real engine.
fn verified_records(path: &Path, round: usize) -> Vec<hugit_contracts::event_record::EventRecord> {
    let bytes = std::fs::read(path).expect("log exists after the race");
    let records: Vec<hugit_contracts::event_record::EventRecord> = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("round {round}: log must be VALID JSON (no clobber): {e}"));
    let mut log = hugit_refstore::EventLog::new();
    for r in records.clone() {
        log.push_record(r)
            .unwrap_or_else(|e| panic!("round {round}: record rehydrates: {e}"));
    }
    hugit_refstore::verify_chain(log.records())
        .unwrap_or_else(|e| panic!("round {round}: chain MUST verify after the race: {e}"));
    records
}
