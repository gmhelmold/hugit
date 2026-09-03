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

/// Run `hugit <args>` with `RUSTFLAGS` set to `rustflags`, returning
/// `(exit_code, parsed_stdout_json)`. Used by the env-axis stale-green test:
/// `RUSTFLAGS` is on the result-affecting allowlist, so changing it between two
/// otherwise-identical runs must bust the memo key (a MISS, not a stale green).
fn run_with_rustflags(args: &[&str], rustflags: &str) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .env("RUSTFLAGS", rustflags)
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
            "run".into(),
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
    let (code, show) = run(&["check", "show", "--log", &log.display().to_string()]);
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
            "run".into(),
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
        "run",
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
        "run",
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
        "run",
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

    let (code, show) = run(&["check", "show", "--log", &log.display().to_string()]);
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
            "run".into(),
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
            "run".into(),
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
    let (code, show) = run(&["check", "show", "--log", &log.display().to_string()]);
    assert_eq!(code, 0, "checks show exits 0: {show}");
    let saved = show["kpis"]["saved_ms"].as_u64().unwrap_or(0);
    assert!(
        saved >= 1,
        "saved_ms > 0 after a warm HIT on a slow command: {show}"
    );
}

/// K-RUN [SHIP-BLOCKER] — the stale-green close: the check command runs through
/// `sh -c` inheriting the FULL ambient environment, so a result-affecting env var
/// changes the gate's outcome. Before the fix the env axis was hardcoded empty,
/// so a change to (e.g.) `RUSTFLAGS` did NOT bust the memo key → a stale GREEN
/// was served with ZERO execution. This proves the env axis now busts the key:
/// a cold MISS under one RUSTFLAGS, then the SAME args under a DIFFERENT RUSTFLAGS
/// is a fresh MISS (cache_hit:false) — not a laundered hit. An UNCHANGED env is
/// still a warm HIT (the hit-rate is preserved — we capture only an allowlist).
#[test]
fn changing_a_result_affecting_env_var_busts_the_memo_key_no_stale_green() {
    let dir = scratch("env-axis");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    // A command whose RESULT depends on RUSTFLAGS (read straight from the env the
    // shell inherits). With RUSTFLAGS=green it exits 0; with anything else, 1.
    let cmd = r#"test "$RUSTFLAGS" = green"#;
    let args = check_args("env-check", cmd, &log, &ac, &root);
    let r: Vec<&str> = args.iter().map(String::as_str).collect();

    // Cold run under RUSTFLAGS=green → MISS, executes, exits 0 (green).
    let (code, cold) = run_with_rustflags(&r, "green");
    assert_eq!(code, 0, "verb reports outcome: {cold}");
    assert_eq!(cold["cache_hit"], false, "cold run is a MISS: {cold}");
    assert_eq!(
        cold["ok"], true,
        "RUSTFLAGS=green ⇒ the command is green: {cold}"
    );

    // Same args, UNCHANGED env (RUSTFLAGS=green) → warm HIT (hit-rate preserved).
    let (_, warm) = run_with_rustflags(&r, "green");
    assert_eq!(
        warm["cache_hit"], true,
        "an UNCHANGED allowlisted env is still a warm HIT: {warm}"
    );

    // Same args, CHANGED RUSTFLAGS=red → the env axis busts the key: a fresh MISS
    // that EXECUTES the now-red command. NOT a stale green served from the cache.
    let (_, changed) = run_with_rustflags(&r, "red");
    assert_eq!(
        changed["cache_hit"], false,
        "a CHANGED result-affecting env var is a MISS, never a stale green: {changed}"
    );
    assert_eq!(
        changed["ok"], false,
        "the re-executed command reflects the now-red env (exit 1): {changed}"
    );
}

// ── WG-CACHE — tamper-evident cache, real toolchain axis, exec timeout, ──────
// ── lookup lock, log-not-found law, cmd-ignored honesty. ─────────────────────

