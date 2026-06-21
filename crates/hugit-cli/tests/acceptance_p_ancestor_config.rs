//! Acceptance — Wave P FIX A: the built-in gates' ANCESTOR toolchain-config
//! files are memo-key inputs, so a change to one BUSTS the key instead of
//! serving a STALE GREEN (the 4th wedge stale-green, Round 11 R11-1).
//!
//! ROOT (Round 11 `wedge-stale-green.md`): WO-GLOBMISS captured the toolchain
//! config (`.cargo/config.toml` / `rustfmt.toml` / `rust-toolchain*` / `clippy.toml`)
//! INSIDE `--root` via the tree-axis glob. But cargo & rustfmt search UPWARD: a
//! config in an ANCESTOR directory of `--root` (up to the filesystem root, plus
//! `$CARGO_HOME/config.toml`) ALSO changes the gate's result, and the
//! `--root`-relative glob never sees it. Live repro: a parent `.cargo/config.toml`
//! capping clippy lints → cold green key K; remove the cap → real cold `exit 101`
//! under the SAME key K → a warm re-run serves a STALE GREEN.
//!
//! THE FIX (Wave P FIX A): for the built-in gates, the effective ANCESTOR
//! toolchain-config files (walking from the canonical `--root` UP to the
//! filesystem root, plus `$CARGO_HOME/config.toml`) are hashed into a digest
//! folded into `def.inputs` (a `def_digest` axis → the memo key). An ancestor
//! config change BUSTS the key → a MISS that RE-EXECUTES and observes the real
//! failing exit. The set is PER-DEF (fmt reads rustfmt+toolchain config; clippy/
//! test read cargo+clippy+toolchain config) and reads SPECIFIC BOUNDED KNOWN
//! filenames — NOT arbitrary files (the unbounded outside-root read stays the
//! disclosed P2 hermetic seam).
//!
//! Also covers FIX B (the umask over-capture): a `chmod 0664` on a non-exec file
//! (a pure umask difference git does not track) no longer busts the key, while a
//! `chmod -x` (the exec bit — result-affecting) still does (N-1 P0 stays closed).
//!
//! These tests shell out to a REAL `cargo clippy`/`cargo fmt`, so they are gated
//! on `cargo` being on PATH. Scratch trees live OUTSIDE the repo.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

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
        "hugit-p-anc-{tag}-{}-{}",
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

