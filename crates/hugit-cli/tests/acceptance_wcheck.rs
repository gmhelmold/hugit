//! Acceptance — WP-W-INT (W-CHECK logic on W0's `check` entry point):
//! `hugit check` executes + records the memoized-CI wedge, end-to-end, against
//! the REAL `hugit` binary.
//!
//! This is the wedge made REAL (the EXECUTE path WB2 left as a forward seam),
//! wired to W0's frozen top-level `check` verb (`--def --log [--store]`):
//!
//!   1. COLD run — the memo key is absent from the local AC, so the def's
//!      command EXECUTES once (`local_executions:1`), a real `CheckResult` is
//!      captured + stored, and (with `--store`) a `check.recorded` row with
//!      `cache_hit:false` lands on the canonical `--log`.
//!   2. WARM re-run, byte-identical inputs — a cache HIT: ZERO local execution
//!      (`local_executions:0`), `duration_ms:0`, `cache_hit:true`. The wedge's
//!      "your green checks never re-run" guarantee, proven across processes.
//!   3. `hugit checks show` over that log now reports NON-NULL wedge KPIs —
//!      `hit_rate_pct` == 50, `hits` == 1, `executed` == 1 — the positive
//!      aggregation that was an honest-null negative-control before any seam
//!      recorded a check.
//!
//! Every output is parsed as stable JSON under the WB0 one-error/one-exit law;
//! the `--log` is the canonical `[EventRecord, …]` array bootstrapped via the
//! engine's own `EventLog` (the same primitive every verb writes through), so the
//! seed is honestly-built, never a hand-forged hash chain.

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wcheck-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit <args>` and return `(exit_code, parsed_stdout_json)`.
fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

/// Bootstrap an empty canonical `[EventRecord, …]` log file at `path`. An empty
/// JSON array is the legitimate "no events yet" world the recorder appends to.
fn write_empty_log(path: &Path) {
    let log = EventLog::new();
    let json = serde_json::to_string_pretty(log.records()).unwrap();
    std::fs::write(path, json).unwrap();
}

/// A tiny, deterministic source tree the tree-axis is scoped over (one matched
/// file). Returns the root path to pass as `--root`.
fn seed_tree(dir: &Path) -> PathBuf {
    let root = dir.join("src");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.rs"), b"fn main() {}\n").unwrap();
    root
}

/// A measurable-duration shell command whose real wall-clock time is > 0 ms on
/// any real OS — used so `saved_ms` is assertably positive after a warm HIT.
/// `sleep 0.05` is 50 ms, orders of magnitude above rounding noise, and safe
/// on macOS (BSD sleep accepts fractional seconds) and Linux.
const SLOW_CMD: &str = "sleep 0.05";

#[test]
fn cold_run_executes_records_miss_then_warm_rerun_is_a_hit_then_show_is_real() {
    let dir = scratch("wedge");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    let common = || -> Vec<String> {
        vec![
            "check".into(),
            "--def".into(),
            "green-check".into(),
            // An ad-hoc, instant, deterministic command — `true` always exits 0.
            "--cmd".into(),
            "true".into(),
            "--log".into(),
            log.display().to_string(),
            // W0's --store is a bool (record onto the log).
            "--store".into(),
            // --ac is the local AC store path (W-INT additive flag).
            "--ac".into(),
            ac.display().to_string(),
            "--root".into(),
            root.display().to_string(),
            "--toolchain".into(),
            "tc-fixed".into(),
        ]
    };

    // ── 1. COLD run — executes once, records a MISS. ────────────────────────
    let cold_args = common();
    let cold_ref: Vec<&str> = cold_args.iter().map(String::as_str).collect();
    let (code, cold) = run(&cold_ref);
    assert_eq!(code, 0, "cold run exits 0: {cold}");
    assert_eq!(cold["cache_hit"], false, "cold run is a MISS: {cold}");
    assert_eq!(
        cold["local_executions"], 1,
        "cold run executes the command exactly once: {cold}"
    );
    assert_eq!(cold["exit"], 0, "`true` exits 0: {cold}");
    assert_eq!(cold["ok"], true, "a zero exit is ok: {cold}");
    assert_eq!(cold["stored"], true, "--store recorded the row: {cold}");
    let cold_key = cold["memo_key"].as_str().unwrap().to_string();
    assert_eq!(cold_key.len(), 64, "memo key is a 64-char digest: {cold}");

    // ── 2. WARM re-run, identical inputs — a HIT, zero execution. ───────────
    let warm_args = common();
    let warm_ref: Vec<&str> = warm_args.iter().map(String::as_str).collect();
    let (code, warm) = run(&warm_ref);
    assert_eq!(code, 0, "warm run exits 0: {warm}");
    assert_eq!(warm["cache_hit"], true, "warm re-run is a HIT: {warm}");
    assert_eq!(
        warm["local_executions"], 0,
        "a hit performs ZERO local execution (the wedge): {warm}"
    );
    assert_eq!(
        warm["duration_ms"], 0,
        "a hit re-spends zero wall-clock: {warm}"
    );
    assert_eq!(
        warm["memo_key"], cold_key,
        "identical inputs key to the identical memo key: {warm}"
    );

    // ── 3. `checks show` now reports REAL, non-null wedge KPIs. ──────────────
    let (code, show) = run(&["checks", "show", "--log", &log.display().to_string()]);
    assert_eq!(code, 0, "checks show exits 0: {show}");
    // Two `check.recorded` rows (one miss, one hit) were appended by THIS verb.
    assert_eq!(show["check_count"], 2, "two checks recorded: {show}");
    // The negative-control note is GONE — there ARE check records now.
    assert!(
        show.get("note").is_none(),
        "with real records there is no honest-null note: {show}"
    );

    let kpis = &show["kpis"];
    assert!(
        !kpis["hit_rate_pct"].is_null(),
        "hit_rate_pct is non-null now: {kpis}"
    );
    assert!(!kpis["hits"].is_null(), "hits is non-null now: {kpis}");
    assert!(
        !kpis["executed"].is_null(),
        "executed is non-null now: {kpis}"
    );
    assert!(
        !kpis["saved_ms"].is_null(),
        "saved_ms is non-null (the hit row carried a duration): {kpis}"
    );

    assert_eq!(kpis["hits"], 1, "exactly one hit: {kpis}");
    assert_eq!(kpis["executed"], 1, "exactly one execution: {kpis}");
    // 1 hit / (1 hit + 1 executed) = 50% — a real, > 0 hit-rate.
    let rate = kpis["hit_rate_pct"].as_f64().unwrap();
    assert!(rate > 0.0, "the hit-rate is greater than zero: {kpis}");
    assert!((rate - 50.0).abs() < 1e-9, "1/2 = 50% hit-rate: {kpis}");
}

#[test]
fn editing_an_input_inside_the_glob_busts_the_cache_to_a_miss() {
    // Honesty: a warm re-run is a hit ONLY for byte-identical inputs. Editing a
    // file inside the def's glob_set changes the tree axis → a new memo key → a
    // MISS that re-executes. (`green-check` is ad-hoc → glob `**/*`.)
    let dir = scratch("bust");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    let args = || -> Vec<String> {
        vec![
            "check".into(),
            "--def".into(),
            "green-check".into(),
            "--cmd".into(),
            "true".into(),
            "--log".into(),
            log.display().to_string(),
            "--store".into(),
            "--ac".into(),
            ac.display().to_string(),
            "--root".into(),
            root.display().to_string(),
            "--toolchain".into(),
            "tc-fixed".into(),
        ]
    };

    let a = args();
    let r: Vec<&str> = a.iter().map(String::as_str).collect();
    let (_, cold) = run(&r);
    assert_eq!(cold["cache_hit"], false, "first run is a miss: {cold}");
    let key1 = cold["memo_key"].as_str().unwrap().to_string();

    // Edit a file inside the glob — the tree axis changes.
    std::fs::write(root.join("lib.rs"), b"fn main() { /* edited */ }\n").unwrap();

    let a2 = args();
    let r2: Vec<&str> = a2.iter().map(String::as_str).collect();
    let (_, after) = run(&r2);
    assert_eq!(
        after["cache_hit"], false,
        "an edit inside the glob busts the cache to a MISS: {after}"
    );
    assert_ne!(
        after["memo_key"].as_str().unwrap(),
        key1,
        "the edited input keys to a different memo key: {after}"
    );
}