/// Build the standard `check` arg vector for an ad-hoc def over a given AC path.
fn check_args(def: &str, cmd: &str, log: &Path, ac: &Path, root: &Path) -> Vec<String> {
    vec![
        "check".into(),
        "run".into(),
        "--def".into(),
        def.into(),
        "--cmd".into(),
        cmd.into(),
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
}

/// WG-CACHE [CRITICAL]: editing the stored `<log>.ac` to flip a RED result's
/// `exit`→0 (forge a green) is DETECTED — the next `hugit check` recomputes the
/// entry self-hash, finds a mismatch, treats it as a MISS, and RE-EXECUTES the
/// real command. The forged green is NEVER served, never laundered into the log.
#[test]
fn tampering_the_ac_to_forge_a_green_is_detected_and_re_executed() {
    let dir = scratch("tamper");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    // Cold run of a RED command (`false` exits 1) — stores a genuine red result.
    let args = check_args("red-check", "false", &log, &ac, &root);
    let r: Vec<&str> = args.iter().map(String::as_str).collect();
    let (code, cold) = run(&r);
    assert_eq!(code, 0, "verb reports outcome: {cold}");
    assert_eq!(cold["cache_hit"], false, "cold run is a MISS: {cold}");
    assert_eq!(cold["ok"], false, "the real result is RED: {cold}");
    assert_eq!(cold["exit"], 1, "`false` exits 1: {cold}");

    // A warm re-run WITHOUT tampering is a faithful red HIT.
    let (_, warm) = run(&r);
    assert_eq!(warm["cache_hit"], true, "untampered warm run HITs: {warm}");
    assert_eq!(warm["ok"], false, "the HIT is still red: {warm}");

    // TAMPER: flip the stored exit 1 → 0 in the `.ac` file (forge a green) WITHOUT
    // recomputing the self-hash. (`true`/`false` both pretty-print `"exit": N`.)
    let raw = std::fs::read_to_string(&ac).unwrap();
    let forged = raw.replace("\"exit\": 1", "\"exit\": 0");
    assert_ne!(forged, raw, "the tamper actually changed the stored exit");
    std::fs::write(&ac, &forged).unwrap();

    // Next check: the self-hash over the forged bytes no longer matches → MISS →
    // RE-EXECUTE the real `false` → ok:false again. The forged green is rejected.
    let (code, after) = run(&r);
    assert_eq!(code, 0, "verb reports outcome: {after}");
    assert_eq!(
        after["cache_hit"], false,
        "a tampered entry is a MISS — the cache re-executes, never serves the forgery: {after}"
    );
    assert_eq!(
        after["ok"], false,
        "re-execution yields the REAL red result, not the forged green: {after}"
    );
    assert_eq!(
        after["exit"], 1,
        "the real `false` exit survives the tamper: {after}"
    );
}

/// WG-CACHE [MEDIUM]: two DIFFERENT toolchain digests key DISTINCTLY — a green
/// cached under toolchain A is a MISS under toolchain B. (Proves the toolchain
/// axis is a real, key-busting input, not the old `local-toolchain` constant.)
#[test]
fn two_toolchain_digests_key_distinctly() {
    let dir = scratch("tc-axis");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    let mut a = check_args("tc-check", "true", &log, &ac, &root);
    // Replace the trailing "tc-fixed" with toolchain A.
    *a.last_mut().unwrap() = "toolchain-A".into();
    let ra: Vec<&str> = a.iter().map(String::as_str).collect();
    let (_, run_a) = run(&ra);
    assert_eq!(
        run_a["cache_hit"], false,
        "first run (A) is a MISS: {run_a}"
    );
    let key_a = run_a["memo_key"].as_str().unwrap().to_string();

    // Same everything but toolchain B → a DIFFERENT memo key → a MISS, not a hit.
    let mut b = a.clone();
    *b.last_mut().unwrap() = "toolchain-B".into();
    let rb: Vec<&str> = b.iter().map(String::as_str).collect();
    let (_, run_b) = run(&rb);
    assert_eq!(
        run_b["cache_hit"], false,
        "a different toolchain digest is a MISS, not a cross-toolchain false hit: {run_b}"
    );
    assert_ne!(
        run_b["memo_key"].as_str().unwrap(),
        key_a,
        "two toolchain digests key to distinct memo keys: {run_b}"
    );
}

/// WG-CACHE: omitting `--toolchain` resolves a REAL active-toolchain digest (a
/// 64-char sha256 hex), not a constant — so a toolchain change busts the key.
#[test]
fn omitting_toolchain_yields_a_real_digest_axis() {
    let dir = scratch("tc-real");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    // No --toolchain flag.
    let (code, v) = run(&[
        "check",
        "run",
        "--def",
        "tc-real-check",
        "--cmd",
        "true",
        "--log",
        &log.display().to_string(),
        "--store",
        "--ac",
        &ac.display().to_string(),
        "--root",
        &root.display().to_string(),
    ]);
    assert_eq!(code, 0, "runs without --toolchain: {v}");
    // The recorded row carries the resolved toolchain_digest; read it back.
    let (_, show) = run(&["check", "show", "--log", &log.display().to_string()]);
    let mk = show["checks"][0]["memo_key"].as_str().unwrap();
    assert_eq!(mk.len(), 64, "memo key is a 64-char digest: {show}");
    // The toolchain digest itself is either a 64-char sha256 hex (rustc probed) or
    // the explicit `toolchain-unprobed` marker — NEVER the old `local-toolchain`
    // constant. We assert it is not that stale constant by re-running with an
    // explicit toolchain equal to "local-toolchain" and confirming a DIFFERENT key.
    let (_, explicit) = run(&[
        "check",
        "run",
        "--def",
        "tc-real-check",
        "--cmd",
        "true",
        "--log",
        &log.display().to_string(),
        "--ac",
        &ac.display().to_string(),
        "--root",
        &root.display().to_string(),
        "--toolchain",
        "local-toolchain",
    ]);
    assert_eq!(
        explicit["cache_hit"], false,
        "the default toolchain axis is NOT the old `local-toolchain` constant: {explicit}"
    );
}

/// WG-CHECK-ROBUST [SHIP-BLOCKER]: a hanging command (`sleep 30`) bounded by a
/// short `--timeout-secs` is KILLED and returns a structured `check_timeout`
/// (exit 2) — it does NOT block forever holding the log lock.
#[test]
fn a_hanging_command_times_out_with_a_structured_error() {
    let dir = scratch("hang");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    let start = std::time::Instant::now();
    let (code, v) = run(&[
        "check",
        "run",
        "--def",
        "hang-check",
        "--cmd",
        "sleep 30",
        "--log",
        &log.display().to_string(),
        "--store",
        "--ac",
        &ac.display().to_string(),
        "--root",
        &root.display().to_string(),
        "--timeout-secs",
        "1",
    ]);
    let elapsed = start.elapsed();
    assert_eq!(code, 2, "a timeout is a structured error, exit 2: {v}");
    assert_eq!(
        v["error"]["kind"], "check_timeout",
        "canonical envelope names the timeout: {v}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "the timeout fired (did not block for the full 30s sleep): {elapsed:?}"
    );
}

/// WG-CHECK-ROBUST: the log-not-found law holds WITHOUT `--store` too. A `hugit
/// check` against a nonexistent `--log` is `log_not_found`/exit-2 — never a
/// silent dry green on a typo'd path.
#[test]
fn missing_log_without_store_is_log_not_found_not_a_silent_green() {
    let dir = scratch("nolog-dry");
    let missing = dir.join("typo.json");
    let root = seed_tree(&dir);
    let (code, v) = run(&[
        "check",
        "run",
        "--def",
        "dry-check",
        "--cmd",
        "true",
        "--log",
        &missing.display().to_string(),
        // NO --store.
        "--root",
        &root.display().to_string(),
    ]);
    assert_eq!(
        code, 2,
        "a missing --log without --store is exit 2 (not a dry green): {v}"
    );
    assert_eq!(
        v["error"]["kind"], "log_not_found",
        "canonical envelope: {v}"
    );
}

/// WG-CHECK-ROBUST: a built-in `--def` that is given a `--cmd` surfaces
/// `cmd_ignored:true` so the agent KNOWS its command had no effect (the built-in
/// gate command wins). An ad-hoc def that USES its `--cmd` reports `false`.
#[test]
fn builtin_def_with_cmd_reports_cmd_ignored() {
    let dir = scratch("cmd-ignored");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    // `fmt` is a built-in; supplying --cmd must be flagged as ignored. Point the
    // built-in's command at a tree with nothing to format under a 30s ceiling;
    // we only care about the cmd_ignored signal, not the gate's exit.
    let (_, v) = run(&[
        "check",
        "run",
        "--def",
        "fmt",
        "--cmd",
        "echo should-be-ignored",
        "--log",
        &log.display().to_string(),
        "--ac",
        &ac.display().to_string(),
        "--root",
        &root.display().to_string(),
        "--toolchain",
        "tc-fixed",
        "--timeout-secs",
        "60",
    ]);
    assert_eq!(
        v["cmd_ignored"], true,
        "a built-in def flags an ignored --cmd: {v}"
    );

    // An ad-hoc def USES its --cmd → cmd_ignored:false.
    let a = check_args("adhoc-check", "true", &log, &ac, &root);
    let r: Vec<&str> = a.iter().map(String::as_str).collect();
    let (_, v2) = run(&r);
    assert_eq!(
        v2["cmd_ignored"], false,
        "an ad-hoc def honours its --cmd (not ignored): {v2}"
    );
}

/// WH-CHECK [lock-poison fix]: two concurrent `hugit check` on identical inputs
/// MAY both execute — the AC lock is now held ONLY around the cache file's own
/// lookup/store ops, NOT across the unbounded execute, so a hung/slow command can
/// never poison the AC lock for everyone else. We deliberately ACCEPT that small
/// benign double-exec window (strictly better than a lock-poison hang). What is
/// NOT acceptable — and is still guaranteed — is a duplicate on the canonical log:
/// the log's own lock serializes the two recorders and the `(memo_key, cache_hit)`
/// dedup drops the second, so the log carries NO duplicate `check.recorded` and
/// the wedge KPIs are never inflated by the race.
///
/// # What this test NOW proves (WI-TESTS — kill theater)
///
/// The STRENGTHENED test positively asserts both of the key properties in one run:
///
/// 1. **Both threads executed** — the shared `--cmd` appends one line to a counter
///    file (`echo x >> counter`) then sleeps 200 ms, giving a wide overlap window.
///    After both threads return, the counter file line-count is the number of
///    threads that reached the execute stage. Because BOTH threads use the IDENTICAL
///    `--cmd` string, they key to the SAME `memo_key` (same def + same tree + same
///    toolchain). The AC lock is released BEFORE execute (the lock-only-cache fix),
///    so both threads find the AC EMPTY, execute simultaneously, and each appends a
///    line → 2 lines in the counter.
///
///    If the lock were reverted to hold-across-execute (regression): thread B blocks
///    until thread A finishes, then finds the AC WARM → `cache_hit:true`, skips
///    execute → only 1 line in the counter. This assertion FAILS in that case.
///
/// 2. **Exactly ONE `check.recorded` MISS row** — despite (possibly) two
///    executions, the `(memo_key, cache_hit:false)` dedup in `record_on_log`
///    fires on the second recorder → the log carries at most ONE MISS row.
///    `executed == 1` and `check_count == 1`.
///    If dedup broke (double-record), `executed == 2` → this assertion FAILs.
///
/// 3. **The hash chain is valid** — verify_chain runs after the concurrent storm
///    via `checks show`; a corrupted atomic append would surface as `chain_broken`.
///
/// # What is NOT guaranteed (documented, not silenced)
///
/// - "Both executed" is PROBABLE but not certain on a very fast host where thread 0
///   completes the AC store between thread 1's lookup and 1's execute decision.
///   In that edge case thread 1 returns `cache_hit:true` (warm hit), never runs the
///   command, and the counter has 1 line. We assert `>= 1` to tolerate that rare
///   fast path while `check_count == 1` + `executed == 1` remain invariant.
/// - `ac_busy` / `log_busy` (retryable contention on a busy CI host) is still
///   accepted for a non-0 exit; such a thread never executed. The counter and log
///   assertions are adjusted for that case.
#[test]
fn concurrent_checks_do_not_double_record_even_if_both_execute() {
    use std::thread;

    let dir = scratch("toctou");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    // The counter file: both threads use the IDENTICAL --cmd string, which appends
    // one line and then sleeps 200 ms. Using the same command is REQUIRED so both
    // threads compute the same def_digest → same memo_key → the dedup can fire.
    // POSIX O_APPEND makes concurrent `echo x >>` safe (each write is atomic).
    let counter = dir.join("exec-counter.txt");
    let cmd = format!("echo x >> {} && sleep 0.2", counter.display());

    // Both threads get IDENTICAL args: same --def, --cmd, --log, --ac, --root,
    // --toolchain → same memo_key. This is the concurrent identical-input scenario
    // the lock-poison fix was designed to handle.
    let args = check_args("race-check", &cmd, &log, &ac, &root);
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let args = args.clone();
            thread::spawn(move || {
                let r: Vec<&str> = args.iter().map(String::as_str).collect();
                run(&r)
            })
        })
        .collect();
    let results: Vec<(i32, Value)> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // Every run either succeeded (exit 0, a real cache verdict) or serialized with
    // a retryable contention error on a state file (`ac_busy` / `log_busy`) —
    // NEVER a poison/clobber that corrupts state.
    for (code, v) in &results {
        if *code == 0 {
            assert!(
                v["cache_hit"].as_bool().is_some(),
                "a successful run reports a real cache verdict: {v}"
            );
        } else {
            let kind = v["error"]["kind"].as_str().unwrap_or("");
            assert!(
                kind == "ac_busy" || kind == "log_busy",
                "the loser serializes with a retryable busy (no poison): {v}"
            );
        }
    }

    // ── 1. Counter lines: how many threads reached the EXECUTE stage ─────────
    //
    // Each execution appends one line. A thread that got `cache_hit:true` or a
    // busy error never ran the command → no line.
    //
    // With the lock-only-cache fix: AC lock released before execute → both find
    // the AC empty → both execute → 2 lines.
    //
    // With lock-across-execute (regression): thread B blocks until A finishes,
    // finds AC warm → HIT, no execute → 1 line. FAILS if reverted.
    //
    // We assert >= 2 only when BOTH threads exited 0. If one got a busy error,
    // we assert >= 1 (the non-busy thread executed).
    let exit_0_count = results.iter().filter(|(code, _)| *code == 0).count();
    let counter_lines = std::fs::read_to_string(&counter)
        .unwrap_or_default()
        .lines()
        .count();

    if exit_0_count == 2 {
        // Both succeeded: if lock-across-execute were reinstated, the 2nd thread
        // would get a warm HIT without executing → only 1 line in the counter.
        // 2 lines prove the AC lock was NOT held across execute.
        assert!(
            counter_lines >= 2,
            "BOTH threads exited 0 but only {counter_lines} line(s) in the \
             exec-counter: if lock-across-execute was reverted the second thread \
             would get a warm hit without executing → 1 line; \
             2+ lines prove both threads executed (lock-only-cache fix is in place)"
        );
    } else {
        // At least one thread got a retryable busy and never executed.
        assert!(
            counter_lines >= 1,
            "no thread reached the execute stage: counter_lines=0 ({results:?})"
        );
    }

    // ── 2. Dedup invariant: the log carries EXACTLY ONE check.recorded row ────
    //
    // Even if both threads executed (2 counter lines), the `(memo_key, false)`
    // dedup fires on the second recorder → at most ONE MISS row on the log.
    // `executed == 1` is the KPI: the log reflects ONE execution, not two.
    //
    // This FAILS if dedup broke: executed == 2 → inflated KPIs.
    // This also FAILS if executed == 0 → no execution at all (something is wrong).
    let (_, show) = run(&["check", "show", "--log", &log.display().to_string()]);
    let executed = show["kpis"]["executed"].as_u64().unwrap_or(0);
    assert_eq!(
        executed, 1,
        "exactly ONE check.recorded MISS row (dedup held despite concurrent \
         double-execute): executed={executed} ({show})"
    );
    // check_count == 1: the dedup must not have let a second row through.
    // (A HIT row would only appear if one thread got a warm AC before executing,
    // in which case the counter also has < 2 lines — consistent.)
    let check_count = show["check_count"].as_u64().unwrap_or(0);
    assert!(
        check_count == 1,
        "exactly ONE total check.recorded row on the log (dedup held): \
         check_count={check_count} ({show})"
    );

    // ── 3. Hash chain is valid after the concurrent storm ────────────────────
    //
    // `checks show` runs `verify_chain` internally; a non-0 exit here means the
    // chain was corrupted during concurrent appends (the lock-serialized atomic
    // seam must prevent this).
    let (show_code, show_v) = run(&["check", "show", "--log", &log.display().to_string()]);
    assert_eq!(
        show_code, 0,
        "checks show exits 0 — the hash chain survived the concurrent storm: {show_v}"
    );
}

