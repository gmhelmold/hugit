//! Acceptance — `hugit watch` (Phase-D event-stream replay).
//!
//! Drives the REAL binary over a REAL chained log built through
//! `EventLog::append_for_test`. Pins: each event is classified + rendered via
//! `hugit_ledger::WatchDisplay` (landing/verdict/ws-state/other), `--class`
//! filters the stream, and the WB0 one-error/one-exit law (missing log ⇒ exit 2).

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::{Value, json};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-watch-{tag}-{}", std::process::id()));
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

fn write_log(path: &Path, events: &[(&str, Value)]) {
    let mut log = EventLog::new();
    for (kind, payload) in events {
        log.append_for_test(*kind, vec![], payload.to_string(), 0);
    }
    std::fs::write(path, serde_json::to_string_pretty(log.records()).unwrap()).unwrap();
}

fn sample_log(path: &Path) {
    write_log(
        path,
        &[
            (
                "intent.landed",
                json!({"intent_id":"i1","campaign":"auth","charter":"a"}),
            ),
            (
                "verdict.recorded",
                json!({"intent":"i1","lens":"panel","verdict":"approve","claims_checked":[]}),
            ),
            ("ws.state.active", json!({"workspace_id":"ws1"})),
            ("journal.note", json!({"note":"hello"})),
        ],
    );
}

#[test]
fn watch_classifies_and_renders_the_stream() {
    let dir = scratch("classify");
    let log = dir.join("log.json");
    sample_log(&log);

    let (code, v) = run(&["watch", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0, "watch exits 0 on a valid log: {v}");

    assert_eq!(v["count"], 4);
    let classes: Vec<&str> = v["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["class"].as_str().unwrap())
        .collect();
    assert_eq!(classes, vec!["landing", "verdict", "ws-state", "other"]);
    // Every line carries a seq and rendered text.
    for line in v["lines"].as_array().unwrap() {
        assert!(line["seq"].is_number());
        assert!(line["text"].is_string());
    }
}

#[test]
fn watch_filters_by_class() {
    let dir = scratch("filter");
    let log = dir.join("log.json");
    sample_log(&log);

    let (code, v) = run(&[
        "watch",
        "--log",
        log.to_str().unwrap(),
        "--class",
        "verdict",
    ]);
    assert_eq!(code, 0);
    assert_eq!(v["count"], 1);
    assert_eq!(v["lines"][0]["class"], "verdict");
}

#[test]
fn watch_missing_log_obeys_the_error_law() {
    let (code, v) = run(&["watch", "--log", "/no/such/watch/log.json"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["kind"], "log_not_found");
}