#[test]
fn store_refuses_a_missing_log_with_the_canonical_envelope() {
    // The canonical-log seam: with `--store`, a `--log` that does not exist is
    // the explicit `log_not_found` error (exit 2), NEVER silently an empty world.
    let dir = scratch("nolog");
    let missing = dir.join("does-not-exist.json");
    let root = seed_tree(&dir);
    let (code, v) = run(&[
        "check",
        "--def",
        "green-check",
        "--cmd",
        "true",
        "--log",
        &missing.display().to_string(),
        "--store",
        "--root",
        &root.display().to_string(),
    ]);
    assert_eq!(
        code, 2,
        "a missing --log under --store is a structured error, exit 2: {v}"
    );
    assert_eq!(
        v["error"]["kind"], "log_not_found",
        "canonical envelope: {v}"
    );
}

#[test]
fn unknown_def_without_cmd_is_a_structured_error() {
    let dir = scratch("unknown");
    let log = dir.join("log.json");
    write_empty_log(&log);
    let (code, v) = run(&[
        "check",
        "--def",
        "not-a-builtin",
        "--log",
        &log.display().to_string(),
    ]);
    assert_eq!(code, 2, "an unknown def with no --cmd is exit 2: {v}");
    assert_eq!(v["error"]["kind"], "unknown_def", "canonical envelope: {v}");
}