// ── WH-CHECK (adversarial Round-4 Cluster B) acceptance additions ─────────────

/// WH-CHECK [HIGH — KPI fabrication]: `check --store` is IDEMPOTENT. Three (or N)
/// identical `check --store` runs do NOT append N `check.recorded` rows — the
/// `(memo_key, cache_hit)` dedup bounds the log to one cold MISS + one warm HIT,
/// so `check_count` and `hit_rate_pct` are STABLE no matter how many times the
/// same check is re-run. A re-run that adds nothing reports `already_recorded`.
#[test]
fn check_store_is_idempotent_repeated_runs_do_not_inflate_kpis() {
    let dir = scratch("idem");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    let args = check_args("idem-check", "true", &log, &ac, &root);
    let r: Vec<&str> = args.iter().map(String::as_str).collect();

    // Run 1: cold MISS — newly recorded.
    let (_, run1) = run(&r);
    assert_eq!(run1["cache_hit"], false, "run1 is a cold MISS: {run1}");
    assert_eq!(
        run1["already_recorded"], false,
        "the cold MISS is a NEW record: {run1}"
    );

    // Run 2: warm HIT — newly recorded (distinct (key,cache_hit) pair).
    let (_, run2) = run(&r);
    assert_eq!(run2["cache_hit"], true, "run2 is a warm HIT: {run2}");
    assert_eq!(
        run2["already_recorded"], false,
        "the FIRST warm HIT is a new record (the wedge's hit row): {run2}"
    );

    // Run 3..=5: identical warm HITs — deduped, NOTHING appended.
    for n in 3..=5 {
        let (_, again) = run(&r);
        assert_eq!(again["cache_hit"], true, "run{n} is a warm HIT: {again}");
        assert_eq!(
            again["already_recorded"], true,
            "a repeated identical HIT appends nothing (idempotent): {again}"
        );
    }

    // The log holds EXACTLY two rows (1 miss + 1 hit) — never five.
    let (_, show) = run(&["check", "show", "--log", &log.display().to_string()]);
    assert_eq!(
        show["check_count"], 2,
        "five identical runs leave exactly two rows (1 miss + 1 hit), not five: {show}"
    );
    // hit_rate is a STABLE 50% — un-inflatable by re-running.
    let rate = show["kpis"]["hit_rate_pct"].as_f64().unwrap();
    assert!(
        (rate - 50.0).abs() < 1e-9,
        "hit-rate is a stable 50%: {show}"
    );
    assert_eq!(show["kpis"]["hits"], 1, "exactly one hit row: {show}");
    assert_eq!(show["kpis"]["executed"], 1, "exactly one miss row: {show}");
}

