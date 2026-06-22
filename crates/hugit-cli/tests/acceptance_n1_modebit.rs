//! Acceptance — Wave N, WP N-1: the POSIX MODE BIT is a tree-axis input, so a
//! `chmod` (SAME content) BUSTS the memo key instead of serving a STALE GREEN.
//!
//! ROOT (sweep-2026-06-12 F-1): the memoized-CI tree axis hashed file CONTENT
//! only, not the POSIX mode/exec bit. A check `--cmd './gate.sh'` over an
//! EXECUTABLE `gate.sh` (exit 0) cold-MISSed GREEN under memo_key K; then
//! `chmod -x gate.sh` (SAME content → K unchanged) served a warm `cache_hit:true
//! exit:0` STALE GREEN, while the real `./gate.sh` now fails `exit 126`
//! (permission denied). The mode is a result-affecting input the key dropped.
//!
//! N-1 folds the POSIX mode into the snapshotted per-file byte field
//! (`frame_file_with_mode`), so a mode change changes the `tree_root` → a MISS
//! that RE-EXECUTES and observes the real failing exit. An unchanged (same mode
//! + content) re-run still HITs — hit-rate preserved (no false busting).
//!
//! Both scratch trees live OUTSIDE the repo (`std::env::temp_dir()`). `#[cfg(unix)]`
//! gates the chmod repro: the executable bit only exists on POSIX.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn scratch(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("hugit-n1-{tag}-{}-{}", std::process::id(), nanos()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_empty_log(path: &Path) {
    let log = EventLog::new();
    std::fs::write(path, serde_json::to_string_pretty(log.records()).unwrap()).unwrap();
}

fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(mode);
    std::fs::set_permissions(path, perms).unwrap();
}

/// Run `hugit check --def adhoc --cmd './gate.sh' --root <root> --store` and
/// return `(process_exit, parsed_json)`.
fn run_check(root: &Path, log: &Path, ac: &Path) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args([
            "check",
            "run",
            "--def",
            "adhoc-n1",
            "--cmd",
            "./gate.sh",
            "--log",
            log.to_str().unwrap(),
            "--ac",
            ac.to_str().unwrap(),
            "--root",
            root.to_str().unwrap(),
            "--store",
        ])
        .output()
        .expect("hugit runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

fn hit(v: &Value) -> bool {
    v.get("cache_hit").and_then(Value::as_bool).unwrap_or(false)
}
fn exit_of(v: &Value) -> i64 {
    v.get("exit").and_then(Value::as_i64).unwrap_or(i64::MIN)
}
fn memo_key(v: &Value) -> String {
    v.get("memo_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// Seed `<root>/gate.sh` as an EXECUTABLE script that exits 0, plus a matched
/// source file so the glob has a stable companion. Returns the `--root`.
fn seed_executable_gate(dir: &Path) -> PathBuf {
    let root = dir.join("work");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("gate.sh"), b"#!/bin/sh\nexit 0\n").unwrap();
    set_mode(&root.join("gate.sh"), 0o755); // executable
    root
}

/// THE REPRO (F-1), closed: gate.sh executable → cold MISS / green / key K.
/// `chmod -x gate.sh` (SAME content, mode-only change) → a MISS that
/// RE-EXECUTES and observes the REAL `exit 126` (permission denied) — NOT a
/// stale green hit — because the memo key CHANGED (the mode is now in the axis).
#[test]
fn chmod_minus_x_busts_the_key_no_stale_green() {
    let dir = scratch("chmod");
    let root = seed_executable_gate(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // 1) Executable gate.sh → cold MISS, GREEN, memo_key K.
    let (_p1, r1) = run_check(&root, &log, &ac);
    assert!(!hit(&r1), "run 1 is a cold MISS: {r1:?}");
    assert_eq!(exit_of(&r1), 0, "executable gate.sh exits 0: {r1:?}");
    let key_k = memo_key(&r1);
    assert!(!key_k.is_empty(), "run 1 reports a memo key: {r1:?}");

    // 2) chmod -x gate.sh — SAME content, mode-only change.
    set_mode(&root.join("gate.sh"), 0o644); // remove exec bit

    let (_p2, r2) = run_check(&root, &log, &ac);
    // The memo key must have CHANGED (mode folded into the tree axis).
    assert_ne!(
        memo_key(&r2),
        key_k,
        "a mode change (same content) MUST bust the memo key: {r2:?}"
    );
    // …so this is a MISS that RE-EXECUTES, NOT a stale-green hit.
    assert!(
        !hit(&r2),
        "chmod -x must produce a re-executing MISS, never a stale-green hit: {r2:?}"
    );
    // …and the re-executed run observes the REAL permission-denied exit (126),
    // not the cached green 0.
    assert_eq!(
        exit_of(&r2),
        126,
        "the non-executable ./gate.sh fails `exit 126` (permission denied) — the \
         stale green is gone: {r2:?}"
    );
}

/// No FALSE busting: re-running with NO change (same mode + same content) is
/// still a warm HIT — hit-rate preserved. The mode fold must bust ONLY on an
/// actual mode change, never on every run.
#[test]
fn unchanged_mode_and_content_is_still_a_hit() {
    let dir = scratch("nochange");
    let root = seed_executable_gate(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Cold MISS.
    let (_p1, r1) = run_check(&root, &log, &ac);
    assert!(!hit(&r1), "run 1 is a cold MISS: {r1:?}");
    assert_eq!(exit_of(&r1), 0, "executable gate.sh exits 0: {r1:?}");
    let key_k = memo_key(&r1);

    // Identical re-run: same mode (0o755), same content → warm HIT.
    let (_p2, r2) = run_check(&root, &log, &ac);
    assert_eq!(
        memo_key(&r2),
        key_k,
        "an unchanged tree keeps the SAME memo key: {r2:?}"
    );
    assert!(
        hit(&r2),
        "no change (same mode + content) must be a warm HIT — hit-rate preserved: {r2:?}"
    );
    assert_eq!(
        exit_of(&r2),
        0,
        "the memoized green is served on the hit: {r2:?}"
    );
}