/// Seed `<base>/parent/proj` as a minimal crate whose `src/main.rs` has an UNUSED
/// variable — `cargo clippy -- -D warnings` DENIES it unless a `rustflags` cap in
/// an ANCESTOR `.cargo/config.toml` allows it. Returns `(parent, proj_root)`.
fn seed_crate_under_parent(base: &Path) -> (PathBuf, PathBuf) {
    let parent = base.join("parent");
    let proj = parent.join("proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(
        proj.join("Cargo.toml"),
        b"[package]\nname = \"p\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    // An unused variable → clippy/rustc warns; `-D warnings` makes it exit 101.
    std::fs::write(
        proj.join("src/main.rs"),
        b"fn main() {\n    let unused = 1;\n}\n",
    )
    .unwrap();
    (parent, proj)
}

/// Run `hugit check --def <def> --root <root> --store`; returns `(exit, json)`.
fn run_check(def: &str, root: &Path, log: &Path, ac: &Path) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args([
            "check",
            "run",
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
            "180",
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
fn memo_key(v: &Value) -> String {
    v.get("memo_key")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// THE R11-1 REPRO, CLOSED: a parent `.cargo/config.toml` is an ANCESTOR of
/// `--root` (above it), so the `--root`-relative glob never sees it. Adding /
/// mutating it MUST bust the clippy memo key (a MISS), not serve a stale green.
#[test]
fn ancestor_cargo_config_change_busts_the_key_no_stale_green() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = scratch("cargo");
    let (parent, root) = seed_crate_under_parent(&dir);
    std::fs::create_dir_all(parent.join(".cargo")).unwrap();
    let cfg = parent.join(".cargo/config.toml");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // 1) NO ancestor config → cold MISS, key K.
    let (_p1, r1) = run_check("clippy", &root, &log, &ac);
    assert!(!hit(&r1), "run 1 is a cold MISS: {r1:?}");
    let key_k = memo_key(&r1);
    assert!(!key_k.is_empty(), "run 1 reports a memo key: {r1:?}");

    // 2) Add the ANCESTOR .cargo/config.toml → MUST bust the key (a MISS), even
    //    though it lives ABOVE --root and the tree glob never matches it.
    std::fs::write(&cfg, b"[build]\nrustflags = [\"--cap-lints=allow\"]\n").unwrap();
    let (_p2, r2) = run_check("clippy", &root, &log, &ac);
    assert_ne!(
        memo_key(&r2),
        key_k,
        "an ANCESTOR .cargo/config.toml MUST bust the memo key (R11-1): {r2:?}"
    );
    assert!(
        !hit(&r2),
        "the ancestor-config change must be a re-executing MISS, not a stale green: {r2:?}"
    );
    let key_capped = memo_key(&r2);

    // 3) MUTATE the ancestor config (remove the cap) → key busts AGAIN. This is
    //    the exact R11-1 mutation that previously left the key unchanged.
    std::fs::write(&cfg, b"[build]\nrustflags = []\n").unwrap();
    let (_p3, r3) = run_check("clippy", &root, &log, &ac);
    assert_ne!(
        memo_key(&r3),
        key_capped,
        "MUTATING the ancestor config MUST bust the key (the R11-1 stale green): {r3:?}"
    );
    assert!(
        !hit(&r3),
        "the ancestor-config mutation must be a re-executing MISS: {r3:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// An ANCESTOR `rustfmt.toml` busts the `fmt` key (rustfmt searches upward).
#[test]
fn ancestor_rustfmt_toml_change_busts_the_fmt_key() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = scratch("rustfmt");
    let (parent, _proj) = seed_crate_under_parent(&dir);
    // A formatted lib so the cold fmt run is a real green.
    let root = parent.join("proj");
    std::fs::write(
        root.join("src/main.rs"),
        b"fn main() {\n    let _ = 1;\n}\n",
    )
    .unwrap();
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    let (_p1, r1) = run_check("fmt", &root, &log, &ac);
    assert!(!hit(&r1), "run 1 is a cold MISS: {r1:?}");
    let key_k = memo_key(&r1);
    assert!(!key_k.is_empty(), "run 1 reports a memo key: {r1:?}");

    // Add an ANCESTOR rustfmt.toml (above --root) → MUST bust the fmt key.
    std::fs::write(parent.join("rustfmt.toml"), b"max_width = 1\n").unwrap();
    let (_p2, r2) = run_check("fmt", &root, &log, &ac);
    assert_ne!(
        memo_key(&r2),
        key_k,
        "an ANCESTOR rustfmt.toml MUST bust the fmt memo key: {r2:?}"
    );
    assert!(
        !hit(&r2),
        "the ancestor rustfmt.toml change is a MISS: {r2:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Hit-rate preserved: with NO ancestor config change, a re-run is a warm HIT;
/// an IN-ROOT config change is still a MISS (WO-GLOBMISS holds); and a DOC edit is
/// still a HIT. The ancestor capture must bust ONLY on a real ancestor change.
#[test]
fn hit_rate_preserved_no_change_inroot_still_miss_doc_still_hit() {
    if !cargo_available() {
        eprintln!("skipping: cargo not on PATH");
        return;
    }
    let dir = scratch("hitrate");
    let (_parent, root) = seed_crate_under_parent(&dir);
    std::fs::create_dir_all(root.join(".cargo")).unwrap();
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Cold MISS, key K.
    let (_p1, r1) = run_check("clippy", &root, &log, &ac);
    assert!(!hit(&r1), "run 1 is a cold MISS: {r1:?}");
    let key_k = memo_key(&r1);

    // No change → warm HIT, same key (hit-rate preserved).
    let (_p2, r2) = run_check("clippy", &root, &log, &ac);
    assert_eq!(memo_key(&r2), key_k, "no change keeps key K: {r2:?}");
    assert!(hit(&r2), "no change must be a warm HIT: {r2:?}");

    // An IN-ROOT .cargo/config.toml change is still a MISS (WO-GLOBMISS holds).
    std::fs::write(
        root.join(".cargo/config.toml"),
        b"[build]\nrustflags = [\"--cap-lints=allow\"]\n",
    )
    .unwrap();
    let (_p3, r3) = run_check("clippy", &root, &log, &ac);
    assert_ne!(
        memo_key(&r3),
        key_k,
        "an IN-ROOT .cargo/config.toml change still busts the key (WO-GLOBMISS): {r3:?}"
    );
    assert!(!hit(&r3), "the in-root config change is a MISS: {r3:?}");
    let key_inroot = memo_key(&r3);

    // A doc edit the gate does not read → still a HIT (hit-rate preserved).
    std::fs::write(root.join("README.md"), b"# docs\n").unwrap();
    let (_p4, r4) = run_check("clippy", &root, &log, &ac);
    assert_eq!(
        memo_key(&r4),
        key_inroot,
        "an unrelated *.md edit must NOT bust the key: {r4:?}"
    );
    assert!(hit(&r4), "a doc edit is still a HIT: {r4:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// FIX B end-to-end: a `chmod -x` busts the key (exec bit is result-affecting, the
/// N-1 P0 stays closed); a `chmod 0664` on a NON-EXEC file (a pure umask
/// difference) does NOT bust the key (the umask no longer poisons the
/// cross-runner cache). Uses an ad-hoc `--cmd cat <file>` so the file is a tree
/// input regardless of cargo.
#[test]
fn chmod_x_busts_but_chmod_0664_does_not() {
    let dir = scratch("mode");
    let root = dir.join("work");
    std::fs::create_dir_all(&root).unwrap();
    let f = root.join("gate.sh");
    std::fs::write(&f, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Ad-hoc check: `true` is the command; the tree axis hashes gate.sh + its mode.
    let run_adhoc = |root: &Path| -> Value {
        let out = Command::new(hugit_bin())
            .args([
                "check",
                "run",
                "--def",
                "modecheck",
                "--cmd",
                "true",
                "--log",
                log.to_str().unwrap(),
                "--ac",
                ac.to_str().unwrap(),
                "--root",
                root.to_str().unwrap(),
                "--store",
                "--timeout-secs",
                "30",
            ])
            .output()
            .expect("hugit runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(stdout.trim()).unwrap_or(Value::Null)
    };

    // Cold MISS at 0644, key K.
    let r1 = run_adhoc(&root);
    assert!(!hit(&r1), "run 1 is a cold MISS: {r1:?}");
    let key_0644 = memo_key(&r1);
    assert!(!key_0644.is_empty(), "run 1 reports a key: {r1:?}");

    // chmod 0664 (only the group-write bit — a UMASK difference) → STILL a HIT,
    // SAME key. FIX B: the umask read/write bits no longer leak into the key.
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o664)).unwrap();
    let r2 = run_adhoc(&root);
    assert_eq!(
        memo_key(&r2),
        key_0644,
        "0644 vs 0664 (umask only) must NOT change the key (FIX B): {r2:?}"
    );
    assert!(
        hit(&r2),
        "a pure umask difference is a cross-runner HIT, not a MISS: {r2:?}"
    );

    // chmod -x… first chmod +x to set the exec bit, which IS result-affecting →
    // MISS, key busts (the N-1 P0 stays closed under exec-only folding).
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
    let r3 = run_adhoc(&root);
    assert_ne!(
        memo_key(&r3),
        key_0644,
        "setting the exec bit MUST bust the key (N-1 P0 stays closed): {r3:?}"
    );
    assert!(!hit(&r3), "an exec-bit change is a MISS: {r3:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