/// WH-CHECK [SHIP-BLOCKER — symlink-cycle stack overflow]: a workspace with a
/// directory symlink LOOP (`work/sub/loop → work`, common in monorepos) is walked
/// safely — the visited-real-path set + depth cap break the cycle, so `hugit
/// check` returns a structured result, NEVER a SIGSEGV / stack-overflow crash.
///
/// # What this test NOW proves (WI-TESTS — kill theater)
///
/// The STRENGTHENED test (unix only) positively asserts the cycle-guard FIRED
/// and the real file was included in the tree hash — not merely that the walk
/// "didn't crash" (which could also be true if the walk simply aborted with an
/// empty file set before ever visiting the cycle).
///
/// **Strategy:** run the check twice — once with the cyclic workspace (`work/sub/
/// loop → work`), once with a REFERENCE tree that is identical except the symlink
/// is absent. Assert the two runs key to the SAME `memo_key`:
///
/// - Same memo_key ⟹ same tree hash ⟹ `sub/a.rs` was included in BOTH walks
///   (the cyclic walk found the real file and the cycle-guard transparently
///   skipped the loop — no extra/missing content).
/// - If the guard had NOT fired and the walk recursed forever, it would crash
///   (SIGSEGV / stack overflow) before producing any JSON.
/// - If the walk had ABORTED EARLY (before finding `sub/a.rs` entirely), the
///   cyclic memo_key would be the hash of an EMPTY file set → different from the
///   reference key → assertion FAILS.
///
/// This test FAILS if:
/// - The cycle-guard is removed: the walk recurses → crash → non-0 exit or
///   unparseable JSON → the exit-0 assertion fails.
/// - The walk aborts before finding `sub/a.rs`: the memo_key would not match
///   the reference (different tree hash → different memo_key).
///
/// # Non-unix platforms
///
/// `std::os::unix::fs::symlink` is unavailable on non-unix targets, so the
/// directory symlink is never created there. On non-unix the test degenerates
/// to a plain "check exits 0 over a normal tree" with no cycle to guard against.
/// This is explicitly documented (not silenced): the guard is a unix-specific
/// code path; the non-unix run is NOT a false green for the guard property —
/// it is a no-op skip. The `cfg(unix)` annotation below makes this clear.
#[test]
fn a_directory_symlink_cycle_does_not_crash_the_tree_walk() {
    let dir = scratch("symcycle");
    let log_cyclic = dir.join("log-cyclic.json");
    let log_ref = dir.join("log-ref.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log_cyclic);
    write_empty_log(&log_ref);

    // ── Build the CYCLIC workspace: work/sub/a.rs + work/sub/loop → work ─────
    let work = dir.join("work");
    let sub = work.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("a.rs"), b"fn x() {}\n").unwrap();

    // ── Run check over the cyclic workspace ──────────────────────────────────
    #[cfg(unix)]
    std::os::unix::fs::symlink(&work, sub.join("loop")).unwrap();

    let cyclic_args = check_args("cycle-check", "true", &log_cyclic, &ac, &work);
    let r: Vec<&str> = cyclic_args.iter().map(String::as_str).collect();
    let (code, cyclic) = run(&r);

    // The verb MUST complete with a real verdict — no crash. A SIGSEGV would
    // produce no parseable JSON and a non-0 exit; a stack overflow would panic.
    assert_eq!(
        code, 0,
        "a symlink-cycle workspace yields a structured result, not a crash: {cyclic}"
    );
    assert_eq!(
        cyclic["ok"], true,
        "the check ran to a real outcome: {cyclic}"
    );
    assert_eq!(
        cyclic["cache_hit"], false,
        "first run is a cold miss: {cyclic}"
    );
    let cyclic_key = cyclic["memo_key"].as_str().unwrap_or("").to_string();
    assert_eq!(
        cyclic_key.len(),
        64,
        "cyclic walk produced a valid 64-char memo_key: {cyclic}"
    );

    // ── Build the REFERENCE workspace (identical content, no symlink) ─────────
    //
    // On non-unix the symlink above was never created, so `work` is already
    // cycle-free; we reuse the same `work` root. On unix we build a fresh
    // reference tree without the loop so the key comparison is meaningful.
    #[cfg(unix)]
    {
        // Reference tree: same file content, no cycle.
        let ref_work = dir.join("ref-work");
        let ref_sub = ref_work.join("sub");
        std::fs::create_dir_all(&ref_sub).unwrap();
        std::fs::write(ref_sub.join("a.rs"), b"fn x() {}\n").unwrap();
        // Use a fresh AC so the reference run is also a cold MISS (key is the
        // tree hash, not the AC state; but a fresh AC avoids a warm hit).
        let ac_ref = dir.join("ac-ref.json");

        let ref_args = check_args("cycle-check", "true", &log_ref, &ac_ref, &ref_work);
        let r2: Vec<&str> = ref_args.iter().map(String::as_str).collect();
        let (ref_code, ref_v) = run(&r2);
        assert_eq!(ref_code, 0, "reference (cycle-free) run exits 0: {ref_v}");
        let ref_key = ref_v["memo_key"].as_str().unwrap_or("").to_string();
        assert_eq!(
            ref_key.len(),
            64,
            "reference walk produced a valid 64-char memo_key: {ref_v}"
        );

        // KEY ASSERTION: the cyclic walk and the cycle-free walk key to the SAME
        // memo_key — meaning the tree hash (the file-content axis) was identical.
        // This positively proves:
        //   (a) the cycle-guard FIRED (the loop was skipped, not recursed)
        //   (b) `sub/a.rs` WAS included in the cyclic walk (same hash as
        //       reference means the same file was found — the walk did NOT abort
        //       early before visiting real files)
        // If the guard hadn't fired → crash → non-0 exit (caught above).
        // If the walk had aborted early → empty tree hash ≠ ref hash → FAILS here.
        assert_eq!(
            cyclic_key, ref_key,
            "cyclic and cycle-free trees key identically — the cycle-guard fired \
             (loop skipped) and sub/a.rs was found in BOTH walks: \
             cyclic_key={cyclic_key} ref_key={ref_key}"
        );
    }

    // On non-unix: the symlink was never created, so there is no cycle to guard
    // against. The test verifies the check exits 0 on a plain tree only — the
    // cycle-guard property is NOT proven on non-unix (unix-only code path).
    // This is explicitly documented; the non-unix run is not a false green.
    #[cfg(not(unix))]
    {
        // Verify the plain tree still works (sanity, not the guard property).
        let _ = (log_ref, cyclic_key); // suppress unused warnings on non-unix
    }
}

