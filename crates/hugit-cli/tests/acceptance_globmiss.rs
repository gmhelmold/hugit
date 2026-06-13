//! Acceptance — WO-GLOBMISS: the built-in gates' TOOLCHAIN-CONFIG files are
//! tree-axis inputs, so a change to one BUSTS the memo key instead of serving a
//! STALE GREEN.
//!
//! ROOT (sweep-2026-06-12 wedge-stale-green-reaudit, finding WO-GLOBMISS): the
//! built-in check defs (`fmt`/`clippy`/`test`) scoped their tree axis to
//! `["**/*.rs", "**/Cargo.toml", "Cargo.lock"]`, but the built-in gate COMMANDS
//! read result-affecting config OUTSIDE that set (`rustfmt.toml` for fmt;
//! `clippy.toml` / `.cargo/config.toml` / `rust-toolchain.toml` for clippy/test).
//! Lead repro: `--def fmt` over FORMATTED code → cold GREEN under memo_key K;
//! then add `rustfmt.toml(max_width=1)` (the code is now unformatted under the
//! new rule, a real `cargo fmt --all --check` is `exit 1`) → the warm re-run was
//! a HIT serving the SAME key K and a stale `exit 0`. The config file was a
//! result-affecting input the glob dropped — the SAME failure class as the N-1
//! mode-bit P0, on the glob-scope axis.
//!
//! THE FIX (WO-GLOBMISS): the built-in glob set is WIDENED (per-def) to include
//! the bounded toolchain-config files, so a change to one changes the `tree_root`
//! → a MISS that RE-EXECUTES and observes the real failing exit. The glob stays
//! NARROW (NOT `**/*`): a doc/`*.md` edit is still a HIT — the wedge's hit-rate
//! is preserved.
//!
//! HONEST RESIDUAL (disclosed, NOT closed here): a built-in `test` can read an
//! ARBITRARY fixture (`tests/data/foo.json`, `include_str!`, a `build.rs`-emitted
//! path) that no bounded glob can predict — that unbounded-read case is the same
//! class as the disclosed P2 hermetic-execution seam (the runner-side isolated
//! rootfs), NOT closable locally by enumerating more globs. These tests close the
//! BOUNDED config vectors only.
//!
//! These tests shell out to a REAL `cargo fmt`/`cargo clippy`, so they are gated
//! on `cargo` being on PATH (CI's self-hosted runner has the toolchain). Both
//! scratch trees live OUTSIDE the repo (`std::env::temp_dir()`).

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

