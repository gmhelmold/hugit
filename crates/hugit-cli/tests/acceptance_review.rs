//! Acceptance — `hugit review` (D7 grounded-evidence Q&A over the log).
//!
//! Drives the REAL binary: `hugit check --store` and `hugit verdict --store` WRITE
//! evidence-bearing records, `hugit review` READS them back and answers by
//! grounded retrieval. Pins: a grounded question is CITED, an ungrounded question
//! is explicitly REFUSED (never fabricated), and the WB0 one-error/one-exit law.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-review-{tag}-{}", std::process::id()));
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

fn empty_log(path: &Path) {
    std::fs::write(path, "[]").unwrap();
}

/// Record a real `check.recorded` via `hugit check --store` (a trivial passing cmd).
fn record_check(log: &str, dir: &Path, name: &str, cmd: &str) {
    let ac = dir.join(format!("{name}.ac"));
    let (code, v) = run(&[
        "check",
        "run",
        "--def",
        name,
        "--cmd",
        cmd,
        "--log",
        log,
        "--store",
        "--ac",
        ac.to_str().unwrap(),
        "--root",
        dir.to_str().unwrap(),
        "--toolchain",
        "tc-fixed",
    ]);
    assert_eq!(code, 0, "check --store must succeed: {v}");
}

#[test]
fn review_cites_a_grounded_check_question() {
    let dir = scratch("cited");
    let log = dir.join("log.json");
    empty_log(&log);
    let log_s = log.to_str().unwrap();

    // A real recorded check named "lint" (a NON-builtin def, so --cmd runs) that
    // passes (`true` → exit 0). A builtin name (fmt/clippy/test) would ignore
    // --cmd and run the real cargo gate, failing in this empty scratch dir.
    record_check(log_s, &dir, "lint", "true");

    // A question grounded by the check evidence (keyword "lint" / "pass").
    let (code, v) = run(&[
        "review",
        "--log",
        log_s,
        "--question",
        "did the lint check pass?",
    ]);
    assert_eq!(code, 0, "a grounded review exits 0: {v}");
    assert_eq!(v["answer"], "cited", "must cite the check evidence: {v}");
    let citations = v["citations"].as_array().expect("citations");
    assert!(
        citations
            .iter()
            .any(|c| c.as_str().unwrap().starts_with("check:")),
        "cites a check evidence ref: {v}"
    );
    let excerpts: String = v["excerpts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap())
        .collect();
    assert!(
        excerpts.contains("lint") && excerpts.to_lowercase().contains("pass"),
        "the excerpt grounds the answer in the real check: {v}"
    );
}

#[test]
fn review_refuses_an_ungrounded_question_never_fabricates() {
    let dir = scratch("refused");
    let log = dir.join("log.json");
    empty_log(&log);
    let log_s = log.to_str().unwrap();
    record_check(log_s, &dir, "build", "true");

    // A question with no grounding evidence on the log → explicit refusal.
    let (code, v) = run(&[
        "review",
        "--log",
        log_s,
        "--question",
        "what is the meaning of life and the airspeed of a swallow?",
    ]);
    assert_eq!(code, 0, "a refusal is still exit 0 (a valid answer): {v}");
    assert_eq!(v["answer"], "refused", "must refuse, not fabricate: {v}");
    assert!(v["reason"].is_string(), "carries a refusal reason: {v}");
    assert!(v["citations"].is_null(), "a refusal cites nothing: {v}");
}

#[test]
fn review_empty_question_obeys_the_error_law() {
    let dir = scratch("empty");
    let log = dir.join("log.json");
    empty_log(&log);
    let (code, v) = run(&[
        "review",
        "--log",
        log.to_str().unwrap(),
        "--question",
        "   ",
    ]);
    assert_eq!(code, 2, "an empty question is a structured error: {v}");
    assert_eq!(v["error"]["kind"], "empty_question");
}

#[test]
fn review_missing_log_obeys_the_error_law() {
    let (code, v) = run(&[
        "review",
        "--log",
        "/no/such/review/log.json",
        "--question",
        "anything",
    ]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["kind"], "log_not_found");
}