/// WH-CHECK [process-group kill REMOVED for safety] + Task #36 [orphan reaping
/// restored without a group signal]: a command that BACKGROUNDS a grandchild
/// (`sleep 30 & … sleep 30`) TIMES OUT PROMPTLY — exit 2 / check_timeout,
/// returning in ~1-2 s (NOT 30 s). The runner deliberately does NOT
/// process-group-signal: a negative-pid kill is unsafe on a linux runner / the
/// engine container (it takes down the whole process tree, incl. our own job —
/// observed on GitHub-hosted CI, even with `setsid`). Instead it enumerates the
/// child's descendants from `/proc` BEFORE killing it and SIGKILLs each by single
/// POSITIVE PID.
///
/// Two guarantees are asserted: (1) the PROMPT timeout — the orphan-held pipe must
/// NOT block the return (the drain-thread join is skipped on timeout); and (2) on
/// LINUX, the backgrounded grandchild IS reaped (Task #36) — its recorded PID is
/// no longer alive shortly after the timeout. On non-linux unix (macOS) `sh -c`
/// execs the command (no forked grandchild) and there is no `/proc`, so only the
/// prompt-timeout guarantee is asserted there.
#[test]
#[cfg(unix)]
fn a_backgrounding_command_times_out_promptly_without_a_group_signal() {
    let dir = scratch("pgroup");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    // The grandchild records its pid to a marker file, then both it and the direct
    // child sleep well past the 1 s ceiling.
    let marker = dir.join("orphan.pid");
    let cmd = format!("sleep 30 & echo $! > {}; sleep 30", marker.display());

    let start = std::time::Instant::now();
    let (code, v) = run(&[
        "check",
        "run",
        "--def",
        "bg-check",
        "--cmd",
        &cmd,
        "--log",
        &log.display().to_string(),
        "--store",
        "--ac",
        &ac.display().to_string(),
        "--root",
        &root.display().to_string(),
        "--toolchain",
        "tc-fixed",
        "--timeout-secs",
        "1",
    ]);
    let elapsed = start.elapsed();
    assert_eq!(code, 2, "the backgrounding command times out, exit 2: {v}");
    assert_eq!(
        v["error"]["kind"], "check_timeout",
        "structured timeout: {v}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(15),
        "the wall-time ceiling is honoured (did not wait the full 30 s): {elapsed:?}"
    );

    // The grandchild recorded its pid.
    let pid = std::fs::read_to_string(&marker).unwrap_or_default();
    let pid = pid.trim().to_string();
    assert!(!pid.is_empty(), "the grandchild recorded its pid: {v}");

    // (2) On LINUX the descendant sweep (Task #36) reaps the backgrounded
    // grandchild by single PID. SIGKILL delivery is prompt but asynchronous, so
    // poll briefly for the pid to become not-alive (`kill -0 <pid>` fails once the
    // process is gone). A positive pid is a single-process probe — never a group.
    #[cfg(target_os = "linux")]
    {
        let mut reaped = false;
        for _ in 0..50 {
            // `kill -0` succeeds iff the pid is alive (and signalable). A non-zero
            // exit means the grandchild is gone — reaped by the descendant sweep.
            let alive = Command::new("kill")
                .arg("-0")
                .arg(&pid)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !alive {
                reaped = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        // Best-effort cleanup in case the assertion is about to fail (don't leak a
        // 30 s sleep into the CI runner).
        if !reaped {
            let _ = Command::new("kill").arg("-KILL").arg(&pid).status();
        }
        assert!(
            reaped,
            "the backgrounded grandchild (pid {pid}) was reaped by the /proc \
             descendant sweep on linux"
        );
    }

    // On non-linux unix (macOS): no /proc descendant sweep, so the grandchild may
    // still be alive — best-effort single-PID cleanup so the test never leaks a
    // 30 s `sleep` (a positive pid is a single-process signal, never a group).
    #[cfg(not(target_os = "linux"))]
    {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = Command::new("kill").arg("-KILL").arg(&pid).status();
    }
}

/// WH-CHECK [SHIP-BLOCKER — lock-poison]: a SLOW check holding mid-execute does
/// NOT poison the AC lock — a DIFFERENT concurrent check still completes cleanly
/// (it is not blocked on a command-long AC lock). The two record independently;
/// the log carries BOTH rows. (Before the fix the slow check held the `.ac` lock
/// across its whole execute, so every concurrent `hugit check` got `ac_busy`.)
#[test]
fn a_slow_check_does_not_poison_the_ac_lock_for_a_concurrent_check() {
    use std::thread;

    let dir = scratch("nopoison");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    // A slow check (sleep 1.5) on toolchain SLOW, launched first.
    let slow = {
        let mut a = check_args("poison-slow", "sleep 1.5", &log, &ac, &root);
        *a.last_mut().unwrap() = "tc-SLOW".into();
        a
    };
    let slow_handle = thread::spawn(move || {
        let r: Vec<&str> = slow.iter().map(String::as_str).collect();
        run(&r)
    });

    // While the slow check is mid-execute (lock released), a FAST check on a
    // DIFFERENT toolchain (distinct memo_key) must complete cleanly.
    thread::sleep(std::time::Duration::from_millis(400));
    let fast = {
        let mut a = check_args("poison-fast", "true", &log, &ac, &root);
        *a.last_mut().unwrap() = "tc-FAST".into();
        a
    };
    let fr: Vec<&str> = fast.iter().map(String::as_str).collect();
    let (fcode, fv) = run(&fr);
    assert_eq!(
        fcode, 0,
        "a concurrent check is NOT poisoned by the slow check's execute: {fv}"
    );
    assert_eq!(
        fv["ok"], true,
        "the concurrent check ran to a real outcome: {fv}"
    );

    let (scode, sv) = slow_handle.join().unwrap();
    assert_eq!(scode, 0, "the slow check itself completes: {sv}");

    // Both checks recorded — the log was never poisoned.
    let (_, show) = run(&["check", "show", "--log", &log.display().to_string()]);
    assert_eq!(
        show["check_count"], 2,
        "both the slow and the concurrent check recorded — no poison: {show}"
    );
}

/// WH-CHECK [MED — ad-hoc `--cmd` never memoizes]: a 2nd identical ad-hoc `--cmd`
/// run on the same inputs is a HIT. The fix excludes hugit's OWN wedge-state files
/// (`--log`, `--ac`, sidecars) from the tree axis, so storing the cold result no
/// longer mutates the tree the next run hashes (which had been busting the key).
/// This reproduces the failure shape the adversary hit: `--root` CONTAINS the
/// `--log`/`--ac` (the realistic cwd default), where the ad-hoc glob `**/*` would
/// otherwise sweep them in.
#[test]
fn an_identical_ad_hoc_cmd_run_is_a_cache_hit_even_with_state_under_root() {
    let dir = scratch("adhoc-memo");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    // A matched source file alongside the log/ac — the ROOT is `dir` itself, so the
    // ad-hoc `**/*` glob would sweep in log.json / ac.json without the exclusion.
    std::fs::write(dir.join("src.rs"), b"fn main() {}\n").unwrap();

    let args = || -> Vec<String> {
        vec![
            "check".into(),
            "run".into(),
            "--def".into(),
            "adhoc-memo-check".into(),
            "--cmd".into(),
            "true".into(),
            "--log".into(),
            log.display().to_string(),
            "--store".into(),
            "--ac".into(),
            ac.display().to_string(),
            // ROOT is the dir that CONTAINS the log + ac (the realistic shape).
            "--root".into(),
            dir.display().to_string(),
            "--toolchain".into(),
            "tc-fixed".into(),
        ]
    };

    let a1 = args();
    let r1: Vec<&str> = a1.iter().map(String::as_str).collect();
    let (_, cold) = run(&r1);
    assert_eq!(
        cold["cache_hit"], false,
        "first ad-hoc run is a MISS: {cold}"
    );
    let key1 = cold["memo_key"].as_str().unwrap().to_string();

    let a2 = args();
    let r2: Vec<&str> = a2.iter().map(String::as_str).collect();
    let (_, warm) = run(&r2);
    assert_eq!(
        warm["cache_hit"], true,
        "a 2nd identical ad-hoc --cmd run is a HIT (state files excluded from the tree axis): {warm}"
    );
    assert_eq!(
        warm["memo_key"].as_str().unwrap(),
        key1,
        "the memo key is STABLE across runs — hugit's own state no longer busts it: {warm}"
    );
}

// ── WJ-CHECK-DRAIN (adversarial Round-6 Cluster C) ───────────────────────────

/// WJ-CHECK-DRAIN [SHIP-BLOCKER — pipe-buffer deadlock]: a command that emits
/// more than the 64 KiB OS pipe-buffer to stdout COMPLETES PROMPTLY instead of
/// deadlocking for 300 s. Before the fix, `ProcessRunner` piped stdout/stderr
/// but never drained them; once the pipe buffer filled the child blocked on its
/// next `write()`, `try_wait()` returned `Ok(None)` forever, and the 300 s
/// timeout was the only way out — holding the process for ~5 min.
///
/// The fix: two background drain threads consume stdout and stderr concurrently
/// with the poll loop (capped at 4 MiB per stream to prevent OOM). The child
/// is never blocked on a full pipe, so a >64 KiB-output command completes in
/// its real wall-clock time, not 300 s.
///
/// This test uses a PORTABLE, BOUNDED flood: `head -c 200000 /dev/zero` emits
/// exactly 200 000 bytes (~195 KiB) to stdout — safely above the 64 KiB
/// pipe-buffer ceiling, safely below OOM, and built from POSIX-standard tools
/// (`head -c` + `/dev/zero`) present on macOS + Linux. It is bounded by
/// construction (no unbounded `yes`), so it cannot OOM CI even if the drain cap
/// regressed.
///
/// Proof:
///   - Without the drain fix: the child fills the 64 KiB buffer, blocks on
///     the next write, never calls exit(); `try_wait()` returns `Ok(None)` on
///     every poll; the loop runs until the 300 s ceiling. The test's 20 s
///     wall-time assertion FAILS (elapsed > 20 s).
///   - With the drain fix: the drain threads consume the output as it arrives;
///     the child exits normally after emitting all bytes; `try_wait()` returns
///     `Ok(Some(status))` with exit 0; the verb reports `ok:true` and
///     `exit:0`; elapsed << 20 s.
#[test]
#[cfg(unix)]
fn large_output_command_completes_promptly_no_pipe_buffer_deadlock() {
    let dir = scratch("drain");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    // A portable >64 KiB flood (200 000 bytes ~ 195 KiB) built from POSIX tools:
    // `/dev/zero` is an infinite source, `head -c` caps it at exactly 200 000
    // bytes. Bounded by construction — no unbounded `yes`.
    let flood_cmd = "head -c 200000 /dev/zero";

    let start = std::time::Instant::now();
    let (code, v) = run(&[
        "check",
        "run",
        "--def",
        "drain-check",
        "--cmd",
        flood_cmd,
        "--log",
        &log.display().to_string(),
        "--store",
        "--ac",
        &ac.display().to_string(),
        "--root",
        &root.display().to_string(),
        "--toolchain",
        "tc-fixed",
        "--timeout-secs",
        "60",
    ]);
    let elapsed = start.elapsed();

    // The command exits 0 and the verb completes promptly (well under the 20 s
    // wall-time budget — the actual runtime on any real host is < 1 s).
    assert_eq!(
        code, 0,
        "a >64 KiB-output check exits 0 (no deadlock): {v}; elapsed: {elapsed:?}"
    );
    assert_eq!(
        v["ok"], true,
        "a >64 KiB-output command that exits 0 is ok: {v}"
    );
    assert_eq!(v["exit"], 0, "exit code is 0: {v}");
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "the >64 KiB-output check completed in {elapsed:?} — NOT deadlocked for 300 s \
         (pipe drain is working; without the fix this would block until the timeout)"
    );
}

/// WJ-CHECK-DRAIN [lock-not-held]: the `--log` FileLock is NOT held during
/// command execution. A concurrent `hugit checks show --log L` while a flood
/// check is running on the same log MUST NOT be blocked out (it should complete
/// in milliseconds, not be stuck waiting for the flood check to finish).
///
/// This test directly verifies the architectural property: the log lock is
/// acquired only inside `record_on_log` (called AFTER `run_memoized` returns),
/// never across the execute. A flooding command must not cause `log_busy` for
/// any concurrent verb.
#[test]
#[cfg(unix)]
fn concurrent_show_is_not_locked_out_during_a_flood_check() {
    use std::thread;

    let dir = scratch("drain-lock");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    // The flood check: emit 200 KB (> the 64 KiB pipe buffer) THEN sleep 1 s so
    // the check is still executing while the concurrent `checks show` runs — a
    // genuine overlap. The 200 KB output proves the drain works (no deadlock);
    // the 1 s sleep proves the log lock is not held across the whole execute
    // (the concurrent verb is not blocked for that second). Bounded by
    // construction (`head -c`), no unbounded `yes`. 30 s timeout is well above
    // the ~1 s real runtime so it is never killed.
    let flood_cmd = "head -c 200000 /dev/zero; sleep 1";
    let log_str = log.display().to_string();
    let ac_str = ac.display().to_string();
    let root_str = root.display().to_string();

    let flood_handle = {
        let log_str = log_str.clone();
        let ac_str = ac_str.clone();
        let root_str = root_str.clone();
        thread::spawn(move || {
            run(&[
                "check",
                "run",
                "--def",
                "drain-lock-check",
                "--cmd",
                flood_cmd,
                "--log",
                &log_str,
                "--store",
                "--ac",
                &ac_str,
                "--root",
                &root_str,
                "--toolchain",
                "tc-fixed",
                "--timeout-secs",
                "30",
            ])
        })
    };

    // Let the flood check start.
    thread::sleep(std::time::Duration::from_millis(50));

    // While the flood check is running, a concurrent `checks show` MUST NOT be
    // locked out. If the log lock were held across execute, this would block
    // until the flood check's record_on_log finished — potentially seconds.
    // With the fix it returns in milliseconds.
    let show_start = std::time::Instant::now();
    let (show_code, show_v) = run(&["check", "show", "--log", &log_str]);
    let show_elapsed = show_start.elapsed();

    // `checks show` must complete promptly (well under 5 s, typically < 100 ms).
    assert!(
        show_elapsed < std::time::Duration::from_secs(5),
        "concurrent `checks show` completed in {show_elapsed:?} — not locked out by \
         the flood check (log lock is not held across execute): {show_v}"
    );
    // `checks show` on a fresh log (no records yet from the flood, which may
    // still be running) exits 0 with check_count 0 or 1 depending on race;
    // we only care that it didn't block.
    assert_eq!(
        show_code, 0,
        "`checks show` exits 0 while a flood check is running: {show_v}"
    );

    // Let the flood check finish and confirm it succeeded.
    let (flood_code, flood_v) = flood_handle.join().unwrap();
    assert_eq!(
        flood_code, 0,
        "the flood check itself completed successfully: {flood_v}"
    );
}

// ── W5 — local-only determinism: CoreLink is NEVER in the runtime path ────────

/// Owner decision: `hugit check` runs LOCAL-ONLY — a `HUGIT_CORELINK_*` env var
/// must have NO effect, the wire must never happen. The local `FileAc` is always
/// used: a cold run MISSES + executes, a warm re-run HITs with `duration_ms:0`
/// (zero local execution) — byte-equivalent behavior to running without the env.
#[test]
fn corelink_env_has_no_effect_check_stays_local() {
    let dir = scratch("local-only");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);
    let root = seed_tree(&dir);

    let args = check_args("local-only-check", "true", &log, &ac, &root);
    let r: Vec<&str> = args.iter().map(String::as_str).collect();

    // Set the full CoreLink AC runtime config as if a live AC were provisioned.
    // If the runtime path still swapped in `HttpAcClient::from_runtime`, this
    // would attempt a wire call (and fail / miss against the dead URL, and the
    // warm re-run would NOT be a local HIT). With the local-only decision the
    // env is inert — the file AC answers both runs.
    let run = |args: &[&str]| {
        let out = Command::new(hugit_bin())
            .args(args)
            .env(
                "HUGIT_CORELINK_AC_URL",
                "http://127.0.0.1:1/zzz-not-a-real-ac",
            )
            .env("HUGIT_CORELINK_TENANT", "tenant-w5")
            .env("HUGIT_CORELINK_PAT", "pat-w5")
            .output()
            .expect("hugit binary runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
        (out.status.code().unwrap_or(-1), v)
    };

    let (code, cold) = run(&r);
    assert_eq!(
        code, 0,
        "cold run with HUGIT_CORELINK_* set exits 0: {cold}"
    );
    assert_eq!(
        cold["cache_hit"], false,
        "the cold run is a local MISS — the env had no effect: {cold}"
    );
    assert_eq!(
        cold["local_executions"], 1,
        "the cold run executed locally exactly once: {cold}"
    );
    let cold_key = cold["memo_key"].as_str().unwrap().to_string();

    let (code, warm) = run(&r);
    assert_eq!(
        code, 0,
        "warm run with HUGIT_CORELINK_* set exits 0: {warm}"
    );
    assert_eq!(
        warm["cache_hit"], true,
        "the warm re-run is a LOCAL HIT (the local FileAc served it): {warm}"
    );
    assert_eq!(
        warm["local_executions"], 0,
        "a hit performs zero local execution: {warm}"
    );
    assert_eq!(
        warm["duration_ms"], 0,
        "a hit re-spends zero wall-clock: {warm}"
    );
    assert_eq!(
        warm["memo_key"].as_str().unwrap(),
        cold_key,
        "identical inputs key identically — the env is NOT a memo-axis: {warm}"
    );
}