#[test]
fn dry_run_without_store_records_nothing_on_the_log() {
    // Without `--store`, the wedge still runs (real cache verdict) but appends
    // NOTHING to the canonical log — `checks show` stays at the honest-null
    // negative control.
    let dir = scratch("dry");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);
    let (code, dry) = run(&[
        "check",
        "--def",
        "green-check",
        "--cmd",
        "true",
        "--log",
        &log.display().to_string(),
        "--ac",
        &ac.display().to_string(),
        "--root",
        &root.display().to_string(),
        "--toolchain",
        "tc-fixed",
    ]);
    assert_eq!(code, 0, "dry run exits 0: {dry}");
    assert_eq!(dry["stored"], false, "no --store ⇒ not recorded: {dry}");

    let (code, show) = run(&["checks", "show", "--log", &log.display().to_string()]);
    assert_eq!(code, 0, "checks show exits 0: {show}");
    assert_eq!(
        show["check_count"], 0,
        "a dry run records nothing on the log: {show}"
    );
    assert!(
        show["kpis"]["hit_rate_pct"].is_null(),
        "no records ⇒ honest-null KPIs: {show}"
    );
}

// ── WG-DOCS test-quality additions ───────────────────────────────────────────

/// A memoized RED stays red — a warm cache HIT for a command that exits 1 must
/// still report `ok:false`. This is the honesty gap: the cache must NOT mask a
/// failing result as green on a warm re-run.
///
/// - Cold run: `false` exits 1 → `ok:false`, `exit:1`, `cache_hit:false`.
/// - Warm re-run (identical inputs, same AC): cache HIT → `ok:false`, `exit:1`,
///   `cache_hit:true`. The stored result is faithfully "red"; the hit only avoids
///   re-execution — it never fabricates success.
#[test]
fn memoized_red_stays_red_warm_hit_does_not_mask_failure() {
    let dir = scratch("red-memo");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    let common = || -> Vec<String> {
        vec![
            "check".into(),
            "--def".into(),
            "red-check".into(),
            "--cmd".into(),
            "false".into(), // always exits 1
            "--log".into(),
            log.display().to_string(),
            "--store".into(),
            "--ac".into(),
            ac.display().to_string(),
            "--root".into(),
            root.display().to_string(),
            "--toolchain".into(),
            "tc-fixed".into(),
        ]
    };

    // Cold: executes, records a MISS with ok:false and exit:1.
    let cold_args = common();
    let cold_ref: Vec<&str> = cold_args.iter().map(String::as_str).collect();
    let (code, cold) = run(&cold_ref);
    // The verb exits 0 (the check ran and reported its result) even though the
    // check itself failed — the error law reports the check outcome, not panics.
    assert_eq!(code, 0, "check verb exits 0 (reports outcome): {cold}");
    assert_eq!(cold["cache_hit"], false, "cold run is a MISS: {cold}");
    assert_eq!(cold["ok"], false, "a `false` command is not ok: {cold}");
    assert_eq!(cold["exit"], 1, "`false` exits 1: {cold}");
    assert_eq!(cold["stored"], true, "--store recorded the MISS: {cold}");

    // Warm re-run — a HIT. The cached result is still the failing one.
    let warm_args = common();
    let warm_ref: Vec<&str> = warm_args.iter().map(String::as_str).collect();
    let (code, warm) = run(&warm_ref);
    assert_eq!(code, 0, "warm run exits 0 (reports cached outcome): {warm}");
    assert_eq!(warm["cache_hit"], true, "warm re-run is a HIT: {warm}");
    assert_eq!(
        warm["ok"], false,
        "the warm HIT faithfully reports the stored FAILURE — not masked green: {warm}"
    );
    assert_eq!(
        warm["exit"], 1,
        "the warm HIT carries the stored exit code 1: {warm}"
    );
}

