//! Acceptance — `hugit ctx resume` (D11 short-horizon session resume).
//!
//! Drives the REAL binary end-to-end: `hugit journal note` WRITES the records,
//! `hugit ctx resume` READS them back — proving the scrubbed-binding join works
//! across the writer→reader seam. Pins: within-horizon reconstruction, the
//! documented `beyond_horizon` / `empty_journal` refusals (never a silent stale
//! reconstruction), and the WB0 one-error/one-exit law.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-ctx-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

/// Seed an empty canonical log (the bootstrap a `journal note` requires).
fn empty_log(path: &Path) {
    std::fs::write(path, "[]").unwrap();
}

/// Append a journal.note via the REAL writer verb.
fn note(log: &str, note: &str, ws: &str, intent: &str) {
    let (code, v) = run(&[
        "journal",
        "note",
        "--log",
        log,
        "--note",
        note,
        "--workspace",
        ws,
        "--intent",
        intent,
    ]);
    assert_eq!(code, 0, "journal note must succeed: {v}");
}

#[test]
fn ctx_resume_reconstructs_within_horizon() {
    let dir = scratch("ok");
    let log = dir.join("log.json");
    empty_log(&log);
    let log_s = log.to_str().unwrap();

    note(log_s, "started task", "ws1", "in1");
    note(log_s, "ran the gate", "ws1", "in1");
    // A note on a DIFFERENT binding must NOT bleed into the reconstruction.
    note(log_s, "unrelated", "ws2", "in2");

    // journal.note records carry recorded_at=0 (clock-untrusted log); pass a
    // --now-ms within the 7-day horizon of 0 so resume is within-horizon.
    let (code, v) = run(&[
        "ctx",
        "resume",
        "--log",
        log_s,
        "--workspace",
        "ws1",
        "--intent",
        "in1",
        "--now-ms",
        "1000",
    ]);
    assert_eq!(code, 0, "within-horizon resume exits 0: {v}");
    assert_eq!(v["reconstructed"], true);
    assert_eq!(v["workspace_id"], "ws1");
    assert_eq!(v["intent_id"], "in1");

    let entries = v["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 2, "only the ws1/in1 binding's 2 notes: {v}");
    assert_eq!(entries[0]["note"], "started task");
    assert_eq!(entries[1]["note"], "ran the gate");
    assert_eq!(entries[0]["seq"], 1);
    assert_eq!(entries[1]["seq"], 2);
}

#[test]
fn ctx_resume_refuses_beyond_horizon_never_a_silent_stale_reconstruction() {
    let dir = scratch("beyond");
    let log = dir.join("log.json");
    empty_log(&log);
    let log_s = log.to_str().unwrap();
    note(log_s, "long ago", "ws1", "in1");

    // age = now_ms - 0 = 700_000_000 ms > the 604_800_000 ms (7-day) horizon.
    let (code, v) = run(&[
        "ctx",
        "resume",
        "--log",
        log_s,
        "--workspace",
        "ws1",
        "--intent",
        "in1",
        "--now-ms",
        "700000000",
    ]);
    assert_eq!(
        code, 2,
        "beyond-horizon is a structured refusal, exit 2: {v}"
    );
    assert_eq!(v["error"]["kind"], "beyond_horizon");
    assert!(
        v["error"]["horizon_ms"].is_number(),
        "carries horizon_ms: {v}"
    );
    assert!(v["error"]["age_ms"].is_number(), "carries age_ms: {v}");
}

#[test]
fn ctx_resume_empty_binding_refuses_honestly() {
    let dir = scratch("empty");
    let log = dir.join("log.json");
    empty_log(&log);
    let log_s = log.to_str().unwrap();
    note(log_s, "only this binding", "ws1", "in1");

    // A binding with no matching notes → empty_journal (not a fabricated context).
    let (code, v) = run(&[
        "ctx",
        "resume",
        "--log",
        log_s,
        "--workspace",
        "nope",
        "--intent",
        "nope",
        "--now-ms",
        "1000",
    ]);
    assert_eq!(code, 2, "an empty binding refuses, exit 2: {v}");
    assert_eq!(v["error"]["kind"], "empty_journal");
}

#[test]
fn ctx_resume_missing_log_obeys_the_error_law() {
    let (code, v) = run(&[
        "ctx",
        "resume",
        "--log",
        "/no/such/ctx/log.json",
        "--workspace",
        "ws1",
        "--intent",
        "in1",
    ]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["kind"], "log_not_found");
}