/// Skip the repro when `cargo` is not on PATH (the built-in gate commands shell
/// out to it). On the CI runner it is always present.
fn cargo_available() -> bool {
    Command::new("cargo")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-globmiss-{tag}-{}-{}",
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

/// Seed `<dir>/work` as a minimal, ALREADY-FORMATTED crate: a clean
/// `cargo fmt --all --check` over it exits 0. Returns the `--root`.
fn seed_formatted_crate(dir: &Path) -> PathBuf {
    let root = dir.join("work");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        b"[package]\nname = \"g\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // Properly rustfmt-formatted under the DEFAULT profile (so the cold fmt run is
    // a true green); narrowing `max_width` later makes the SAME bytes unformatted.
    std::fs::write(
        root.join("src/lib.rs"),
        b"pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .unwrap();
    root
}

/// Run `hugit check --def <def> --root <root> --store` and return
/// `(process_exit, parsed_json)`. A generous timeout covers a real `cargo` run.
fn run_check(def: &str, root: &Path, log: &Path, ac: &Path) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args([
            "check",
            "--def",
            def,
            "--log",
            log.to_str().unwrap(),
            "--ac",
            ac.to_str().unwrap(),
            "--root",
            root.to_str().unwrap(),
            "--store",
            "--timeout-secs",
            "120",
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

/// THE REPRO (WO-GLOBMISS), CLOSED: `--def fmt` over FORMATTED code → cold MISS /
/// GREEN / key K. Adding `rustfmt.toml(max_width=1)` (SAME source bytes, now
/// unformatted under the new rule → real `cargo fmt --check` is `exit 1`) → a
/// MISS that RE-EXECUTES and observes the REAL `exit 1` — NOT a stale-green hit —
/// because the memo key CHANGED (`rustfmt.toml` is now in the tree axis).
#[test]
fn rustfmt_toml_change_busts_the_key_no_stale_green() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = scratch("fmt");
    let root = seed_formatted_crate(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // 1) Formatted code, no rustfmt.toml → cold MISS, GREEN, memo_key K.
    let (_p1, r1) = run_check("fmt", &root, &log, &ac);
    assert!(!hit(&r1), "run 1 is a cold MISS: {r1:?}");
    assert_eq!(exit_of(&r1), 0, "formatted code passes `cargo fmt`: {r1:?}");
    let key_k = memo_key(&r1);
    assert!(!key_k.is_empty(), "run 1 reports a memo key: {r1:?}");

    // 2) Add rustfmt.toml(max_width=1) — the SAME source is now unformatted under
    //    the new rule; ground truth: `cargo fmt --all --check` is exit 1.
    std::fs::write(root.join("rustfmt.toml"), b"max_width = 1\n").unwrap();

    let (_p2, r2) = run_check("fmt", &root, &log, &ac);
    // The memo key MUST have changed (rustfmt.toml folded into the tree axis).
    assert_ne!(
        memo_key(&r2),
        key_k,
        "adding rustfmt.toml MUST bust the memo key: {r2:?}"
    );
    // …so this is a MISS that RE-EXECUTES, NOT a stale-green hit.
    assert!(
        !hit(&r2),
        "the config change must produce a re-executing MISS, never a stale-green hit: {r2:?}"
    );
    // …and the re-executed run observes the REAL failing exit, not the cached 0.
    assert_eq!(
        exit_of(&r2),
        1,
        "the now-unformatted code fails `cargo fmt --check` exit 1 — the stale \
         green is gone: {r2:?}"
    );
}

/// Hit-rate preserved (no FALSE busting): with NO config change, a fmt re-run is
/// still a warm HIT; and adding a DOC file (`README.md`) — which the gate does not
/// read and which the narrow glob does not match — is STILL a HIT. The widened
/// glob must bust ONLY on a real config change, never on every run or on docs.
#[test]
fn no_config_change_and_doc_edit_stay_hits() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = scratch("fmt-hit");
    let root = seed_formatted_crate(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Cold MISS, GREEN, key K.
    let (_p1, r1) = run_check("fmt", &root, &log, &ac);
    assert!(!hit(&r1), "run 1 is a cold MISS: {r1:?}");
    assert_eq!(exit_of(&r1), 0, "formatted code passes: {r1:?}");
    let key_k = memo_key(&r1);

    // Identical re-run, no change → warm HIT, same key.
    let (_p2, r2) = run_check("fmt", &root, &log, &ac);
    assert_eq!(
        memo_key(&r2),
        key_k,
        "an unchanged tree keeps key K: {r2:?}"
    );
    assert!(
        hit(&r2),
        "no change must be a warm HIT — hit-rate preserved: {r2:?}"
    );

    // Add a DOC file the gate does not read and the narrow glob does not match.
    std::fs::write(root.join("README.md"), b"# docs\n").unwrap();
    let (_p3, r3) = run_check("fmt", &root, &log, &ac);
    assert_eq!(
        memo_key(&r3),
        key_k,
        "an unrelated *.md edit must NOT bust the key (glob stays narrow): {r3:?}"
    );
    assert!(
        hit(&r3),
        "a doc edit is still a HIT — the wedge's hit-rate is not gutted: {r3:?}"
    );
}

/// The clippy/test config vectors: a `clippy.toml` or `.cargo/config.toml` change
/// busts the memo key under `--def clippy` (these gates read the lint + cargo
/// build config). We assert the KEY change + MISS (the cold clippy verdict in a
/// throwaway crate is not load-bearing — the stale-green class is the key
/// collision, identical to the fmt green→red case).
#[test]
fn clippy_config_files_bust_the_key() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = scratch("clippy");
    let root = seed_formatted_crate(&dir);
    std::fs::create_dir_all(root.join(".cargo")).unwrap();
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Cold MISS under clippy, key K.
    let (_p1, r1) = run_check("clippy", &root, &log, &ac);
    assert!(!hit(&r1), "run 1 is a cold MISS: {r1:?}");
    let key_k = memo_key(&r1);
    assert!(!key_k.is_empty(), "run 1 reports a memo key: {r1:?}");

    // Add .cargo/config.toml (rustflags is a clippy input) → MISS, key busts.
    std::fs::write(
        root.join(".cargo/config.toml"),
        b"[build]\nrustflags = [\"-Dwarnings\"]\n",
    )
    .unwrap();
    let (_p2, r2) = run_check("clippy", &root, &log, &ac);
    let key_cargo = memo_key(&r2);
    assert_ne!(
        key_cargo, key_k,
        "adding .cargo/config.toml MUST bust the clippy memo key: {r2:?}"
    );
    assert!(
        !hit(&r2),
        "the .cargo/config.toml change must be a re-executing MISS, not a hit: {r2:?}"
    );

    // Add clippy.toml (a lint-config input) → MISS, key busts again.
    std::fs::write(
        root.join("clippy.toml"),
        b"too-many-arguments-threshold = 2\n",
    )
    .unwrap();
    let (_p3, r3) = run_check("clippy", &root, &log, &ac);
    assert_ne!(
        memo_key(&r3),
        key_cargo,
        "adding clippy.toml MUST bust the clippy memo key: {r3:?}"
    );
    assert!(
        !hit(&r3),
        "the clippy.toml change must be a re-executing MISS, not a hit: {r3:?}"
    );
}