/// A measurable-duration cold run gives a positive `saved_ms` on a warm HIT.
///
/// Uses `SLOW_CMD` (sleep 50 ms) so the cold execution takes a non-zero wall
/// clock. After a warm HIT the `checks show` KPI `saved_ms` must be > 0
/// (the duration saved by the cache hit).
#[test]
fn saved_ms_is_positive_after_warm_hit_on_slow_command() {
    let dir = scratch("saved-ms");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    let common = || -> Vec<String> {
        vec![
            "check".into(),
            "--def".into(),
            "slow-check".into(),
            "--cmd".into(),
            SLOW_CMD.into(),
            "--log".into(),
            log.display().to_string(),
            "--store".into(),
            "--ac".into(),
            ac.display().to_string(),
            "--root".into(),
            root.display().to_string(),
            "--toolchain".into(),
            "tc-fixed".into(),
        ]
    };

    // Cold: executes SLOW_CMD, stores a measured duration.
    let cold_args = common();
    let cold_ref: Vec<&str> = cold_args.iter().map(String::as_str).collect();
    let (code, cold) = run(&cold_ref);
    assert_eq!(code, 0, "cold slow run exits 0: {cold}");
    assert_eq!(cold["cache_hit"], false, "cold run is a MISS: {cold}");
    assert_eq!(cold["ok"], true, "sleep exits 0: {cold}");
    let cold_ms = cold["duration_ms"].as_u64().unwrap_or(0);
    // The cold duration should be >= 1 ms (50 ms nominal; we use 1 ms as the
    // lower bound to avoid flakes on slow CI where the measurement could round
    // differently, while still ensuring it is non-zero).
    assert!(
        cold_ms >= 1,
        "cold run duration_ms >= 1 ms (measured): {cold}"
    );

    // Warm: a HIT. duration_ms must be 0 (no re-execution).
    let warm_args = common();
    let warm_ref: Vec<&str> = warm_args.iter().map(String::as_str).collect();
    let (code, warm) = run(&warm_ref);
    assert_eq!(code, 0, "warm slow run exits 0: {warm}");
    assert_eq!(warm["cache_hit"], true, "warm re-run is a HIT: {warm}");
    assert_eq!(warm["duration_ms"], 0, "a HIT has duration_ms:0: {warm}");

    // `checks show` — `saved_ms` is the cold duration saved by the HIT.
    let (code, show) = run(&["checks", "show", "--log", &log.display().to_string()]);
    assert_eq!(code, 0, "checks show exits 0: {show}");
    let saved = show["kpis"]["saved_ms"].as_u64().unwrap_or(0);
    assert!(
        saved >= 1,
        "saved_ms > 0 after a warm HIT on a slow command: {show}"
    );
}
