//! Acceptance — PS-11: `--env-axis <VAR>` declares a custom env dependency.
//!
//! The hermetic spawn (Round-8 C3 / Wave L-C) CLEARS every ambient env var not on
//! the built-in result-affecting allowlist, so an ad-hoc `--cmd` check that reads a
//! CUSTOM var sees it UNSET and a change to it cannot bust the memo key. PS-11 lets
//! the caller DECLARE such a var with `--env-axis VAR`: the named var is then (a)
//! folded into the memo key (so a change to its value is a MISS) AND (b) passed
//! through to the spawn — declared == keyed == present, never a silent stale green.
//!
//! These tests drive the REAL `hugit` binary and set the var on the CHILD process's
//! env (`Command::env`), so there is no process-global mutation in the test runner
//! (the cross-test data race). The tree axis is held constant across every run, so a
//! second-run MISS can only be the declared env var busting the key.

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
    let dir = std::env::temp_dir().join(format!(
        "hugit-ps11-{tag}-{}-{}",
        std::process::id(),
        nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_empty_log(path: &Path) {
    let log = EventLog::new();
    std::fs::write(path, serde_json::to_string_pretty(log.records()).unwrap()).unwrap();
}

/// Seed a tiny matched tree under `dir/src` so the tree axis is non-empty and
/// CONSTANT across runs (only the env var varies between runs).
fn seed_tree(dir: &Path) -> PathBuf {
    let root = dir.join("src");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.rs"), b"fn main() {}\n").unwrap();
    root
}

/// Run `hugit check` for an ad-hoc always-succeeds command, optionally declaring an
/// `--env-axis` var and setting a custom var on the child env. Returns `cache_hit`.
#[allow(clippy::too_many_arguments)]
fn run_check(
    root: &Path,
    log: &Path,
    ac: &Path,
    declare_axis: bool,
    custom_var: Option<(&str, &str)>,
) -> bool {
    let mut c = Command::new(hugit_bin());
    c.args([
        "check",
        "run",
        "--def",
        "adhoc-ps11",
        "--cmd",
        "echo hi", // always exit 0 — we are testing the MEMO KEY, not the exit
        "--log",
        log.to_str().unwrap(),
        "--ac",
        ac.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--store",
    ]);
    if declare_axis {
        c.args(["--env-axis", "MY_GATE_MODE"]);
    }
    if let Some((k, v)) = custom_var {
        c.env(k, v);
    }
    let out = c.output().expect("hugit runs");
    let v: Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap_or(Value::Null);
    assert_eq!(
        out.status.code(),
        Some(0),
        "check must succeed (exit 0); got {v}"
    );
    v.get("cache_hit").and_then(Value::as_bool).unwrap_or(false)
}

// ── Declared: a change to the --env-axis var's value is a MISS ────────────────
#[test]
fn declared_env_axis_var_change_is_a_miss() {
    let dir = scratch("declared");
    let root = seed_tree(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Cold run with MY_GATE_MODE=strict, declared → MISS, exit 0 stored.
    assert!(
        !run_check(&root, &log, &ac, true, Some(("MY_GATE_MODE", "strict"))),
        "first run is a cold MISS"
    );
    // Identical re-run → HIT (warm, same key).
    assert!(
        run_check(&root, &log, &ac, true, Some(("MY_GATE_MODE", "strict"))),
        "identical re-run is a warm HIT"
    );
    // Change ONLY the declared var's value → MISS (the var is keyed via --env-axis).
    assert!(
        !run_check(&root, &log, &ac, true, Some(("MY_GATE_MODE", "lax"))),
        "changing a DECLARED env-axis var busts the memo key (MISS) — PS-11 soundness"
    );
}

// ── Undeclared: the SAME var change is invariant (hermetic clear; hit-rate) ────
#[test]
fn undeclared_custom_var_change_is_invariant() {
    let dir = scratch("undeclared");
    let root = seed_tree(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Cold run with MY_GATE_MODE=strict, NOT declared → MISS.
    assert!(
        !run_check(&root, &log, &ac, false, Some(("MY_GATE_MODE", "strict"))),
        "first run is a cold MISS"
    );
    // Change the var's value WITHOUT declaring it → still a HIT: the hermetic spawn
    // clears it, it is not in the key, so it cannot bust the cache (hit-rate kept).
    assert!(
        run_check(&root, &log, &ac, false, Some(("MY_GATE_MODE", "lax"))),
        "an UNDECLARED custom var does not affect the memo key (warm HIT)"
    );
}
