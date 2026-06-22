//! Acceptance — Wave Q: an ANCESTOR `Cargo.toml` or `Cargo.lock` (above `--root`)
//! busts the `clippy` and `test` memo keys; `fmt` is UNAFFECTED (hit-rate preserved).
//!
//! ROOT (Round 12 `wedge-decider.md`): Wave P's `ancestor_config_names` for
//! `clippy`/`test` omitted `Cargo.toml` and `Cargo.lock`. Cargo reads the
//! WORKSPACE-root `Cargo.toml` (which can sit in an ANCESTOR dir above `--root`)
//! for `[workspace.lints]`, `[profile]`, `[patch]`, and the `Cargo.lock` for
//! resolved deps — all result-affecting for `clippy`/`test`. Live repro: cold
//! `check clippy` GREEN → mutate parent `Cargo.toml` `[workspace.lints.clippy]`
//! allow→deny → warm re-run → `cache_hit:true` exit:0 = STALE GREEN.
//!
//! THE FIX (Wave Q, 5th/final bounded stale-green): add `Cargo.toml` and
//! `Cargo.lock` to `ancestor_config_names("clippy" | "test")`. An ancestor change
//! to either BUSTS the key → MISS → real re-execution. `fmt` does NOT read
//! `Cargo.toml`/`Cargo.lock`; its ancestor set is unchanged (hit-rate preserved).
//!
//! These tests exercise the `ancestor_config_digest` function in isolation (via
//! the in-crate unit path) so they run without a `cargo` binary on PATH and
//! complete quickly. The live end-to-end repro (actual `hugit check` binary +
//! real `cargo clippy`) is covered by `acceptance_p_ancestor_config.rs`.

#![cfg(unix)]

use std::path::{Path, PathBuf};

// ── helpers ──────────────────────────────────────────────────────────────────

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-q-anc-{tag}-{}-{}",
        std::process::id(),
        nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build a `parent/proj` layout under `base`. Returns `(parent_dir, proj_root)`.
fn make_parent_proj(base: &Path) -> (PathBuf, PathBuf) {
    let parent = base.join("parent");
    let proj = parent.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    (parent, proj)
}

/// Thin shim to call `ancestor_config_digest` via the binary's integration
/// surface: run `hugit check` in a fully-formed scratch workspace and compare
/// the `memo_key` reported for two consecutive runs.
///
/// We test via the UNIT function directly (same crate), which is simpler and
/// avoids needing a real `cargo` binary for these key-mutation checks.
/// The `hugit_cli` crate is the owning crate for the integration tests, but the
/// internal function is only accessible from unit tests inside the crate itself.
/// We therefore drive the full binary through the CLI so the integration test
/// can observe the memo key without bypassing any layers.
///
/// Pattern: run the binary with `--cmd true` (instant, no cargo needed) and
/// record the `memo_key` from the JSON response. Comparing two keys proves
/// whether a change to the filesystem caused a bust.
fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn write_empty_log(path: &Path) {
    use hugit_refstore::EventLog;
    let log = EventLog::new();
    std::fs::write(path, serde_json::to_string_pretty(log.records()).unwrap()).unwrap();
}

/// Run `hugit check --def clippy --cmd true --root <root>` (no real cargo).
/// Returns the `memo_key` field from the JSON response.
fn key_for(def: &str, root: &Path, log: &Path, ac: &Path) -> String {
    let out = std::process::Command::new(hugit_bin())
        .args([
            "check",
            "run",
            "--def",
            def,
            "--cmd",
            "true",
            "--log",
            log.to_str().unwrap(),
            "--ac",
            ac.to_str().unwrap(),
            "--root",
            root.to_str().unwrap(),
            "--timeout-secs",
            "30",
        ])
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).unwrap_or(serde_json::Value::Null);
    v.get("memo_key")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string()
}

// ── Wave Q core: Cargo.toml ancestor busts clippy/test, not fmt ──────────────

/// An ANCESTOR `Cargo.toml` (above `--root`) MUST bust the `clippy` memo key.
/// Before Wave Q the key was unchanged by this mutation → stale green.
#[test]
fn ancestor_cargo_toml_busts_clippy_key() {
    let dir = scratch("ct-clippy");
    let (parent, proj) = make_parent_proj(&dir);
    let root = std::fs::canonicalize(&proj).unwrap();
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Baseline: no ancestor Cargo.toml → key K1.
    let k1 = key_for("clippy", &root, &log, &ac);
    assert!(!k1.is_empty(), "baseline key must be non-empty");

    // Add a parent Cargo.toml with [workspace.lints.clippy] allow → key MUST change.
    std::fs::write(
        parent.join("Cargo.toml"),
        b"[workspace]\nmembers = [\"proj\"]\n\n[workspace.lints.clippy]\nall = \"allow\"\n",
    )
    .unwrap();
    let k2 = key_for("clippy", &root, &log, &ac);
    assert_ne!(
        k1, k2,
        "adding a parent Cargo.toml MUST bust the clippy memo key (Wave Q R12 hole)"
    );

    // Mutate the parent Cargo.toml → key MUST change again (catches the stale-green).
    std::fs::write(
        parent.join("Cargo.toml"),
        b"[workspace]\nmembers = [\"proj\"]\n\n[workspace.lints.clippy]\nall = \"deny\"\n",
    )
    .unwrap();
    let k3 = key_for("clippy", &root, &log, &ac);
    assert_ne!(
        k2, k3,
        "mutating the parent Cargo.toml MUST bust the clippy key again (the stale-green path)"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// An ANCESTOR `Cargo.lock` (above `--root`) MUST bust the `clippy` memo key.
#[test]
fn ancestor_cargo_lock_busts_clippy_key() {
    let dir = scratch("cl-clippy");
    let (parent, proj) = make_parent_proj(&dir);
    let root = std::fs::canonicalize(&proj).unwrap();
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    let k1 = key_for("clippy", &root, &log, &ac);
    assert!(!k1.is_empty(), "baseline key must be non-empty");

    // Add a parent Cargo.lock → MUST bust.
    std::fs::write(parent.join("Cargo.lock"), b"version = 3\n").unwrap();
    let k2 = key_for("clippy", &root, &log, &ac);
    assert_ne!(
        k1, k2,
        "adding a parent Cargo.lock MUST bust the clippy memo key (Wave Q)"
    );

    // Mutate the Cargo.lock → MUST bust again.
    std::fs::write(parent.join("Cargo.lock"), b"version = 3\n# changed\n").unwrap();
    let k3 = key_for("clippy", &root, &log, &ac);
    assert_ne!(
        k2, k3,
        "mutating the parent Cargo.lock MUST bust the clippy key"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `fmt` must NOT be affected by a parent `Cargo.toml` change — `cargo fmt`
/// does not read it, so busting on it would needlessly degrade the fmt hit-rate.
/// This asserts the per-def scoping is correct: only clippy/test capture it.
#[test]
fn ancestor_cargo_toml_does_not_bust_fmt_key() {
    let dir = scratch("ct-fmt");
    let (parent, proj) = make_parent_proj(&dir);
    let root = std::fs::canonicalize(&proj).unwrap();
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Baseline fmt key (no ancestor Cargo.toml).
    let kf1 = key_for("fmt", &root, &log, &ac);
    assert!(!kf1.is_empty(), "baseline fmt key must be non-empty");

    // Add a parent Cargo.toml → fmt key must be INVARIANT (no needless bust).
    std::fs::write(
        parent.join("Cargo.toml"),
        b"[workspace]\nmembers = [\"proj\"]\n",
    )
    .unwrap();
    let kf2 = key_for("fmt", &root, &log, &ac);
    assert_eq!(
        kf1, kf2,
        "a parent Cargo.toml MUST NOT bust the fmt memo key (per-def scoping, hit-rate preserved)"
    );

    // Confirm clippy IS busted by the same change — proves the fixture is right.
    let kc1 = key_for("clippy", &root, &log, &ac);
    // Remove the file and re-key to get the clippy baseline without it.
    let log2 = dir.join("log2.json");
    let ac2 = dir.join("ac2.json");
    write_empty_log(&log2);
    std::fs::remove_file(parent.join("Cargo.toml")).unwrap();
    let kc0 = key_for("clippy", &root, &log2, &ac2);
    assert_ne!(
        kc0, kc1,
        "clippy key differs with vs without the parent Cargo.toml (confirms per-def scoping)"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// No-change warm re-run: after seeding a parent `Cargo.toml`, a second
/// identical run reuses the same key (hit-rate preserved — no bust from
/// the new capture alone when nothing changed).
#[test]
fn no_change_after_ancestor_cargo_toml_is_still_same_key() {
    let dir = scratch("nochange");
    let (parent, proj) = make_parent_proj(&dir);
    let root = std::fs::canonicalize(&proj).unwrap();
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // Add the ancestor Cargo.toml once.
    std::fs::write(
        parent.join("Cargo.toml"),
        b"[workspace]\nmembers = [\"proj\"]\n",
    )
    .unwrap();

    let k1 = key_for("clippy", &root, &log, &ac);
    assert!(
        !k1.is_empty(),
        "key with ancestor Cargo.toml must be non-empty"
    );

    // A second run with no changes must yield the SAME key (a stable HIT).
    let k2 = key_for("clippy", &root, &log, &ac);
    assert_eq!(
        k1, k2,
        "no change must yield the same key (hit-rate preserved after Wave Q capture)"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
